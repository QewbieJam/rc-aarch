//! 系统调用分发（阶段 3 / ch3）
//!
//! 对照 rCore ch3 的 `os/src/syscall/mod.rs`：
//!   rCore: `syscall(syscall_id, args)`，调用号在 `x[17]`(a7)，参数在 `x[10..12]`(a0..a2)
//!   我们:  调用号在 **x8**，参数在 **x0/x1/x2**，返回值放回 **x0**
//!          —— 这是 AArch64/Linux 的 ABI 习惯（x8 = syscall number）
//!
//! ch3 新增：
//!   124 sched_yield  —— 主动让出 CPU（对照 rCore 的 SYSCALL_YIELD = 124）
//!   169 get_time     —— 毫秒时间（对照 rCore 的 SYSCALL_GET_TIME = 169）
//!   调用号和 Linux/AArch64 对齐，将来换到真 Linux ABI 时不用改号。

mod fs;
mod process;

pub use fs::*;
pub use process::*;

/// 对照 rCore 的 `const SYSCALL_READ: usize = 63;`
pub const SYSCALL_READ: usize = 63;
/// 对照 rCore 的 `const SYSCALL_WRITE: usize = 64;`
pub const SYSCALL_WRITE: usize = 64;
/// ch6：开/关文件（Linux open/close）
pub const SYSCALL_OPEN: usize = 56;
pub const SYSCALL_CLOSE: usize = 57;
/// ch7：dup + pipe
pub const SYSCALL_DUP: usize = 24;
pub const SYSCALL_PIPE: usize = 59;
/// ch7：信号
pub const SYSCALL_KILL: usize = 129;
pub const SYSCALL_SIGACTION: usize = 134;
pub const SYSCALL_SIGPROCMASK: usize = 135;
pub const SYSCALL_SIGRETURN: usize = 139;
/// 对照 rCore 的 `const SYSCALL_EXIT: usize = 93;`
pub const SYSCALL_EXIT: usize = 93;
/// 对照 rCore 的 `const SYSCALL_YIELD: usize = 124;`（Linux sched_yield）
pub const SYSCALL_YIELD: usize = 124;
/// 对照 rCore 的 `const SYSCALL_GET_TIME: usize = 169;`
pub const SYSCALL_GET_TIME: usize = 169;
/// ch4 新增：改 program break（Linux brk，214）
pub const SYSCALL_SBRK: usize = 214;
/// ch5 新增：进程相关（编号和 Linux/aarch64 对齐）
pub const SYSCALL_GETPID: usize = 172;
pub const SYSCALL_FORK: usize = 220;
pub const SYSCALL_EXEC: usize = 221;
pub const SYSCALL_WAITPID: usize = 260;

/// 分发（对照 rCore 的 `pub fn syscall(syscall_id: usize, args: [usize; 3]) -> isize`）
pub fn syscall(syscall_id: usize, args: [usize; 3]) -> isize {
    match syscall_id {
        SYSCALL_READ => sys_read(args[0], args[1] as *const u8, args[2]),
        SYSCALL_WRITE => sys_write(args[0], args[1] as *const u8, args[2]),
        SYSCALL_DUP => sys_dup(args[0]),
        SYSCALL_OPEN => sys_open(args[0] as *const u8, args[1] as u32),
        SYSCALL_CLOSE => sys_close(args[0]),
        SYSCALL_PIPE => sys_pipe(args[0] as *mut usize),
        SYSCALL_KILL => sys_kill(args[0], args[1] as i32),
        SYSCALL_SIGACTION => sys_sigaction(
            args[0] as i32,
            args[1] as *const crate::task::SignalAction,
            args[2] as *mut crate::task::SignalAction,
        ),
        SYSCALL_SIGPROCMASK => sys_sigprocmask(args[0] as u32),
        SYSCALL_SIGRETURN => sys_sigreturn(),
        SYSCALL_EXIT => sys_exit(args[0] as i32),
        SYSCALL_YIELD => sys_yield(),
        SYSCALL_GET_TIME => sys_get_time(),
        SYSCALL_SBRK => sys_sbrk(args[0] as i32),
        SYSCALL_GETPID => sys_getpid(),
        SYSCALL_FORK => sys_fork(),
        SYSCALL_EXEC => sys_exec(args[0] as *const u8, args[1] as *const usize),
        SYSCALL_WAITPID => sys_waitpid(args[0] as isize, args[1] as *mut i32),
        _ => {
            crate::println!("[syscall] 不支持的调用号: {}", syscall_id);
            -1
        }
    }
}
