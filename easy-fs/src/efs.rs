//! EasyFileSystem：超级块 + 两个位图 + 区域几何（ch6）
//! —— 对照 upstream 的 `easy-fs/src/efs.rs`
//!
//! 磁盘布局见 `lib.rs` 文件头。这里负责：
//!   - `create`：格式化一块新盘（清空、写超级块、建根目录 inode）
//!   - `open`：读超级块，恢复区域几何
//!   - 分配/释放 inode 和数据块（位图操作）

use super::layout::BlockBuf;
use super::{Bitmap, BlockDevice, DiskInode, DiskInodeType, Inode, Mutex, SuperBlock, BLOCK_SZ};
use alloc::sync::Arc;
use core::ptr;

pub struct EasyFileSystem {
    pub block_device: Arc<dyn BlockDevice>,
    pub inode_bitmap: Bitmap,
    pub data_bitmap: Bitmap,
    inode_area_start_block: u32,
    data_area_start_block: u32,
}

impl EasyFileSystem {
    /// 格式化一块新盘（对照 upstream 的 `create`）
    ///
    /// `inode_bitmap_blocks` 一般给 1：一块位图 = 4096 个 inode 名额，
    /// 但 inode 区就要 4096*128/512 = 1024 块 —— 所以**盘不能太小**。
    /// 我们用的盘是 2048 块（1MB），够放。
    pub fn create(
        block_device: Arc<dyn BlockDevice>,
        total_blocks: u32,
        inode_bitmap_blocks: u32,
    ) -> Arc<Mutex<Self>> {
        let inode_bitmap = Bitmap::new(1, inode_bitmap_blocks as usize);
        let inode_num = inode_bitmap.maximum();
        let inode_area_blocks =
            ((inode_num * core::mem::size_of::<DiskInode>() + BLOCK_SZ - 1) / BLOCK_SZ) as u32;
        let inode_total_blocks = inode_bitmap_blocks + inode_area_blocks;
        assert!(total_blocks > 1 + inode_total_blocks, "盘太小了");
        let data_total_blocks = total_blocks - 1 - inode_total_blocks;
        let data_bitmap_blocks = (data_total_blocks + 4096) / 4097;
        let data_area_blocks = data_total_blocks - data_bitmap_blocks;
        let data_bitmap = Bitmap::new(
            (1 + inode_total_blocks) as usize,
            data_bitmap_blocks as usize,
        );
        let mut efs = Self {
            block_device: Arc::clone(&block_device),
            inode_bitmap,
            data_bitmap,
            inode_area_start_block: 1 + inode_bitmap_blocks,
            data_area_start_block: 1 + inode_total_blocks + data_bitmap_blocks,
        };

        // 1) 清空整块盘
        let zero = BlockBuf::zeroed();
        for i in 0..total_blocks {
            block_device.write_block(i as usize, &zero.0);
        }

        // 2) 写超级块
        let mut sb = BlockBuf::zeroed();
        let super_block = unsafe { &mut *sb.as_mut_ptr::<SuperBlock>() };
        super_block.initialize(
            total_blocks,
            inode_bitmap_blocks,
            inode_area_blocks,
            data_bitmap_blocks,
            data_area_blocks,
        );
        block_device.write_block(0, &sb.0);

        // 3) 建根目录 inode（必须拿到 0 号）
        assert_eq!(efs.alloc_inode(), 0);
        let (root_block_id, root_offset) = efs.get_disk_inode_pos(0);
        let mut buf = BlockBuf::zeroed();
        block_device.read_block(root_block_id as usize, &mut buf.0);
        let root = unsafe { &mut *(buf.0.as_mut_ptr().add(root_offset) as *mut DiskInode) };
        root.initialize(DiskInodeType::Directory);
        block_device.write_block(root_block_id as usize, &buf.0);

        Arc::new(Mutex::new(efs))
    }

    /// 打开一块已经格式化好的盘（对照 upstream 的 `open`）
    pub fn open(block_device: Arc<dyn BlockDevice>) -> Arc<Mutex<Self>> {
        let mut buf = BlockBuf::zeroed();
        block_device.read_block(0, &mut buf.0);
        let super_block = unsafe { ptr::read_unaligned(buf.as_ptr::<SuperBlock>()) };
        assert!(super_block.is_valid(), "不是 easy-fs 的盘（magic 不对）");
        let inode_total_blocks = super_block.inode_bitmap_blocks + super_block.inode_area_blocks;
        let efs = Self {
            block_device,
            inode_bitmap: Bitmap::new(1, super_block.inode_bitmap_blocks as usize),
            data_bitmap: Bitmap::new(
                (1 + inode_total_blocks) as usize,
                super_block.data_bitmap_blocks as usize,
            ),
            inode_area_start_block: 1 + super_block.inode_bitmap_blocks,
            data_area_start_block: 1 + inode_total_blocks + super_block.data_bitmap_blocks,
        };
        Arc::new(Mutex::new(efs))
    }

    /// 根目录的 Inode
    pub fn root_inode(efs: &Arc<Mutex<Self>>) -> Inode {
        let block_device = Arc::clone(&efs.lock().block_device);
        let (block_id, block_offset) = efs.lock().get_disk_inode_pos(0);
        Inode::new(block_id, block_offset, Arc::clone(efs), block_device)
    }

    /// inode 号 → (所在块号, 块内偏移)
    pub fn get_disk_inode_pos(&self, inode_id: u32) -> (u32, usize) {
        let inode_size = core::mem::size_of::<DiskInode>();
        let inodes_per_block = (BLOCK_SZ / inode_size) as u32;
        let block_id = self.inode_area_start_block + inode_id / inodes_per_block;
        (
            block_id,
            (inode_id % inodes_per_block) as usize * inode_size,
        )
    }

    /// 数据块号（相对）→ 实际磁盘块号
    pub fn get_data_block_id(&self, data_block_id: u32) -> u32 {
        self.data_area_start_block + data_block_id
    }

    /// 分配一个 inode 号
    pub fn alloc_inode(&mut self) -> u32 {
        self.inode_bitmap.alloc(self.block_device.as_ref()).unwrap() as u32
    }

    /// 分配一个数据块（返回**实际**磁盘块号）
    pub fn alloc_data(&mut self) -> u32 {
        self.data_bitmap.alloc(self.block_device.as_ref()).unwrap() as u32 + self.data_area_start_block
    }

    /// 释放一个数据块（先清零再清位图）
    pub fn dealloc_data(&mut self, block_id: u32) {
        let zero = BlockBuf::zeroed();
        self.block_device.write_block(block_id as usize, &zero.0);
        self.data_bitmap.dealloc(
            self.block_device.as_ref(),
            (block_id - self.data_area_start_block) as usize,
        );
    }
}
