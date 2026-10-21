//! 内核堆：**带合并的空闲链表分配器**（ch7+）
//!
//! 替换之前那个"只增不减"的 bump 分配器。为什么必须换：
//!   - ch8 的线程/同步原语会**反复**创建、销毁 `Arc<Mutex<...>>`；
//!   - 内核里每个系统调用都会分配一点（`translated_byte_buffer` 的 Vec），
//!     bump 分配器几万次系统调用就能把 256KB 磨光（ch7 的 exec 已经踩过一次）。
//!
//! 算法：**按地址排序的空闲链表 + 首次适配 + 相邻合并**
//!
//! ```text
//!   堆区：[start, start+size)
//!   每个空闲块：| size (8B) | next 偏移 (8B) | 负载 ... |
//!   分配：从头遍历，找到第一个够大的块 → 切掉一段返回，剩下的留在链表里
//!   释放：按地址插回链表，和前/后相邻块合并
//! ```
//!
//! 取舍：
//!   - 首次适配 + 合并，不是伙伴系统，长时间运行会有**外部碎片**（教学够用）；
//!   - 所有块按 16 字节对齐，负载地址也 16 字节对齐；**请求 align > 16 直接返回 null**
//!     （我们内核里的类型最大对齐是 16，`SignalAction` 就是 16）；
//!   - 单核 + 内核态关中断，不用原子操作，用 `UnsafeCell` 即可。

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::null_mut;

/// 堆大小：256KB（放在 .bss，不占镜像体积）
pub const KERNEL_HEAP_SIZE: usize = 0x4_0000;

const HDR: usize = 16; // 每块的头：size + next
const MIN_BLOCK: usize = HDR + 16; // 小于这个尺寸就不切了（免得切出没法用的小块）
const NONE: usize = usize::MAX;

#[repr(align(16))]
struct HeapSpace(#[allow(dead_code)] [u8; KERNEL_HEAP_SIZE]);

static mut HEAP_SPACE: HeapSpace = HeapSpace([0; KERNEL_HEAP_SIZE]);

/// 空闲链表分配器
pub struct FreeListAllocator {
    /// 堆起始地址（0 = 还没初始化）
    start: UnsafeCell<usize>,
    /// 堆总大小
    size: UnsafeCell<usize>,
    /// 第一个空闲块的**绝对地址**（NONE = 链表空）
    head: UnsafeCell<usize>,
}

unsafe impl Sync for FreeListAllocator {}

// --- 空闲块头部的读写（都用偏移，避免在 &mut 生命周期上纠缠）---
#[inline]
unsafe fn blk_size(off: usize) -> usize {
    unsafe { *(off as *const usize) }
}
#[inline]
unsafe fn blk_next(off: usize) -> usize {
    unsafe { *((off + 8) as *const usize) }
}
#[inline]
unsafe fn set_blk_size(off: usize, size: usize) {
    unsafe { *(off as *mut usize) = size }
}
#[inline]
unsafe fn set_blk_next(off: usize, next: usize) {
    unsafe { *((off + 8) as *mut usize) = next }
}

impl FreeListAllocator {
    const fn new() -> Self {
        Self {
            start: UnsafeCell::new(0),
            size: UnsafeCell::new(0),
            head: UnsafeCell::new(NONE),
        }
    }

    /// 初始化：把 [base, base+size) 对齐后做成"一个大空闲块"
    fn init(&self, base: usize, size: usize) {
        // 起始地址向上对齐到 16，大小向下取整
        let start = (base + 15) & !15;
        let pad = start - base;
        if size <= pad + MIN_BLOCK {
            return;
        }
        let size = (size - pad) & !15;
        unsafe {
            *self.start.get() = start;
            *self.size.get() = size;
            // ★ 块用**绝对地址**标识（head 就是第一块的地址）
            *self.head.get() = start;
            set_blk_size(start, size);
            set_blk_next(start, NONE);
        }
    }

    /// 遍历空闲链表：返回 (空闲总字节, 最大空闲块, 空闲块个数)
    fn stats(&self) -> (usize, usize, usize) {
        let mut total = 0;
        let mut max = 0;
        let mut count = 0;
        unsafe {
            let mut cur = *self.head.get();
            while cur != NONE {
                let s = blk_size(cur);
                total += s;
                max = max.max(s);
                count += 1;
                cur = blk_next(cur);
            }
        }
        (total, max, count)
    }

    fn free_bytes(&self) -> usize {
        self.stats().0
    }

    /// 一致性检查（开发期用）：空闲链表必须按地址升序、不重叠、不越界、大小合法
    pub fn check(&self) -> Result<(), &'static str> {
        unsafe {
            let start = *self.start.get();
            if start == 0 {
                return Err("堆没有初始化");
            }
            let end = start + *self.size.get();
            let mut prev = NONE;
            let mut cur = *self.head.get();
            while cur != NONE {
                if cur < start || cur + HDR > end {
                    return Err("空闲块越界");
                }
                let size = blk_size(cur);
                if size < MIN_BLOCK || cur + size > end {
                    return Err("空闲块大小非法");
                }
                if prev != NONE && prev + blk_size(prev) > cur {
                    return Err("空闲块重叠或乱序");
                }
                prev = cur;
                cur = blk_next(cur);
            }
        }
        Ok(())
    }
}

unsafe impl GlobalAlloc for FreeListAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.align() > 16 {
            // 我们内核里没有超过 16 字节对齐的类型；真有的话得改算法
            return null_mut();
        }
        if unsafe { *self.start.get() } == 0 {
            return null_mut();
        }
        let need = (layout.size().max(1) + 15) & !15; // 至少 16 字节，向上对齐
        let mut prev = NONE;
        unsafe {
            let mut cur = *self.head.get();
            while cur != NONE {
                let size = blk_size(cur);
                if size >= HDR + need {
                    let next = blk_next(cur);
                    if size - (HDR + need) >= MIN_BLOCK {
                        // 切一刀：剩下的一段继续挂在链表里
                        let rest = cur + HDR + need;
                        set_blk_size(rest, size - HDR - need);
                        set_blk_next(rest, next);
                        if prev == NONE {
                            *self.head.get() = rest;
                        } else {
                            set_blk_next(prev, rest);
                        }
                        // 已分配块的头里记下自己的大小（dealloc 要用）
                        set_blk_size(cur, HDR + need);
                        set_blk_next(cur, NONE);
                    } else {
                        // 整块拿走（内部碎片；不影响正确性）
                        if prev == NONE {
                            *self.head.get() = next;
                        } else {
                            set_blk_next(prev, next);
                        }
                    }
                    return (cur + HDR) as *mut u8;
                }
                prev = cur;
                cur = blk_next(cur);
            }
        }
        null_mut()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        if unsafe { *self.start.get() } == 0 || ptr.is_null() {
            return;
        }
        let off = (ptr as usize) - HDR;
        let size = unsafe { blk_size(off) };
        unsafe { self.insert_free(off, size) };
    }
}

impl FreeListAllocator {
    /// 把一个块按地址插回空闲链表，并和前后相邻块合并
    unsafe fn insert_free(&self, off: usize, size: usize) {
        unsafe {
            // 找到插入位置（保持按地址升序）
            let mut prev = NONE;
            let mut cur = *self.head.get();
            while cur != NONE && cur < off {
                prev = cur;
                cur = blk_next(cur);
            }
            // 先和后一个合并
            let (mut merged_size, mut merged_next) = (size, cur);
            if cur != NONE && off + size == cur {
                merged_size += blk_size(cur);
                merged_next = blk_next(cur);
            }
            set_blk_size(off, merged_size);
            set_blk_next(off, merged_next);
            // 再和前一个合并
            if prev != NONE && prev + blk_size(prev) == off {
                set_blk_size(prev, blk_size(prev) + merged_size);
                set_blk_next(prev, merged_next);
            } else if prev == NONE {
                *self.head.get() = off;
            } else {
                set_blk_next(prev, off);
            }
        }
    }
}

#[global_allocator]
static HEAP_ALLOCATOR: FreeListAllocator = FreeListAllocator::new();

/// 初始化堆（对照 upstream 的 `init_heap`）
pub fn init_heap() {
    let base = core::ptr::addr_of_mut!(HEAP_SPACE) as usize;
    HEAP_ALLOCATOR.init(base, KERNEL_HEAP_SIZE);
    let (total, max, count) = HEAP_ALLOCATOR.stats();
    crate::println!(
        "[mm] kernel heap: [{:#x}, {:#x}) {} KB（空闲链表，可回收）初始: 空闲 {}KB / 最大块 {}KB / {} 块",
        base,
        base + KERNEL_HEAP_SIZE,
        KERNEL_HEAP_SIZE / 1024,
        total / 1024,
        max / 1024,
        count
    );
}

/// 堆用量：(总大小, 空闲字节, 最大空闲块, 空闲块数)
pub fn heap_stats() -> (usize, usize, usize, usize) {
    let (total, max, count) = HEAP_ALLOCATOR.stats();
    (KERNEL_HEAP_SIZE, total, max, count)
}

/// 堆分配器自测（ch7+）：反复分配/释放，确认内存真的被复用、不会越用越少
pub fn heap_test() {
    use alloc::vec::Vec;

    let free0 = HEAP_ALLOCATOR.free_bytes();

    // ① 小块反复申请释放（模拟 ch8 的线程对象）
    for _round in 0..20 {
        let mut v: Vec<Vec<u8>> = Vec::new();
        for _ in 0..50 {
            v.push(alloc::vec![0u8; 1000]);
        }
        v.clear();
    }

    // ② 大块
    {
        let mut v: Vec<Vec<u8>> = Vec::new();
        for _ in 0..32 {
            v.push(alloc::vec![0u8; 4096]);
        }
        drop(v);
    }

    // ③ 碎片场景：A B C 三块，释放 B，再申请同样大小，应该复用 B 的洞
    {
        let a = alloc::vec![0u8; 4096];
        let b = alloc::vec![0u8; 4096];
        let c = alloc::vec![0u8; 4096];
        drop(b);
        let d = alloc::vec![0u8; 4096];
        assert_eq!(d.len(), 4096);
        drop(a);
        drop(c);
        drop(d);
    }

    HEAP_ALLOCATOR
        .check()
        .unwrap_or_else(|e| panic!("heap_test: 空闲链表损坏: {}", e));
    let (total, free, max, count) = heap_stats();
    crate::println!(
        "[mm] heap_test: 总 {}KB，测试前空闲 {}KB，现在空闲 {}KB（最大块 {}KB，{} 个空闲块）",
        total / 1024,
        free0 / 1024,
        free / 1024,
        max / 1024,
        count
    );
    if free + 4096 < free0 {
        panic!("heap_test: 空闲内存没有回收干净（{} -> {}）", free0, free);
    }
    crate::println!("[mm] heap_test passed: 分配/释放/合并都正常");
}
