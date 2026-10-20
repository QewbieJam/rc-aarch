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

use crate::mm::{translated_refmut, translated_str};
use crate::task::{
    add_task, current_task, current_user_token, exit_current_and_run_next,
    suspend_current_and_run_next,
};
use crate::timer::get_time_ms;
use alloc::sync::Arc;

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
pub fn sys_exec(path: *const u8) -> isize {
    let token = current_user_token();
    let path = translated_str(token, path);
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
    current_task().unwrap().exec(&buf.as_slice()[..size]);
    0
}

/// 等待子进程（对照 upstream 的 `sys_waitpid`）
///
/// - 没有这个子进程          → -1
/// - 有但还在跑              → -2
/// - 有僵尸子进程            → 回收它，返回它的 pid，并把 exit_code 写回用户指针
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
    });
    if let Some((idx, _)) = pair {
        // 从 children 里摘掉；此时 Arc 只剩我们手里这一个
        let child = inner.children.remove(idx);
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        let exit_code = child.inner_exclusive_access().exit_code;
        // 写回用户指针（父进程的地址空间）
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        // child 在这里 drop：TCB / 页表 / 页帧 / 内核栈 / pid 全部回收
        drop(child);
        found_pid as isize
    } else {
        -2
    }
}
