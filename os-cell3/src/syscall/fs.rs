//! 文件类系统调用（ch6）—— 对照 rCore ch6 的 `os/src/syscall/fs.rs`
//!
//! ch6 新增：`open(56)` / `close(57)` / `read(63)`；`write(64)` 不再是"只写串口"，
//! 而是按 fd 分发：
//!   fd 1（stdout）→ 直接打印
//!   fd ≥ 3        → 找当前进程 fd_table 里的 OSInode，读写文件
//!   fd 0（stdin） → 还没接串口输入，返回 -1（ch7 再说）

use crate::fs::{OpenFlags, open_file};
use crate::mm::{translated_byte_buffer, translated_str};
use crate::task::{current_task, current_user_token};
use alloc::vec::Vec;

const FD_STDIN: usize = 0;
const FD_STDOUT: usize = 1;

/// 写（对照 upstream 的 `sys_write`）
pub fn sys_write(fd: usize, buffer: *const u8, len: usize) -> isize {
    // stdout：不进 fd_table，直接打印
    if fd == FD_STDOUT {
        let token = current_user_token();
        for buf in translated_byte_buffer(token, buffer, len) {
            for &b in buf.iter() {
                crate::console::uart_putc(b);
            }
        }
        return len as isize;
    }
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    let file = match &inner.fd_table[fd] {
        Some(f) => f.clone(),
        None => return -1,
    };
    if !file.writable() {
        return -1;
    }
    drop(inner); // 避免和文件操作里可能的借用冲突
    let bufs = translated_byte_buffer(token, buffer, len);
    let refs: Vec<&[u8]> = bufs.iter().map(|b| &**b).collect();
    file.write(&refs) as isize
}

/// 读（对照 upstream 的 `sys_read`）
pub fn sys_read(fd: usize, buffer: *const u8, len: usize) -> isize {
    if fd == FD_STDIN {
        // TODO(ch7)：接 UART 输入
        return -1;
    }
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    let file = match &inner.fd_table[fd] {
        Some(f) => f.clone(),
        None => return -1,
    };
    if !file.readable() {
        return -1;
    }
    drop(inner);
    let mut bufs = translated_byte_buffer(token, buffer, len);
    file.read(&mut bufs) as isize
}

/// 打开（对照 upstream 的 `sys_open`）
pub fn sys_open(path: *const u8, flags: u32) -> isize {
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags)) {
        let task = current_task().unwrap();
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        -1
    }
}

/// 关闭（对照 upstream 的 `sys_close`）
pub fn sys_close(fd: usize) -> isize {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();
    0
}
