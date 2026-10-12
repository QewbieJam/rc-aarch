//! 文件类系统调用（阶段 2 / ch2-B）
//!
//! 对照 rCore ch2 的 `os/src/syscall/fs.rs`：只有一个 `sys_write`。

/// 写（对照 rCore 的 `pub fn sys_write(fd: usize, buffer: *const u8, len: usize) -> isize`）
///
/// rCore 里输出最终走 SBI（`console_putchar`）；我们走
/// `console::uart_putc`（`hvc` 进 hypervisor 虚拟控制台 + mini-UART 直写）。
/// 这也是"内核态"的输出，用户程序只能通过这个 syscall 间接打印。
pub fn sys_write(fd: usize, buffer: *const u8, len: usize) -> isize {
    if fd != 1 {
        // 只支持 stdout（fd=1），与 rCore ch2 的行为一致
        return -1;
    }
    // 安全检查：调用者（用户程序）给的指针必须落在本 cell 的内存里，
    // 否则内核会跟着一起崩。ch2 阶段只做最粗的检查：
    //   必须落在 cell3 的可写 RAM 区（0xFA000000 ~ 0xFC000000）
    let addr = buffer as usize;
    const CELL_BASE: usize = 0xFA000000;
    const CELL_END: usize = 0xFC000000;
    if addr < CELL_BASE || addr.saturating_add(len) > CELL_END {
        crate::println!("[sys_write] 非法缓冲区 {:#x}+{}", addr, len);
        return -1;
    }
    let slice = unsafe { core::slice::from_raw_parts(buffer, len) };
    // 不是 UTF-8 也没关系：按字节原样输出
    for &b in slice {
        crate::console::uart_putc(b);
    }
    len as isize
}
