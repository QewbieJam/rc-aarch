//! 内核文件系统层（ch6）
//! —— 对照 rCore ch6 的 `os/src/fs/{mod,inode,stdio}.rs` + `drivers/block/*`
//!
//! 三件事：
//!   ① **块设备**：`RamDisk` —— 在页帧池里拿一块连续内存当磁盘（2048 块 × 512B = 1MB）；
//!   ② **挂载**：启动时把这 1MB 格式化成 easy-fs，并把内嵌的 app 写成文件，
//!      然后**重新 open** 一遍（顺便验证超级块/位图写对了）；
//!   ③ **文件对象**：`OSInode` = 磁盘 inode + 当前读写偏移 + 可读/可写标志。
//!
//! ★ 和 upstream 的差别：upstream 用 virtio-blk（QEMU），我们有 Jailhouse cell 里没有
//!   块设备，所以用 RAM 磁盘。easy-fs 本身不关心底层是什么，`BlockDevice` trait 一换就行。
//!   （真要在板子上持久化，得走 ivshmem / virtio，那是 ch7 之后的事。）

use crate::sync::UPSafeCell;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use easy_fs::{BlockDevice, EasyFileSystem, Inode};

/// 磁盘大小（块数），1MB
pub const DISK_BLOCKS: usize = 2048;
const DISK_PAGES: usize = DISK_BLOCKS * easy_fs::BLOCK_SZ / 4096;

/// 页帧池里的 RAM 磁盘
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

/// 根目录（挂载后才有）
static ROOT_INODE: UPSafeCell<Option<Arc<Inode>>> = unsafe { UPSafeCell::new(None) };

/// 取根目录 inode
pub fn root_inode() -> Arc<Inode> {
    ROOT_INODE
        .exclusive_access()
        .as_ref()
        .expect("文件系统还没挂载")
        .clone()
}

/// 初始化文件系统（对照 upstream 的 `ROOT_INODE` 初始化块）
pub fn init() {
    // ① 从页帧池要一块连续的 1MB 当磁盘
    let start_ppn = crate::mm::frame_alloc_contig(DISK_PAGES).expect("RAM 磁盘：页帧不够");
    let start = crate::mm::PhysAddr::from(start_ppn).0;
    // frame_alloc_contig 不清零，这里手动清（格式化反正也会清，但保险）
    unsafe {
        core::ptr::write_bytes(start as *mut u8, 0, DISK_PAGES * 4096);
    }
    let disk = Arc::new(RamDisk {
        start,
        blocks: DISK_BLOCKS,
    });
    crate::println!(
        "[fs] RAM 磁盘: {:#x} 起，{} 块 × {}B = {} KB",
        start,
        DISK_BLOCKS,
        easy_fs::BLOCK_SZ,
        DISK_BLOCKS * easy_fs::BLOCK_SZ / 1024
    );

    // ② 格式化 + 写入文件
    let efs = EasyFileSystem::create(disk.clone(), DISK_BLOCKS as u32, 1);
    let root = EasyFileSystem::root_inode(&efs);

    // 把内嵌的 app ELF 全部写成文件（ch5 的 `_app_names` 表里有名字）
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

    // ③ 重新 open 一遍（验证超级块/位图/目录都真的落到盘上了）
    let efs2 = EasyFileSystem::open(disk.clone());
    let root2 = EasyFileSystem::root_inode(&efs2);
    let mut names: Vec<String> = root2.ls();
    names.sort();
    crate::println!("[fs] 挂载成功，根目录: {:?}", names);

    *ROOT_INODE.exclusive_access() = Some(Arc::new(root2));
}

// ---------------------------------------------------------------------------
// 打开标志
// ---------------------------------------------------------------------------

/// 文件打开标志（对照 upstream 的 `OpenFlags`，用 Linux 的位）
#[derive(Copy, Clone)]
pub struct OpenFlags(pub u32);

impl OpenFlags {
    #[allow(dead_code)]
    pub const RDONLY: Self = Self(0);
    pub const WRONLY: Self = Self(1 << 0);
    #[allow(dead_code)]
    pub const RDWR: Self = Self(1 << 1);
    pub const CREATE: Self = Self(1 << 9);
    pub const TRUNC: Self = Self(1 << 10);

    pub fn from_bits(bits: u32) -> Self {
        Self(bits)
    }
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    /// 返回 (可读, 可写)（对照 upstream 的 `read_write`）
    pub fn read_write(self) -> (bool, bool) {
        if self.0 == 0 {
            (true, false)
        } else if self.contains(Self::WRONLY) {
            (false, true)
        } else {
            (true, true)
        }
    }
}

// ---------------------------------------------------------------------------
// OSInode：内核里的"打开的文件"
// ---------------------------------------------------------------------------

pub struct OSInode {
    readable: bool,
    writable: bool,
    inner: UPSafeCell<OSInodeInner>,
}

pub struct OSInodeInner {
    offset: usize,
    inode: Arc<Inode>,
}

impl OSInode {
    pub fn new(readable: bool, writable: bool, inode: Arc<Inode>) -> Self {
        Self {
            readable,
            writable,
            inner: unsafe { UPSafeCell::new(OSInodeInner { offset: 0, inode }) },
        }
    }

    pub fn readable(&self) -> bool {
        self.readable
    }
    pub fn writable(&self) -> bool {
        self.writable
    }

    /// 把文件读进若干用户缓冲区（跨页），返回总字节数
    pub fn read(&self, buffers: &mut [&mut [u8]]) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total = 0;
        for slice in buffers.iter_mut() {
            let n = inner.inode.read_at(inner.offset, *slice);
            if n == 0 {
                break;
            }
            inner.offset += n;
            total += n;
        }
        total
    }

    /// 把若干用户缓冲区写进文件
    pub fn write(&self, buffers: &[&[u8]]) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total = 0;
        for slice in buffers.iter() {
            let n = inner.inode.write_at(inner.offset, slice);
            assert_eq!(n, slice.len());
            inner.offset += n;
            total += n;
        }
        total
    }

    /// 从当前 offset 读到文件尾，返回 Vec（`exec` 读 ELF 用）
    pub fn read_all(&self) -> Vec<u8> {
        let mut inner = self.inner.exclusive_access();
        let mut buf = [0u8; 512];
        let mut v: Vec<u8> = Vec::new();
        loop {
            let n = inner.inode.read_at(inner.offset, &mut buf);
            if n == 0 {
                break;
            }
            inner.offset += n;
            v.extend_from_slice(&buf[..n]);
        }
        v
    }
}

// ---------------------------------------------------------------------------
// 打开/创建文件
// ---------------------------------------------------------------------------

/// 按名字打开（或创建）根目录下的文件（对照 upstream 的 `open_file`）
pub fn open_file(name: &str, flags: OpenFlags) -> Option<Arc<OSInode>> {
    let (readable, writable) = flags.read_write();
    let root = root_inode();
    if flags.contains(OpenFlags::CREATE) {
        if let Some(inode) = root.find(name) {
            inode.clear();
            Some(Arc::new(OSInode::new(readable, writable, inode)))
        } else {
            root.create(name)
                .map(|inode| Arc::new(OSInode::new(readable, writable, inode)))
        }
    } else {
        root.find(name).map(|inode| {
            if flags.contains(OpenFlags::TRUNC) {
                inode.clear();
            }
            Arc::new(OSInode::new(readable, writable, inode))
        })
    }
}

/// 把整个文件读成 Vec<u8>（`sys_exec` 用）
pub fn read_file_by_name(name: &str) -> Option<Vec<u8>> {
    open_file(name, OpenFlags::RDONLY).map(|f| f.read_all())
}

/// 列出根目录（启动时打印用）
pub fn list_files() -> Vec<String> {
    let mut v = root_inode().ls();
    v.sort();
    v
}
