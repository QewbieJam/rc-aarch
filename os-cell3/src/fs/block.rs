//! 块设备：页帧池里的 RAM 磁盘 + **块缓存（ch9）**
//!
//! ch6/ch7/ch8 里 easy-fs 每次读写都直接落到 RAM 磁盘（`RamDisk`）；
//! ch9 在中间加一层 `CachedDisk`：
//!   - 读：命中缓存就直接拷内存，未命中去盘里读并占一个缓存槽（LRU 淘汰）；
//!   - 写：**写回（write-back）**——只改缓存、标脏，淘汰/`sync_all` 时再落盘；
//!   - 统计命中/未命中/写回次数，停机时打印，用来证明"块缓存真的减少了设备 I/O"。
//!
//! 为什么缓存放在 `BlockDevice` 这一层而不是 easy-fs 里：我们保留了 ch6 的
//! easy-fs 实现不动（upstream ch9 是重写 easy-fs + 内置缓存），在 trait 边界上
//! 包一层能达到同样的效果，改动面小得多。

use crate::sync::UPSafeCell;
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use easy_fs::BlockDevice;

/// 磁盘大小（块数），1MB
pub const DISK_BLOCKS: usize = 4096; // 2MB：ch8 起 app 变多（线程/同步测试），1MB 装不下了
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

// ===========================================================================
// ch9：块缓存
// ===========================================================================

/// 缓存多少个块（32 × 512B = 16KB；RAM 磁盘本来就在内存里，这里主要是
/// 减少"上层反复解析同一个块"和演示缓存效果）
pub const CACHE_BLOCKS: usize = 32;

/// 对照实验开关：`false` = 绕过缓存直接读盘（测 ch9 前后的差异用）
pub const CACHE_ENABLE: bool = true;

struct CacheEntry {
    block_id: usize,
    data: Box<[u8; easy_fs::BLOCK_SZ]>,
    dirty: bool,
}

struct CacheInner {
    entries: Vec<CacheEntry>,
    /// LRU：队头最久没被访问
    lru: VecDeque<usize>,
    hits: usize,
    misses: usize,
    writes: usize,
    evictions: usize,
}

impl CacheInner {
    fn touch(&mut self, pos: usize) {
        self.lru.retain(|&p| p != pos);
        self.lru.push_back(pos);
    }
    /// 找一个空闲槽位（不够就按 LRU 淘汰一个，脏块先写回）
    fn alloc_slot(&mut self, disk: &RamDisk) -> usize {
        if self.entries.len() < CACHE_BLOCKS {
            let pos = self.entries.len();
            self.entries.push(CacheEntry {
                block_id: usize::MAX,
                data: Box::new([0u8; easy_fs::BLOCK_SZ]),
                dirty: false,
            });
            self.lru.push_back(pos);
            return pos;
        }
        let pos = self.lru.pop_front().unwrap();
        if self.entries[pos].dirty {
            disk.write_block(self.entries[pos].block_id, &self.entries[pos].data[..]);
            self.evictions += 1;
            self.entries[pos].dirty = false;
        }
        self.lru.push_back(pos);
        pos
    }
}

/// 带块缓存的块设备（写回策略）
pub struct CachedDisk {
    disk: Arc<RamDisk>,
    cache: UPSafeCell<CacheInner>,
}

impl CachedDisk {
    pub fn new(disk: Arc<RamDisk>) -> Arc<Self> {
        Arc::new(Self {
            disk,
            cache: unsafe {
                UPSafeCell::new(CacheInner {
                    entries: Vec::new(),
                    lru: VecDeque::new(),
                    hits: 0,
                    misses: 0,
                    writes: 0,
                    evictions: 0,
                })
            },
        })
    }

    /// 把所有脏块写回磁盘（停机和 exec 之前调用）
    pub fn sync_all(&self) {
        let mut cache = self.cache.exclusive_access();
        let mut n = 0;
        for entry in cache.entries.iter_mut() {
            if entry.dirty {
                self.disk.write_block(entry.block_id, &entry.data[..]);
                entry.dirty = false;
                n += 1;
            }
        }
        cache.evictions += n;
    }

    pub fn stats(&self) -> (usize, usize, usize, usize) {
        let cache = self.cache.exclusive_access();
        (cache.hits, cache.misses, cache.writes, cache.evictions)
    }
}

impl BlockDevice for CachedDisk {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        if !CACHE_ENABLE {
            self.disk.read_block(block_id, buf);
            return;
        }
        let mut cache = self.cache.exclusive_access();
        if let Some(pos) = cache.entries.iter().position(|e| e.block_id == block_id) {
            buf.copy_from_slice(&cache.entries[pos].data[..]);
            cache.hits += 1;
            cache.touch(pos);
            return;
        }
        cache.misses += 1;
        let pos = cache.alloc_slot(&self.disk);
        cache.entries[pos].block_id = block_id;
        self.disk.read_block(block_id, &mut cache.entries[pos].data[..]);
        buf.copy_from_slice(&cache.entries[pos].data[..]);
        cache.touch(pos);
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) {
        if !CACHE_ENABLE {
            self.disk.write_block(block_id, buf);
            return;
        }
        let mut cache = self.cache.exclusive_access();
        cache.writes += 1;
        // 已经缓存：只改缓存并标脏（写回）
        if let Some(pos) = cache.entries.iter().position(|e| e.block_id == block_id) {
            cache.entries[pos].data.copy_from_slice(buf);
            cache.entries[pos].dirty = true;
            cache.touch(pos);
            return;
        }
        // 不在缓存：直接写穿（省一次"读旧块"）
        self.disk.write_block(block_id, buf);
    }
}

/// 全局唯一的 CachedDisk（拿统计、统一 sync 用）
static CACHED_DISK: UPSafeCell<Option<Arc<CachedDisk>>> = unsafe { UPSafeCell::new(None) };

/// 块缓存统计：(命中, 未命中, 写, 淘汰写回)
pub fn cache_stats() -> (usize, usize, usize, usize) {
    match CACHED_DISK.exclusive_access().as_ref() {
        Some(d) => d.stats(),
        None => (0, 0, 0, 0),
    }
}

/// 把脏块全部写回
pub fn cache_sync_all() {
    if let Some(d) = CACHED_DISK.exclusive_access().as_ref() {
        d.sync_all();
    }
}

/// 从页帧池要一块连续的 1MB，做成 RAM 磁盘（外面包一层块缓存）
pub fn make_disk() -> Arc<CachedDisk> {
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
    let ram = Arc::new(RamDisk {
        start,
        blocks: DISK_BLOCKS,
    });
    let disk = CachedDisk::new(ram);
    *CACHED_DISK.exclusive_access() = Some(disk.clone());
    disk
}
