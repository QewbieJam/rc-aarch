//! 文件类系统调用（ch4）—— 对照 rCore ch4 的 `os/src/syscall/fs.rs`
//!
//! ★ ch4 和 ch3 最大的区别：**用户指针不能直接解引用了**。
//!   ch3 里内核和用户共用一套物理地址，`buf` 指针在内核里读就是用户的内存。
//!   ch4 每个任务有自己的页表，用户 VA 在内核当前页表下未必有效 → 必须
//!   先用**当前任务的页表**把 [buf, buf+len) 翻译成物理页，再从物理页读。
//!   这就是 `mm::translated_byte_buffer` 干的事（对照 upstream 的同名函数）。

use crate::mm::translated_byte_buffer;
use crate::task::current_user_token;

const FD_STDOUT: usize = 1;

/// 写（对照 upstream 的 `sys_write`）
pub fn sys_write(fd: usize, buffer: *const u8, len: usize) -> isize {
    if fd != FD_STDOUT {
        return -1;
    }
    let buffers = translated_byte_buffer(current_user_token(), buffer, len);
    for buf in buffers {
        for &b in buf.iter() {
            crate::console::uart_putc(b);
        }
    }
    len as isize
}
