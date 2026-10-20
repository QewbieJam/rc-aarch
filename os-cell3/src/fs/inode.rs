//! 打开的文件对象（ch6）—— 对照 rCore ch6 的 `os/src/fs/inode.rs`
//!
//! `OSInode` = 磁盘 inode + 当前读写偏移 + 可读/可写标志。
//! ch7 起它实现了 `File` trait，和标准输入输出、管道走同一套 fd 分发。

use super::File;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use easy_fs::Inode;

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

/// 设置根目录（mount 时调用）
pub fn set_root(inode: Arc<Inode>) {
    *ROOT_INODE.exclusive_access() = Some(inode);
}

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
    /// 返回 (可读, 可写)
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

    /// 从当前 offset 读到文件尾，返回 Vec（`exec` 读 ELF 用；ch7 起 exec 改用 FrameBuf）
    #[allow(dead_code)]
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

impl File for OSInode {
    fn readable(&self) -> bool {
        self.readable
    }
    fn writable(&self) -> bool {
        self.writable
    }
    fn read(&self, mut buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total = 0;
        for slice in buf.buffers.iter_mut() {
            let n = inner.inode.read_at(inner.offset, *slice);
            if n == 0 {
                break;
            }
            inner.offset += n;
            total += n;
        }
        total
    }
    fn write(&self, buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total = 0;
        for slice in buf.buffers.iter() {
            let n = inner.inode.write_at(inner.offset, *slice);
            assert_eq!(n, slice.len());
            inner.offset += n;
            total += n;
        }
        total
    }
}

/// 按名字打开（或创建）根目录下的文件（对照 upstream 的 `open_file`）
pub fn open_file(name: &str, flags: OpenFlags) -> Option<Arc<dyn File + Send + Sync>> {
    let (readable, writable) = flags.read_write();
    let root = root_inode();
    type FileArc = Arc<dyn File + Send + Sync>;
    if flags.contains(OpenFlags::CREATE) {
        if let Some(inode) = root.find(name) {
            inode.clear();
            Some(Arc::new(OSInode::new(readable, writable, inode)) as FileArc)
        } else {
            root.create(name)
                .map(|inode| Arc::new(OSInode::new(readable, writable, inode)) as FileArc)
        }
    } else {
        root.find(name).map(|inode| {
            if flags.contains(OpenFlags::TRUNC) {
                inode.clear();
            }
            Arc::new(OSInode::new(readable, writable, inode)) as FileArc
        })
    }
}

/// 把整个文件读成 Vec<u8>（ch6 的 `sys_exec` 用；ch7 起改用 `read_file_to`）
#[allow(dead_code)]
pub fn read_file_by_name(name: &str) -> Option<Vec<u8>> {
    let inode = root_inode().find(name)?;
    Some(OSInode::new(true, false, inode).read_all())
}

/// 根目录下某个文件的大小（exec 用）
pub fn file_size(name: &str) -> Option<usize> {
    root_inode().find(name).map(|inode| inode.size() as usize)
}

/// 把文件读进给定缓冲区（exec 用，避免 Vec 扩容泄漏）
pub fn read_file_to(name: &str, dst: &mut [u8]) -> Option<usize> {
    let inode = root_inode().find(name)?;
    let mut off = 0;
    while off < dst.len() {
        let n = inode.read_at(off, &mut dst[off..]);
        if n == 0 {
            break;
        }
        off += n;
    }
    Some(off)
}

/// 列出根目录（启动时打印用）
pub fn list_files() -> Vec<String> {
    let mut v = root_inode().ls();
    v.sort();
    v
}
