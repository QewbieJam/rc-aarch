//! `__switch` 的 Rust 包装（ch3）—— 对照 rCore ch3 的 `os/src/task/switch.rs`
//!
//! 真正的切换在 `switch.S` 里（必须汇编，理由见那边注释）。这里只是把
//! `global_asm!` 拉进来 + 声明符号，让 Rust 侧能安全地调用。

use super::TaskContext;
use core::arch::global_asm;

global_asm!(include_str!("switch.S"));

unsafe extern "C" {
    /// 保存当前上下文到 `current`，再从 `next` 恢复并跳过去。
    ///
    /// # Safety
    /// - 两个指针都必须指向有效的 `TaskContext`；
    /// - 调用前必须把手里的借用/锁都 drop 掉（这个函数"半路离开"当前执行流，
    ///   回来时已经过了任意长的时间，期间别的任务可能动过全局状态）。
    pub unsafe fn __switch(
        current_task_cx_ptr: *mut TaskContext,
        next_task_cx_ptr: *const TaskContext,
    );
}
