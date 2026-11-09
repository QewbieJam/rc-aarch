//! 进程类系统调用（ch5）—— 对照 rCore ch5 的 `os/src/syscall/process.rs`
//!
//! ch5 新增四个：
//!   getpid(172) / fork(220) / exec(221) / waitpid(260)
//!
//! fork 的语义（和 Linux 一致）：
//!   - 父进程里返回**子进程 pid**（>0）；
//!   - 子进程里返回 **0**；
//!   - 失败返回负数（我们这里分配不出来的话会 panic，暂不模拟失败）。
//!
//! exec 的语义：把当前进程的地址空间整个换掉，从新 ELF 的入口重新开始跑。
//! 注意它是"换汤不换药"—— pid、父子关系、内核栈都不变。

use crate::mm::{translated_ref, translated_refmut, translated_str};
use crate::task::{
    MAX_SIG, SignalAction, SignalFlags, TaskControlBlock, add_task, current_task,
    current_user_token, exit_current_and_run_next, pid2process, suspend_current_and_run_next,
};
use crate::timer::get_time_ms;

/// 退出（对照 upstream 的 `sys_exit`）
pub fn sys_exit(exit_code: i32) -> ! {
    exit_current_and_run_next(exit_code)
}

/// 主动让出
pub fn sys_yield() -> isize {
    suspend_current_and_run_next();
    0
}

/// 毫秒时间
pub fn sys_get_time() -> isize {
    get_time_ms() as isize
}

/// 改 program break（ch4 的 sbrk，ch5 继续用）
pub fn sys_sbrk(size: i32) -> isize {
    match crate::task::change_program_brk(size) {
        Some(old) => old as isize,
        None => -1,
    }
}

/// 取当前进程 pid（对照 upstream 的 `sys_getpid`）
pub fn sys_getpid() -> isize {
    current_task().unwrap().getpid() as isize
}

/// 复制当前进程（对照 upstream 的 `sys_fork`）
pub fn sys_fork() -> isize {
    let current = current_task().unwrap();
    let new_task = current.fork();
    let new_pid = new_task.getpid();
    // ★ 子进程的 TrapContext 已经在 fork() 里把 x0 置成 0 了；
    //   父进程的返回值由 trap_handler 写回 x0（= new_pid）。
    add_task(new_task);
    new_pid as isize
}

/// 换成另一个 ELF 执行（对照 upstream 的 `sys_exec`）
///
/// `path` 是用户态 C 字符串指针，必须先翻译回内核（ch4 的坑 4）。
pub fn sys_exec(path: *const u8, argv: *const usize) -> isize {
    let token = current_user_token();
    let path = translated_str(token, path);
    // ch7：把 argv 指针数组里的每个字符串拷回内核
    let mut args: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
    if !argv.is_null() {
        let mut p = argv;
        loop {
            if args.len() > 16 {
                break; // 防御：用户数组没写结尾的 NULL 时别读飞
            }
            let arg_ptr = *translated_ref(token, p);
            if arg_ptr == 0 {
                break;
            }
            args.push(translated_str(token, arg_ptr as *const u8));
            p = unsafe { p.add(1) };
        }
    }
    // ch7：ELF 读进**页帧缓冲**（不用 Vec —— 内核 bump 堆不回收，Vec 扩容会泄漏）
    let size = match crate::fs::file_size(path.as_str()) {
        Some(s) if s > 0 => s,
        _ => {
            crate::println!("[kernel] exec: 文件系统里没有 \"{}\"", path);
            return -1;
        }
    };
    let mut buf = crate::mm::FrameBuf::alloc(size);
    let n = crate::fs::read_file_to(path.as_str(), &mut buf.as_mut_slice()[..size]).unwrap_or(0);
    if n != size {
        crate::println!("[kernel] exec: \"{}\" 只读到 {}/{} 字节", path, n, size);
        return -1;
    }
    let argc = args.len();
    current_task().unwrap().exec(&buf.as_slice()[..size], &args);
    // 返回 argc：trap_handler 会把它写回 x0（exec 后入口处的第一个参数）
    argc as isize
}

/// 等待子进程（对照 upstream 的 `sys_waitpid`）
///
/// - 没有这个子进程          → -1
/// - 有但还在跑              → -2
/// - 有僵尸子进程            → 回收它，返回它的 pid，并把 exit_code 写回用户指针
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    let task = current_task().unwrap();
    let process = task.process();
    let token = process.inner_exclusive_access().get_user_token();
    let mut pcb = process.inner_exclusive_access();
    if !pcb
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
    }
    let pair = pcb.children.iter().enumerate().find(|(_, p)| {
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
    });
    if let Some((idx, _)) = pair {
        // 从 children 里摘掉；此时 Arc 只剩我们手里这一个
        let child = pcb.children.remove(idx);
        let found_pid = child.getpid();
        let exit_code = child.inner_exclusive_access().exit_code;
        // 写回用户指针（父进程的地址空间）
        *translated_refmut(token, exit_code_ptr) = exit_code;
        // child 在这里 drop：PCB / 页表 / 页帧 / 内核栈 / pid 全部回收
        drop(child);
        found_pid as isize
    } else {
        -2
    }
}

// ---------------------------------------------------------------------------
// ch7：信号
// ---------------------------------------------------------------------------

/// 给某个进程发信号（对照 upstream 的 `sys_kill`）
///
/// `signum` 为 0 时是 SIGDEF（Linux 里用来"探测进程存在"），我们也允许。
pub fn sys_kill(pid: usize, signum: i32) -> isize {
    if signum < 0 || signum as usize > MAX_SIG {
        return -1;
    }
    if let Some(process) = pid2process(pid) {
        if let Some(flag) = SignalFlags::from_bits(1u32 << signum) {
            let threads = process.inner_exclusive_access().threads.clone();
            let mut delivered = false;
            for task in threads.iter() {
                let mut inner = task.inner_exclusive_access();
                if !inner.signals.contains(flag) {
                    inner.signals.insert(flag);
                    delivered = true;
                }
            }
            if delivered {
                0
            } else {
                -1 // 同一个信号还没处理完，别重复挂
            }
        } else {
            -1
        }
    } else {
        -1
    }
}

/// 设置信号处理函数（对照 upstream 的 `sys_sigaction`）
pub fn sys_sigaction(
    signum: i32,
    action: *const SignalAction,
    old_action: *mut SignalAction,
) -> isize {
    let token = current_user_token();
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if signum < 0 || signum as usize > MAX_SIG {
        return -1;
    }
    if action.is_null() || old_action.is_null() {
        return -1;
    }
    let flag = SignalFlags(1u32 << signum);
    // SIGKILL / SIGSTOP 不允许用户接管（和 Linux 一致）
    if flag == SignalFlags::SIGKILL || flag == SignalFlags::SIGSTOP {
        return -1;
    }
    let prev = inner.signal_actions.table[signum as usize];
    *translated_refmut(token, old_action) = prev;
    inner.signal_actions.table[signum as usize] = *translated_ref(token, action);
    0
}

/// 设置信号屏蔽字（对照 upstream 的 `sys_sigprocmask`）
pub fn sys_sigprocmask(mask: u32) -> isize {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let old = inner.signal_mask;
    if let Some(flag) = SignalFlags::from_bits(mask) {
        inner.signal_mask = flag;
        old.bits() as isize
    } else {
        -1
    }
}

/// 从信号处理函数返回（对照 upstream 的 `sys_sigreturn`）
///
/// 恢复进 handler 之前备份的 TrapContext；返回值就是恢复后的 x0，
/// 这样 trap_handler 写回 x0 时不会把现场改坏。
pub fn sys_sigreturn() -> isize {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    inner.handling_sig = -1;
    let cx = inner.get_trap_cx();
    match inner.trap_ctx_backup.take() {
        Some(backup) => {
            *cx = backup;
            cx.x[0] as isize
        }
        None => -1,
    }
}

// ---------------------------------------------------------------------------
// ch8：线程
// ---------------------------------------------------------------------------

/// 在当前进程里新建一个线程（ch8）
///
/// 参数是用户 VA：`entry` 是入口、`arg` 是传给入口的参数、`stack_top` 是该线程
/// 自己的用户栈顶（我们的扩展：upstream 让所有线程共用一个栈顶）。
pub fn sys_thread_create(entry: usize, arg: usize, stack_top: usize) -> isize {
    let task = current_task().unwrap();
    let process = task.process();
    if entry == 0 || stack_top == 0 {
        return -1;
    }
    let new_thread = TaskControlBlock::new_thread_in(process, entry, arg, stack_top);
    let tid = new_thread.gettid();
    add_task(new_thread);
    tid as isize
}

/// 取当前线程号
pub fn sys_gettid() -> isize {
    current_task().unwrap().gettid() as isize
}

/// 等待同进程的某个线程退出，返回它的退出码（ch8）
///
/// - 该 tid 不属于当前进程 → -1
/// - 还没退出               → -2
/// - 已退出                 → 回收它（从 threads 里摘掉），返回 exit_code
pub fn sys_waittid(tid: usize) -> isize {
    let task = current_task().unwrap();
    let process = task.process();
    let mut pcb = process.inner_exclusive_access();
    let idx = match pcb.threads.iter().position(|t| t.gettid() == tid) {
        Some(i) => i,
        None => return -1,
    };
    // 不能等自己（否则永远等不到）
    if pcb.threads[idx].gettid() == task.gettid() {
        return -1;
    }
    let zombie = pcb.threads[idx].inner_exclusive_access().is_zombie();
    if zombie {
        let exit_code = pcb.threads[idx].inner_exclusive_access().exit_code;
        let dead = pcb.threads.remove(idx);
        drop(dead); // 回收 TCB / 内核栈 / tid
        exit_code as isize
    } else {
        -2
    }
}
