//! easy-fs：一个"教学用"的极简文件系统（ch6）
//! —— 对照 rCore-Tutorial-v3 ch6 的 `easy-fs` crate
//!
//! 磁盘布局（和 upstream 完全一致，方便对照）：
//!
//! ```text
//!   block 0            SuperBlock（magic + 各区域块数）
//!   block 1..          inode 位图（每 bit 一个 inode 是否被占用）
//!   ...                inode 区（每块 512/128 = 4 个 DiskInode）
//!   ...                data 位图（每 bit 一个数据块是否被占用）
//!   ... 末尾           data 区（真正存文件内容）
//! ```
//!
//! ★ 和 upstream 的差别（为了少抄点代码、也为了适配我们的环境）：
//!   1) **没有 block cache**：直接对块设备读/写。upstream 缓存块是为了减少 virtio 往返，
//!      我们的块设备是内存里的 RAM 磁盘，直接读写既简单又不慢。
//!   2) **没实现 indirect2**（超大文件的三级索引）。一个文件最多
//!      28 + 128 = 156 块 × 512B ≈ 78KB，够放我们的 app 和测试文件；
//!      超过会 panic（文档里写清楚）。
//!   3) 锁用 `Mutex`（单核 RefCell 语义）代替 `spin::Mutex`。

#![no_std]

extern crate alloc;

mod bitmap;
mod block_dev;
mod efs;
mod layout;
mod vfs;

/// 块大小（512 字节，和 upstream 一致）
pub const BLOCK_SZ: usize = 512;

pub use bitmap::Bitmap;
pub use block_dev::BlockDevice;
pub use efs::EasyFileSystem;
pub use layout::{DirEntry, DiskInode, DiskInodeType, SuperBlock};
pub use vfs::Inode;

use core::cell::{RefCell, RefMut};

/// 单核用的自旋"锁"（其实就是 RefCell + unsafe Sync）
///
/// 为什么不用 `spin::Mutex`：我们的内核是单核的，而且文件系统操作全部发生在
/// 内核态（PSTATE.I=1），不存在真正的并发。RefCell 还能帮我们发现"重入加锁"
/// 这种逻辑错误（重入会 panic 而不是死锁）。
pub struct Mutex<T> {
    inner: RefCell<T>,
}

unsafe impl<T> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// 新建
    pub const fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }
    /// 加锁（返回可变借用）
    pub fn lock(&self) -> RefMut<'_, T> {
        self.inner.borrow_mut()
    }
}
