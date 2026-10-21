//! 任务/进程管理（ch5/ch8）—— 对照 rCore ch8 的 `os/src/task/mod.rs`
//!
//! ch5 的模块划分（和 upstream 一一对应）：
//!   task/context.rs    TaskContext（不变）
//!   task/switch.S/.rs  __switch（不变）
//!   task/task.rs       ProcessControlBlock + TaskControlBlock（ch8 拆开）
//!   task/pid.rs        pid 分配 + 内核栈
//!   task/manager.rs    就绪队列（FIFO）
//!   task/processor.rs  Processor + run_tasks + schedule
//!   task/mod.rs        把上面几个粘起来：suspend / exit / initproc
//!
//! ★ 和 ch4 最大的行为变化：
//!   - 不再"数组里扫 Ready"，而是 **idle 循环 + 就绪队列**；
//!   - 进程退出 → **Zombie**（保留 TCB 等父进程 waitpid 回收），
//!     而不是直接从调度器消失；
//!   - 第一个进程固定是 `initproc`（pid 1），它退出就意味着系统该关机了。

mod action;
mod context;
mod manager;
mod pid;
mod processor;
mod signal;
mod switch;

#[allow(clippy::module_inception)]
mod task;

use crate::loader::get_app_data_by_name;
use crate::sync::UPSafeCell;
use alloc::sync::Arc;
pub use task::{ProcessControlBlock, TaskControlBlock, TaskStatus};

pub use action::SignalAction;
pub use context::TaskContext;
pub use manager::{
    add_task, pid2process, register_process, remove_from_pid2process, wakeup_task,
};
pub use signal::{MAX_SIG, SignalFlags};
pub use processor::{
    current_task, current_task_id, current_trap_cx, current_user_token, run_tasks, schedule,
    take_current_task,
};

/// initproc 的 pid（它退出 = 系统关机）
pub const IDLE_PID: usize = 1;

/// 全局唯一的 initproc（也会被用来"托管"孤儿进程）
static INITPROC: UPSafeCell<Option<Arc<TaskControlBlock>>> = unsafe { UPSafeCell::new(None) };

/// 取 initproc 的 Arc
pub fn initproc() -> Arc<TaskControlBlock> {
    INITPROC
        .exclusive_access()
        .as_ref()
        .expect("initproc 还没创建")
        .clone()
}

/// 创建 initproc 并放进就绪队列（对照 upstream 的 `add_initproc`）
pub fn add_initproc() {
    let initproc = TaskControlBlock::new(
        get_app_data_by_name("initproc").expect("找不到 initproc 这个 app"),
    );
    *INITPROC.exclusive_access() = Some(initproc.clone());
    add_task(initproc);
    // initproc 的代码是刚加载的（新物理页），同步一次 I-cache
    crate::mm::sync_icache_all();
}

/// 主动让出：Running → Ready，回就绪队列，切回调度器
pub fn suspend_current_and_run_next() {
    let task = take_current_task().expect("没有正在运行的进程");
    let mut inner = task.inner_exclusive_access();
    let task_cx_ptr = &mut inner.task_cx as *mut TaskContext;
    inner.task_status = TaskStatus::Ready;
    drop(inner);

    add_task(task);
    schedule(task_cx_ptr);
}

/// 阻塞当前线程：从调度器里摘掉、**不**放回就绪队列，等 `wakeup_task` 再唤醒
///
/// 和 `suspend_current_and_run_next` 的区别：
///   - suspend  = yield（自己回就绪队列，回来继续跑）；
///   - block    = sleep（只有 wakeup 才回来）。
/// 互斥锁/信号量/条件变量等"等一个资源"必须用 block，否则等待者会立刻
/// 又被调度上来、越过阻塞点继续跑（ch8 调试时踩过：semaphore_down 没拦住）。
pub fn block_current_and_run_next() {
    let task = take_current_task().expect("没有正在运行的线程");
    let task_cx_ptr = {
        let mut inner = task.inner_exclusive_access();
        inner.task_status = TaskStatus::Ready;
        &mut inner.task_cx as *mut TaskContext
    };
    schedule(task_cx_ptr);
}

/// 退出当前进程：Running → Zombie，记录退出码，切回调度器
///
/// 返回类型是 `!`：Zombie 永远不会再被调度，所以这个函数不会返回。
pub fn exit_current_and_run_next(exit_code: i32) -> ! {
    let task = take_current_task().expect("没有正在运行的线程");
    let process = task.process();
    let pid = process.getpid();
    // 标记当前线程为 Zombie，记下退出码（waittid 用）
    {
        let mut inner = task.inner_exclusive_access();
        inner.task_status = TaskStatus::Zombie;
        inner.exit_code = exit_code;
    }

    // 主线程退出 = 整个进程退出（同进程的其他线程一并变 Zombie）
    if task.tid == pid {
        // ★ initproc 退出 = 系统该关了（对应 upstream 的 IDLE_PID 判断）
        if pid == IDLE_PID {
            println!("[kernel] initproc 退出，exit_code={}，系统停机", exit_code);
            crate::trap::shutdown_with(exit_code);
        }
        remove_from_pid2process(pid);
        process.exit(exit_code);
        drop(process);
    }
    drop(task);

    // 不再保存上下文，直接切回调度器
    let mut _unused = TaskContext::zero_init();
    schedule(&mut _unused as *mut TaskContext);
    panic!("Zombie 线程不应该被再次调度");
}

/// sbrk：改当前进程的 program break
pub fn change_program_brk(size: i32) -> Option<usize> {
    let task = current_task().expect("没有正在运行的线程");
    let process = task.process();
    let mut inner = process.inner_exclusive_access();
    let old_break = inner.program_brk;
    let new_brk = inner.program_brk as isize + size as isize;
    if new_brk < inner.heap_bottom as isize {
        return None;
    }
    let heap_bottom = inner.heap_bottom;
    let result = if size < 0 {
        inner
            .memory_set
            .shrink_to(heap_bottom.into(), (new_brk as usize).into())
    } else {
        inner
            .memory_set
            .append_to(heap_bottom.into(), (new_brk as usize).into())
    };
    if result {
        inner.program_brk = new_brk as usize;
        let asid = task.asid;
        inner.memory_set.activate(asid);
        Some(old_break)
    } else {
        None
    }
}

// ===========================================================================
// ch7：信号
// ===========================================================================

/// 给当前进程挂一个待处理信号（异常处理也会用：缺页→SIGSEGV、非法指令→SIGILL）
pub fn current_add_signal(signal: SignalFlags) {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    inner.signals.insert(signal);
}

/// 内核自己处理的信号（SIGKILL/SIGSTOP/SIGCONT）
fn call_kernel_signal_handler(signal: SignalFlags) {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if signal == SignalFlags::SIGSTOP {
        inner.frozen = true;
        inner.signals.remove(SignalFlags::SIGSTOP);
    } else if signal == SignalFlags::SIGCONT {
        if inner.signals.contains(SignalFlags::SIGCONT) {
            inner.signals.remove(SignalFlags::SIGCONT);
            inner.frozen = false;
        }
    } else {
        // SIGKILL（以及默认动作就是"杀"的其它内核信号）
        inner.killed = true;
    }
}

/// 把信号交给用户注册的处理函数：备份现场，把 ELR 改成 handler，x0 = 信号号
///
/// ★ 和 RISC-V 的 POSIX 实现不同，upstream 这里**不改用户栈**（没有 SignalContext /
///   trampoline）：处理函数必须自己调用 `sigreturn()` 回到被中断的现场
///   （备份在 TCB 的 `trap_ctx_backup` 里）。AArch64 上照抄，省掉一堆栈布局的坑。
fn call_user_signal_handler(sig: usize, signal: SignalFlags) {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let handler = inner.signal_actions.table[sig].handler;
    if handler != 0 {
        inner.handling_sig = sig as isize;
        inner.signals.remove(signal);
        // 备份现场
        let trap_ctx = inner.get_trap_cx();
        inner.trap_ctx_backup = Some(*trap_ctx);
        // 改现场：跳到处理函数，x0 = 信号号
        trap_ctx.elr_el1 = handler;
        trap_ctx.x[0] = sig;
    } else {
        // handler == 0：默认动作。错误类信号会留给 check_signals_error 去结束进程；
        // 其它信号就当作忽略（upstream 也是只打印一行）。
    }
}

/// 扫一遍 pending 信号，能处理就处理
fn check_pending_signals() {
    for sig in 0..=MAX_SIG {
        let task = current_task().unwrap();
        let task_inner = task.inner_exclusive_access();
        let signal = SignalFlags(1u32 << sig);
        if task_inner.signals.contains(signal) && !task_inner.signal_mask.contains(signal) {
            // 正在处理某个信号时，它的 handler.mask 里列出的信号要再屏蔽
            let mut masked = false;
            let handling_sig = task_inner.handling_sig;
            if handling_sig != -1 {
                masked = task_inner.signal_actions.table[handling_sig as usize]
                    .mask
                    .contains(signal);
            }
            if masked {
                continue;
            }
            drop(task_inner);
            drop(task);
            if signal == SignalFlags::SIGKILL
                || signal == SignalFlags::SIGSTOP
                || signal == SignalFlags::SIGCONT
                || signal == SignalFlags::SIGDEF
            {
                call_kernel_signal_handler(signal);
            } else {
                call_user_signal_handler(sig, signal);
                return;
            }
        }
    }
}

/// 每次从内核返回用户态之前调用（对照 upstream 的 `handle_signals`）
///
/// 被 SIGSTOP 冻住的进程会一直让出 CPU，直到 SIGCONT。
pub fn handle_signals() {
    loop {
        check_pending_signals();
        let (frozen, killed) = {
            let task = current_task().unwrap();
            let inner = task.inner_exclusive_access();
            (inner.frozen, inner.killed)
        };
        if !frozen || killed {
            break;
        }
        suspend_current_and_run_next();
    }
}

/// 当前进程有没有"必须退出"的信号（对照 upstream 的 `check_signals_error_of_current`）
pub fn check_signals_error_of_current() -> Option<(i32, &'static str)> {
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    inner.signals.check_error()
}
