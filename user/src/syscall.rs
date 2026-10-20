//! 用户态系统调用（ch3）
//!
//! 对照 rCore ch3 的 `user/src/syscall.rs`：
//!   rCore: `ecall`，调用号在 **x17(a7)**，参数在 **x10/x11/x12(a0/a1/a2)**，返回值 x10
//!   我们:  `svc #0`，调用号在 **x8**，参数在 **x0/x1/x2**，返回值 x0
//!          （AArch64/Linux 的 ABI 习惯；内核侧 syscall/mod.rs 与之对应）

use core::arch::asm;

const SYSCALL_WRITE: usize = 64;
const SYSCALL_EXIT: usize = 93;
/// ch3 新增：Linux sched_yield 的号，和内核 syscall/mod.rs 保持一致
const SYSCALL_YIELD: usize = 124;
/// ch3 新增：毫秒时间
const SYSCALL_GET_TIME: usize = 169;

/// 发起一次系统调用（对照 rCore 的 `fn syscall(id: usize, args: [usize; 3]) -> isize`）
fn syscall(id: usize, args: [usize; 3]) -> isize {
    let mut ret: isize;
    unsafe {
        asm!(
            "svc #0",
            inlateout("x0") args[0] => ret, // 参数 0 / 返回值
            in("x1") args[1],
            in("x2") args[2],
            in("x8") id,
        );
    }
    ret
}

pub fn sys_write(fd: usize, buffer: &[u8]) -> isize {
    syscall(SYSCALL_WRITE, [fd, buffer.as_ptr() as usize, buffer.len()])
}

pub fn sys_exit(exit_code: i32) -> isize {
    syscall(SYSCALL_EXIT, [exit_code as usize, 0, 0])
}

/// 主动让出 CPU（对照 rCore 的 `pub fn sys_yield() -> isize`）
pub fn sys_yield() -> isize {
    syscall(SYSCALL_YIELD, [0, 0, 0])
}

/// 读毫秒时间（对照 rCore 的 `pub fn sys_get_time() -> isize`）
pub fn sys_get_time() -> isize {
    syscall(SYSCALL_GET_TIME, [0, 0, 0])
}
