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

/// 对照 rCore 的 `const SYSCALL_WRITE: usize = 64;`
pub const SYSCALL_WRITE: usize = 64;
/// 对照 rCore 的 `const SYSCALL_EXIT: usize = 93;`
pub const SYSCALL_EXIT: usize = 93;
/// 对照 rCore 的 `const SYSCALL_YIELD: usize = 124;`（Linux sched_yield）
pub const SYSCALL_YIELD: usize = 124;
/// 对照 rCore 的 `const SYSCALL_GET_TIME: usize = 169;`
pub const SYSCALL_GET_TIME: usize = 169;

/// 分发（对照 rCore 的 `pub fn syscall(syscall_id: usize, args: [usize; 3]) -> isize`）
pub fn syscall(syscall_id: usize, args: [usize; 3]) -> isize {
    match syscall_id {
        SYSCALL_WRITE => sys_write(args[0], args[1] as *const u8, args[2]),
        SYSCALL_EXIT => sys_exit(args[0] as i32),
        SYSCALL_YIELD => sys_yield(),
        SYSCALL_GET_TIME => sys_get_time(),
        _ => {
            crate::println!("[syscall] 不支持的调用号: {}", syscall_id);
            -1
        }
    }
}
