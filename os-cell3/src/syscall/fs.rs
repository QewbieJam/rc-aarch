//! 文件/管道类系统调用（ch6/ch7）—— 对照 rCore ch7 的 `os/src/syscall/fs.rs`
//!
//! ch7 的变化：
//!   - fd 背后不一定是文件了（可能是管道、标准输入输出），所以统一走 `File` trait；
//!   - 新增 `pipe(59)` 和 `dup(24)`；
//!   - 读写用 `UserBuffer`（跨页缓冲区）传给 File 实现。

use crate::fs::{OpenFlags, make_pipe, open_file};
use crate::mm::{UserBuffer, translated_byte_buffer, translated_refmut, translated_str};
use crate::task::{current_task, current_user_token};
use alloc::sync::Arc;

/// 写（对照 upstream 的 `sys_write`）
pub fn sys_write(fd: usize, buffer: *const u8, len: usize) -> isize {
    let token = current_user_token();
    let task = current_task().unwrap();
    let process = task.process();
    let inner = process.inner_exclusive_access();
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
    drop(inner); // 避免和文件操作里的借用冲突
    file.write(UserBuffer::new(translated_byte_buffer(token, buffer, len))) as isize
}

/// 读（对照 upstream 的 `sys_read`）
pub fn sys_read(fd: usize, buffer: *const u8, len: usize) -> isize {
    let token = current_user_token();
    let task = current_task().unwrap();
    let process = task.process();
    let inner = process.inner_exclusive_access();
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
    file.read(UserBuffer::new(translated_byte_buffer(token, buffer, len))) as isize
}

/// 打开（对照 upstream 的 `sys_open`）
pub fn sys_open(path: *const u8, flags: u32) -> isize {
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags)) {
        let task = current_task().unwrap();
        let process = task.process();
        let mut inner = process.inner_exclusive_access();
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
    let process = task.process();
    let mut inner = process.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();
    0
}

/// 创建管道（对照 upstream 的 `sys_pipe`）
///
/// 往用户给的 `usize[2]` 里写 (读端 fd, 写端 fd)。
pub fn sys_pipe(pipe: *mut usize) -> isize {
    let task = current_task().unwrap();
    let token = current_user_token();
    let process = task.process();
    let mut inner = process.inner_exclusive_access();
    let (read_end, write_end) = make_pipe();
    let read_fd = inner.alloc_fd();
    inner.fd_table[read_fd] = Some(read_end);
    let write_fd = inner.alloc_fd();
    inner.fd_table[write_fd] = Some(write_end);
    *translated_refmut(token, pipe) = read_fd;
    *translated_refmut(token, unsafe { pipe.add(1) }) = write_fd;
    0
}

/// 复制 fd（对照 upstream 的 `sys_dup`）
pub fn sys_dup(fd: usize) -> isize {
    let task = current_task().unwrap();
    let process = task.process();
    let mut inner = process.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    let file = match &inner.fd_table[fd] {
        Some(f) => Arc::clone(f),
        None => return -1,
    };
    let new_fd = inner.alloc_fd();
    inner.fd_table[new_fd] = Some(file);
    new_fd as isize
}
