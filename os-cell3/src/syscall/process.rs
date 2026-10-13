//! 进程类系统调用（阶段 2 / ch2-B）
//!
//! 对照 rCore ch2 的 `os/src/syscall/process.rs`：
//!   rCore: `pub fn sys_exit(exit_code: i32) -> !` → `sbi::shutdown(exit_code)`
//!          （关掉整台机器，因为 ch2 的批处理是"所有 app 跑完就结束"）
//!   我们:  裸机 cell3 里没有"关机"（`hvc` 也没有关机服务），所以先打印后停住；
//!          ch2-C 会改成"干掉当前 app、换下一个 app"（对应 `batch::run_next_app`）

/// 退出（对照 rCore ch2 的 `pub fn sys_exit(exit_code: i32) -> ! { run_next_app() }`）
///
/// rCore ch2 里 app 主动退出 → 内核切下一个 app；我们完全照做。
pub fn sys_exit(exit_code: i32) -> ! {
    crate::println!("[kernel] Application exited with code {}", exit_code);
    crate::batch::run_next_app()
}
