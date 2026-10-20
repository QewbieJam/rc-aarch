//! VFS 层：Inode（ch6）—— 对照 upstream 的 `easy-fs/src/vfs.rs`
//!
//! 一个 `Inode` = (磁盘 inode 的位置) + (指向整个文件系统的 Arc) + (块设备)。
//! 文件内容/目录项都通过 `read_at/write_at` 按字节读写。
//!
//! ★ 和 upstream 的差别：没有 block cache，`read_disk_inode/modify_disk_inode`
//!   直接读/写块设备（我们的盘在内存里，没所谓）。

use super::layout::{BlockBuf, DIRENT_SZ};
use super::{
    BlockDevice, DirEntry, DiskInode, DiskInodeType, EasyFileSystem, Mutex, BLOCK_SZ,
};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::RefMut;

pub struct Inode {
    block_id: usize,
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
}

impl Inode {
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
        }
    }

    /// 读出磁盘 inode 交给 f
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        let mut buf = BlockBuf::zeroed();
        self.block_device.read_block(self.block_id, &mut buf.0);
        let inode = unsafe { &*(buf.0.as_ptr().add(self.block_offset) as *const DiskInode) };
        f(inode)
    }

    /// 修改磁盘 inode（f 返回后写回磁盘）
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        let mut buf = BlockBuf::zeroed();
        self.block_device.read_block(self.block_id, &mut buf.0);
        let inode = unsafe { &mut *(buf.0.as_mut_ptr().add(self.block_offset) as *mut DiskInode) };
        let v = f(inode);
        self.block_device.write_block(self.block_id, &buf.0);
        v
    }

    /// 在目录里按名字找 inode 号
    fn find_inode_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        assert!(disk_inode.is_dir());
        let file_count = (disk_inode.size as usize) / DIRENT_SZ;
        for i in 0..file_count {
            let mut dirent = DirEntry::empty();
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), self.block_device.as_ref()),
                DIRENT_SZ
            );
            if dirent.name() == name {
                return Some(dirent.inode_number());
            }
        }
        None
    }

    /// 按名字找子 inode
    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            self.find_inode_id(name, disk_inode).map(|inode_id| {
                let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
                Arc::new(Self::new(
                    block_id,
                    block_offset,
                    self.fs.clone(),
                    self.block_device.clone(),
                ))
            })
        })
    }

    /// 把 inode 的 size 撑大（顺便分配数据块）
    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut RefMut<'_, EasyFileSystem>,
    ) {
        if new_size < disk_inode.size {
            return;
        }
        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, self.block_device.as_ref());
    }

    /// 在当前目录下创建文件（对照 upstream 的 `create`）
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        // 已经有同名文件？
        if self
            .read_disk_inode(|root_inode| self.find_inode_id(name, root_inode))
            .is_some()
        {
            return None;
        }
        // 分配并初始化新 inode
        let new_inode_id = fs.alloc_inode();
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
        let mut buf = BlockBuf::zeroed();
        self.block_device
            .read_block(new_inode_block_id as usize, &mut buf.0);
        let new_inode =
            unsafe { &mut *(buf.0.as_mut_ptr().add(new_inode_block_offset) as *mut DiskInode) };
        new_inode.initialize(DiskInodeType::File);
        self.block_device
            .write_block(new_inode_block_id as usize, &buf.0);

        // 在父目录里追加一个目录项
        self.modify_disk_inode(|root_inode| {
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            self.increase_size(new_size as u32, root_inode, &mut fs);
            let dirent = DirEntry::new(name, new_inode_id);
            root_inode.write_at(file_count * DIRENT_SZ, dirent.as_bytes(), self.block_device.as_ref());
        });

        let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);
        Some(Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        )))
    }

    /// 列出目录内容
    pub fn ls(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SZ;
            let mut v: Vec<String> = Vec::new();
            for i in 0..file_count {
                let mut dirent = DirEntry::empty();
                assert_eq!(
                    disk_inode.read_at(i * DIRENT_SZ, dirent.as_bytes_mut(), self.block_device.as_ref()),
                    DIRENT_SZ
                );
                v.push(String::from(dirent.name()));
            }
            v
        })
    }

    /// 文件大小（ch7：exec 要按文件大小精确分配缓冲）
    pub fn size(&self) -> u32 {
        self.read_disk_inode(|disk_inode| disk_inode.size)
    }

    /// 从 offset 读
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, self.block_device.as_ref()))
    }

    /// 往 offset 写（会自动扩文件）
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| {
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            disk_inode.write_at(offset, buf, self.block_device.as_ref())
        });
        size
    }

    /// 清空文件
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.size;
            let data_blocks_dealloc = disk_inode.clear_size(self.block_device.as_ref());
            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);
            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });
    }
}

// 块大小常量在别处用得到
#[allow(unused)]
const _: usize = BLOCK_SZ;
