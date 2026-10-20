//! 物理页帧分配器（ch4）—— 对照 rCore ch4 的 `os/src/mm/frame_allocator.rs`
//!
//! 语义完全一样：内核启动时把 `[ekernel, MEMORY_END)` 这段物理内存按 4KB 切成一堆
//! 页帧（frame），用"栈式分配器 + 回收栈"管理；`FrameTracker` 的生命周期 = 页帧的
//! 生命周期（Drop 时自动归还）。
//!
//! 和 upstream 的差别只有一处：`lazy_static!` 换成了 `const fn` 静态构造
//! （我们从 ch2 起就不引 `lazy_static`）。

use super::{PhysAddr, PhysPageNum};
use crate::config::MEMORY_END;
use crate::sync::UPSafeCell;
use alloc::vec::Vec;
use core::fmt::{self, Debug, Formatter};

/// 一个页帧的"所有权凭证"：谁持有它，谁就独占这一页；Drop 时自动归还。
pub struct FrameTracker {
    pub ppn: PhysPageNum,
}

impl FrameTracker {
    /// 拿到的页帧先清零（对照 upstream 的 `FrameTracker::new`）
    pub fn new(ppn: PhysPageNum) -> Self {
        let bytes_array = ppn.get_bytes_array();
        for i in bytes_array {
            *i = 0;
        }
        Self { ppn }
    }
}

impl Debug for FrameTracker {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "FrameTracker:PPN={:#x}", self.ppn.0)
    }
}

impl Drop for FrameTracker {
    fn drop(&mut self) {
        frame_dealloc(self.ppn);
    }
}

trait FrameAllocator {
    fn alloc(&mut self) -> Option<PhysPageNum>;
    fn dealloc(&mut self, ppn: PhysPageNum);
}

/// 栈式页帧分配器（对照 upstream 的 `StackFrameAllocator`）
///
/// - 没回收过：`current` 一路往上加（从物理低地址往高地址分配）
/// - 有回收：优先从 `recycled` 里拿（LIFO）
pub struct StackFrameAllocator {
    current: usize,
    end: usize,
    recycled: Vec<usize>,
}

impl StackFrameAllocator {
    const fn new() -> Self {
        Self {
            current: 0,
            end: 0,
            recycled: Vec::new(),
        }
    }
    fn init(&mut self, l: PhysPageNum, r: PhysPageNum) {
        self.current = l.0;
        self.end = r.0;
    }
}

impl FrameAllocator for StackFrameAllocator {
    fn alloc(&mut self) -> Option<PhysPageNum> {
        if let Some(ppn) = self.recycled.pop() {
            Some(ppn.into())
        } else if self.current == self.end {
            None
        } else {
            self.current += 1;
            Some((self.current - 1).into())
        }
    }
    fn dealloc(&mut self, ppn: PhysPageNum) {
        let ppn = ppn.0;
        // 合法性检查：不能让没分配过的页被"归还"（否则以后会重复分配）
        if ppn >= self.current || self.recycled.iter().any(|&v| v == ppn) {
            panic!("Frame ppn={:#x} 没有被分配过！", ppn);
        }
        self.recycled.push(ppn);
    }
}

/// 全局唯一的分配器实例
static FRAME_ALLOCATOR: UPSafeCell<StackFrameAllocator> =
    unsafe { UPSafeCell::new(StackFrameAllocator::new()) };

/// 初始化：把 `[ekernel, MEMORY_END)` 交给分配器（对照 upstream 的 `init_frame_allocator`）
///
/// ★ `ekernel` 是链接脚本里的 `__kernel_end`：内核镜像（含 .bss 里的栈/堆）之后
///   的物理地址。**绝对不能**把内核自己占的页也发出去。
pub fn init_frame_allocator() {
    unsafe extern "C" {
        safe fn __kernel_end();
    }
    let start = PhysAddr::from(__kernel_end as *const () as usize).ceil();
    let end = PhysAddr::from(MEMORY_END).floor();
    crate::println!(
        "[mm] frame allocator: [{:#x}, {:#x}) 共 {} 页（{} MB）",
        start.0 << 12,
        end.0 << 12,
        end.0 - start.0,
        (end.0 - start.0) * 4 / 1024
    );
    FRAME_ALLOCATOR.exclusive_access().init(start, end);
}

/// 申请一个页帧
pub fn frame_alloc() -> Option<FrameTracker> {
    FRAME_ALLOCATOR
        .exclusive_access()
        .alloc()
        .map(FrameTracker::new)
}

/// 归还一个页帧（只由 `FrameTracker::drop` 调用）
fn frame_dealloc(ppn: PhysPageNum) {
    FRAME_ALLOCATOR.exclusive_access().dealloc(ppn);
}

/// 页帧分配器自测（对照 upstream 的 `frame_allocator_test`）
#[allow(unused)]
pub fn frame_allocator_test() {
    let mut v: Vec<FrameTracker> = Vec::new();
    for _ in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:?}", frame);
        v.push(frame);
    }
    v.clear();
    for _ in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:?}", frame);
        v.push(frame);
    }
    drop(v);
    println!("frame_allocator_test passed!");
}
