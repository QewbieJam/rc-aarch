//! 进程/任务类系统调用（ch3）—— 对照 rCore ch3 的 `os/src/syscall/process.rs`
//!
//! ch2 里 `sys_exit` 是"杀掉 app 换下一个"（`batch::run_next_app`）；
//! ch3 改成走任务调度器：
//!   - `sys_exit`  → `task::exit_current_and_run_next()`（状态置 Exited，不再调度）
//!   - `sys_yield` → `task::suspend_current_and_run_next()`（状态回 Ready，让出 CPU）
//!   - `sys_get_time` → 毫秒时间（ch3 的 app 用它观察"时间片"）

use crate::task::{change_program_brk, exit_current_and_run_next, suspend_current_and_run_next};
use crate::timer::get_time_ms;

/// 退出（对照 rCore 的 `pub fn sys_exit(exit_code: i32) -> !`）
///
/// 注意返回类型是 `!`：一个任务退出之后就再也不会被调度回来了。
pub fn sys_exit(exit_code: i32) -> ! {
    crate::println!("[kernel] Application exited with code {}", exit_code);
    exit_current_and_run_next()
}

/// 主动让出 CPU（对照 rCore 的 `pub fn sys_yield() -> isize`）
pub fn sys_yield() -> isize {
    suspend_current_and_run_next();
    0
}

/// 当前时间（毫秒）（对照 rCore 的 `pub fn sys_get_time() -> isize`）
pub fn sys_get_time() -> isize {
    get_time_ms() as isize
}

/// 改 program break（对照 rCore ch4 的 `pub fn sys_sbrk(size: i32) -> isize`）
///
/// 成功返回**旧的** brk，失败返回 -1。
/// 语义和 Linux 的 brk/sbrk 一致：`sbrk(0)` 查当前 brk，`sbrk(n)` 扩/缩 n 字节。
pub fn sys_sbrk(size: i32) -> isize {
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
