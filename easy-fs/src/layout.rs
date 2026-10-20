//! 磁盘布局（ch6）—— 对照 upstream 的 `easy-fs/src/layout.rs`
//!
//! 这里定义了"磁盘上长什么样"：超级块、磁盘 inode、目录项。
//! 所有结构都是 `#[repr(C)]`，直接按字节读写块设备。
//!
//! 磁盘布局：
//! ```text
//!   block 0                 SuperBlock
//!   block 1 ..              inode 位图（1 bit / inode）
//!   ...                     inode 区（1 块 = 512/128 = 4 个 DiskInode）
//!   ...                     data 位图（1 bit / 数据块）
//!   ...                     data 区
//! ```
//!
//! ★ 我们只实现了 **direct + indirect1** 两级索引（upstream 还有 indirect2）：
//!   单文件最大 28 + 128 = 156 块 ≈ 78KB，放 app 和测试文件足够了。
//!   超过会在 `increase_size` 里 panic，不会静默写坏。

use super::{BlockDevice, BLOCK_SZ};
use alloc::vec::Vec;
use core::fmt::{self, Debug, Formatter};

/// 超级块的魔数
const EFS_MAGIC: u32 = 0x3b80_0001;
/// 直接索引的块数
pub const INODE_DIRECT_COUNT: usize = 28;
/// 文件名最大长度（不含结尾的 '\0'）
pub const NAME_LENGTH_LIMIT: usize = 27;
/// 一级间接索引能覆盖的块数
pub const INODE_INDIRECT1_COUNT: usize = BLOCK_SZ / 4;
/// 直接索引的边界
pub const DIRECT_BOUND: usize = INODE_DIRECT_COUNT;
/// 一级间接索引的边界（我们支持的最大值）
pub const INDIRECT1_BOUND: usize = DIRECT_BOUND + INODE_INDIRECT1_COUNT;

/// 超级块（对照 upstream 的 `SuperBlock`）
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SuperBlock {
    magic: u32,
    pub total_blocks: u32,
    pub inode_bitmap_blocks: u32,
    pub inode_area_blocks: u32,
    pub data_bitmap_blocks: u32,
    pub data_area_blocks: u32,
}

impl Debug for SuperBlock {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SuperBlock")
            .field("total_blocks", &self.total_blocks)
            .field("inode_bitmap_blocks", &self.inode_bitmap_blocks)
            .field("inode_area_blocks", &self.inode_area_blocks)
            .field("data_bitmap_blocks", &self.data_bitmap_blocks)
            .field("data_area_blocks", &self.data_area_blocks)
            .finish()
    }
}

impl SuperBlock {
    pub fn initialize(
        &mut self,
        total_blocks: u32,
        inode_bitmap_blocks: u32,
        inode_area_blocks: u32,
        data_bitmap_blocks: u32,
        data_area_blocks: u32,
    ) {
        *self = Self {
            magic: EFS_MAGIC,
            total_blocks,
            inode_bitmap_blocks,
            inode_area_blocks,
            data_bitmap_blocks,
            data_area_blocks,
        };
    }
    pub fn is_valid(&self) -> bool {
        self.magic == EFS_MAGIC
    }
}

/// inode 类型
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiskInodeType {
    File,
    Directory,
}

/// 磁盘 inode（对照 upstream 的 `DiskInode`，128 字节）
///
/// 保留 `indirect2` 字段是为了和 upstream 的布局一致（虽然我们不用它）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DiskInode {
    pub size: u32,
    pub direct: [u32; INODE_DIRECT_COUNT],
    pub indirect1: u32,
    pub indirect2: u32,
    type_: DiskInodeType,
}

/// 一个 512 字节的块缓冲区，**8 字节对齐**（这样转到 DiskInode/DirEntry/间接块
/// 这些结构体指针时不会出现未对齐访问）
#[repr(C, align(8))]
pub struct BlockBuf(pub [u8; BLOCK_SZ]);

impl BlockBuf {
    pub const fn zeroed() -> Self {
        Self([0; BLOCK_SZ])
    }
    pub fn as_ptr<T>(&self) -> *const T {
        self.0.as_ptr() as *const T
    }
    pub fn as_mut_ptr<T>(&mut self) -> *mut T {
        self.0.as_mut_ptr() as *mut T
    }
}

/// 一级间接块：512/4 = 128 个 u32 块号
type IndirectBlock = [u32; INODE_INDIRECT1_COUNT];

impl DiskInode {
    pub fn initialize(&mut self, type_: DiskInodeType) {
        self.size = 0;
        for v in self.direct.iter_mut() {
            *v = 0;
        }
        self.indirect1 = 0;
        self.indirect2 = 0;
        self.type_ = type_;
    }
    pub fn is_dir(&self) -> bool {
        self.type_ == DiskInodeType::Directory
    }
    pub fn is_file(&self) -> bool {
        self.type_ == DiskInodeType::File
    }

    /// 文件占多少个数据块
    pub fn data_blocks(&self) -> u32 {
        Self::_data_blocks(self.size)
    }
    fn _data_blocks(size: u32) -> u32 {
        (size + BLOCK_SZ as u32 - 1) / BLOCK_SZ as u32
    }
    /// 含间接块在内一共要多少块
    pub fn total_blocks(size: u32) -> u32 {
        let data_blocks = Self::_data_blocks(size) as usize;
        let mut total = data_blocks;
        if data_blocks > INODE_DIRECT_COUNT {
            total += 1; // indirect1
        }
        total as u32
    }
    /// 把 size 撑到 new_size 需要额外多少块
    pub fn blocks_num_needed(&self, new_size: u32) -> u32 {
        assert!(new_size >= self.size);
        Self::total_blocks(new_size) - Self::total_blocks(self.size)
    }

    /// 第 `inner_id` 个数据块的实际块号（direct / indirect1）
    pub fn get_block_id(&self, inner_id: u32, dev: &dyn BlockDevice) -> u32 {
        let inner_id = inner_id as usize;
        if inner_id < INODE_DIRECT_COUNT {
            self.direct[inner_id]
        } else {
            let index = inner_id - INODE_DIRECT_COUNT;
            assert!(index < INODE_INDIRECT1_COUNT, "文件太大（只支持 direct+indirect1）");
            let mut block = BlockBuf::zeroed();
            dev.read_block(self.indirect1 as usize, &mut block.0);
            let indirect = unsafe { &*block.as_ptr::<IndirectBlock>() };
            indirect[index]
        }
    }

    /// 把 size 撑到 new_size，并把 `new_blocks` 里的块挂上去
    pub fn increase_size(
        &mut self,
        new_size: u32,
        new_blocks: Vec<u32>,
        dev: &dyn BlockDevice,
    ) {
        let mut current_blocks = self.data_blocks();
        self.size = new_size;
        let total_blocks = self.data_blocks();
        assert!(
            total_blocks as usize <= INDIRECT1_BOUND,
            "文件太大（{} 块 > 上限 {}）—— easy-fs 只实现了 direct+indirect1",
            total_blocks,
            INDIRECT1_BOUND
        );
        let mut new_blocks = new_blocks.into_iter();

        // 1) 填 direct
        while current_blocks < total_blocks.min(INODE_DIRECT_COUNT as u32) {
            self.direct[current_blocks as usize] = new_blocks.next().unwrap();
            current_blocks += 1;
        }
        // 2) 填 indirect1
        if total_blocks > INODE_DIRECT_COUNT as u32 {
            if current_blocks == INODE_DIRECT_COUNT as u32 {
                self.indirect1 = new_blocks.next().unwrap();
            }
            let mut block = BlockBuf::zeroed();
            dev.read_block(self.indirect1 as usize, &mut block.0);
            let indirect = unsafe { &mut *block.as_mut_ptr::<IndirectBlock>() };
            let mut i = (current_blocks - INODE_DIRECT_COUNT as u32) as usize;
            let end = (total_blocks - INODE_DIRECT_COUNT as u32) as usize;
            while i < end {
                indirect[i] = new_blocks.next().unwrap();
                i += 1;
            }
            dev.write_block(self.indirect1 as usize, &block.0);
        }
    }

    /// 从 offset 读进 buf，返回读了多少字节
    pub fn read_at(&self, offset: usize, buf: &mut [u8], dev: &dyn BlockDevice) -> usize {
        if offset >= self.size as usize {
            return 0;
        }
        let mut start = offset;
        let end = (offset + buf.len()).min(self.size as usize);
        let mut read = 0usize;
        let mut block = BlockBuf::zeroed();
        while start < end {
            let block_id = self.get_block_id((start / BLOCK_SZ) as u32, dev) as usize;
            let block_offset = start % BLOCK_SZ;
            let n = (BLOCK_SZ - block_offset).min(end - start);
            dev.read_block(block_id, &mut block.0);
            buf[read..read + n].copy_from_slice(&block.0[block_offset..block_offset + n]);
            read += n;
            start += n;
        }
        read
    }

    /// 从 offset 写入 buf，返回写了多少字节
    pub fn write_at(&mut self, offset: usize, buf: &[u8], dev: &dyn BlockDevice) -> usize {
        assert!(offset + buf.len() <= self.size as usize, "write_at 超出文件大小");
        let mut start = offset;
        let end = offset + buf.len();
        let mut written = 0usize;
        let mut block = BlockBuf::zeroed();
        while start < end {
            let block_id = self.get_block_id((start / BLOCK_SZ) as u32, dev) as usize;
            let block_offset = start % BLOCK_SZ;
            let n = (BLOCK_SZ - block_offset).min(end - start);
            dev.read_block(block_id, &mut block.0);
            block.0[block_offset..block_offset + n].copy_from_slice(&buf[written..written + n]);
            dev.write_block(block_id, &block.0);
            written += n;
            start += n;
        }
        written
    }

    /// 清空文件，返回要回收的数据块
    pub fn clear_size(&mut self, dev: &dyn BlockDevice) -> Vec<u32> {
        let mut v: Vec<u32> = Vec::new();
        let mut block = BlockBuf::zeroed();
        let blocks = self.data_blocks();
        let direct_end = (blocks as usize).min(INODE_DIRECT_COUNT);
        for i in 0..direct_end {
            v.push(self.direct[i]);
            self.direct[i] = 0;
        }
        if blocks as usize > INODE_DIRECT_COUNT {
            dev.read_block(self.indirect1 as usize, &mut block.0);
            let indirect = unsafe { &*block.as_ptr::<IndirectBlock>() };
            let mut i = 0;
            while i < (blocks as usize - INODE_DIRECT_COUNT) {
                v.push(indirect[i]);
                i += 1;
            }
            v.push(self.indirect1);
            self.indirect1 = 0;
        }
        self.size = 0;
        v
    }
}

/// 目录项（32 字节：28 字节名字 + 4 字节 inode 号）
pub const DIRENT_SZ: usize = 32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DirEntry {
    name: [u8; NAME_LENGTH_LIMIT + 1],
    inode_number: u32,
}

impl DirEntry {
    pub fn empty() -> Self {
        Self {
            name: [0; NAME_LENGTH_LIMIT + 1],
            inode_number: 0,
        }
    }
    pub fn new(name: &str, inode_number: u32) -> Self {
        let mut bytes = [0u8; NAME_LENGTH_LIMIT + 1];
        let src = name.as_bytes();
        assert!(src.len() <= NAME_LENGTH_LIMIT, "文件名太长");
        bytes[..src.len()].copy_from_slice(src);
        Self {
            name: bytes,
            inode_number,
        }
    }
    pub fn name(&self) -> &str {
        let mut len = 0;
        while len < NAME_LENGTH_LIMIT && self.name[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.name[..len]).unwrap()
    }
    pub fn inode_number(&self) -> u32 {
        self.inode_number
    }
    pub fn as_bytes(&self) -> &[u8; DIRENT_SZ] {
        unsafe { &*(self as *const Self as *const [u8; DIRENT_SZ]) }
    }
    pub fn as_bytes_mut(&mut self) -> &mut [u8; DIRENT_SZ] {
        unsafe { &mut *(self as *mut Self as *mut [u8; DIRENT_SZ]) }
    }
}

// 确保布局和假设一致（编译期检查）
const _: () = assert!(core::mem::size_of::<DiskInode>() == 128);
const _: () = assert!(core::mem::size_of::<DirEntry>() == DIRENT_SZ);
const _: () = assert!(core::mem::size_of::<SuperBlock>() == 24);
