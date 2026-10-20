//! 内核文件系统层（ch6/ch7）
//! —— 对照 rCore ch6/ch7 的 `os/src/fs/{mod,inode,pipe,stdio}.rs` + `drivers/block/*`
//!
//! 结构：
//!   block.rs   RAM 磁盘（页帧池里的连续 1MB）实现 easy-fs 的 BlockDevice
//!   inode.rs   OSInode / OpenFlags / open_file / 根目录
//!   stdio.rs   Stdin / Stdout
//!   pipe.rs    管道（ch7）
//!
//! `File` trait 是 fd 分发的核心：一个 fd 背后可能是文件、管道、标准输入输出，
//! 系统调用只认 `readable/writable/read/write` 这几个方法。

mod block;
mod inode;
mod pipe;
mod stdio;

use crate::mm::UserBuffer;
use alloc::sync::Arc;
use easy_fs::EasyFileSystem;

/// 所有"能当 fd 用"的东西都要实现这个 trait
pub trait File: Send + Sync {
    fn readable(&self) -> bool;
    fn writable(&self) -> bool;
    fn read(&self, buf: UserBuffer) -> usize;
    fn write(&self, buf: UserBuffer) -> usize;
}

#[allow(unused_imports)]
pub use inode::{
    OSInode, OpenFlags, file_size, list_files, open_file, read_file_by_name, read_file_to,
    root_inode,
};
pub use pipe::make_pipe;
pub use stdio::{Stdin, Stdout};

/// 初始化文件系统（ch6）：
/// 造盘 → 格式化 → 把内嵌 app 写成文件 → 重新 open 自检 → 设为根目录
pub fn init() {
    let disk = block::make_disk();

    // 格式化 + 写入文件
    let efs = EasyFileSystem::create(disk.clone(), block::DISK_BLOCKS as u32, 1);
    let root = EasyFileSystem::root_inode(&efs);

    let num_app = crate::loader::get_num_app();
    for i in 0..num_app {
        let name = crate::loader::app_name(i);
        let data = crate::loader::get_app_data(i);
        let inode = root
            .create(name)
            .unwrap_or_else(|| panic!("创建 {} 失败", name));
        let n = inode.write_at(0, data);
        assert_eq!(n, data.len(), "写入 {} 不完整", name);
        crate::println!("[fs] 写入 /{}（{} 字节）", name, n);
    }
    // 再放一个纯文本文件，给 cat 用
    let hello = root.create("hello.txt").expect("创建 hello.txt 失败");
    hello.write_at(0, b"hello from easy-fs on rc-aarch!\n");

    // 重新 open 一遍（验证超级块/位图/目录都真的落到盘上了）
    let efs2 = EasyFileSystem::open(disk.clone());
    let root2 = EasyFileSystem::root_inode(&efs2);
    inode::set_root(Arc::new(root2));
    crate::println!("[fs] 挂载成功，根目录: {:?}", list_files());
}
