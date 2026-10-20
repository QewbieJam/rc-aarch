//! 块设备：页帧池里的 RAM 磁盘（ch6）
//!
//! 把 1MB 连续页帧当成一块"盘"，实现 easy-fs 的 `BlockDevice`。
//! 为什么不用真块设备：Jailhouse cell 里没有分给我们的块设备（SD 卡归 root cell），
//! 而 easy-fs 只依赖 `BlockDevice` trait，换底层不影响文件系统本身。

use alloc::sync::Arc;
use easy_fs::BlockDevice;

/// 磁盘大小（块数），1MB
pub const DISK_BLOCKS: usize = 2048;
const DISK_PAGES: usize = DISK_BLOCKS * easy_fs::BLOCK_SZ / 4096;

pub struct RamDisk {
    /// 起始物理地址（恒等映射，直接当指针用）
    start: usize,
    blocks: usize,
}

// 裸指针本身不是 Send/Sync，但我们单核且只在内核态访问，手动声明
unsafe impl Send for RamDisk {}
unsafe impl Sync for RamDisk {}

impl BlockDevice for RamDisk {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        assert!(block_id < self.blocks, "读越界");
        unsafe {
            core::ptr::copy_nonoverlapping(
                (self.start + block_id * easy_fs::BLOCK_SZ) as *const u8,
                buf.as_mut_ptr(),
                easy_fs::BLOCK_SZ,
            );
        }
    }
    fn write_block(&self, block_id: usize, buf: &[u8]) {
        assert!(block_id < self.blocks, "写越界");
        unsafe {
            core::ptr::copy_nonoverlapping(
                buf.as_ptr(),
                (self.start + block_id * easy_fs::BLOCK_SZ) as *mut u8,
                easy_fs::BLOCK_SZ,
            );
        }
    }
}

/// 从页帧池要一块连续的 1MB，做成 RAM 磁盘
pub fn make_disk() -> Arc<RamDisk> {
    let start_ppn = crate::mm::frame_alloc_contig(DISK_PAGES).expect("RAM 磁盘：页帧不够");
    let start = crate::mm::PhysAddr::from(start_ppn).0;
    // frame_alloc_contig 不清零，这里手动清（格式化反正也会清，但保险）
    unsafe {
        core::ptr::write_bytes(start as *mut u8, 0, DISK_PAGES * 4096);
    }
    crate::println!(
        "[fs] RAM 磁盘: {:#x} 起，{} 块 × {}B = {} KB",
        start,
        DISK_BLOCKS,
        easy_fs::BLOCK_SZ,
        DISK_BLOCKS * easy_fs::BLOCK_SZ / 1024
    );
    Arc::new(RamDisk {
        start,
        blocks: DISK_BLOCKS,
    })
}
