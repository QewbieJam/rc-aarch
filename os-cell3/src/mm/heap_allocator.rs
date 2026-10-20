//! 内核堆分配器（ch4）
//!
//! upstream 用的是 `buddy_system_allocator`，我们**不引外部 crate**，写一个
//! 够用的 bump 分配器：从一块静态数组里一路往上分，`dealloc` 是空操作。
//!
//! 为什么这样就够：ch4 的堆只用来放 `Vec`（页表帧列表、MapArea 列表、任务表），
//! 数量少、生命周期长（几乎都是"活到关机"）。真正需要反复分配/释放的页帧走
//! `frame_allocator`（有回收栈），不经过这里。
//!
//! ⚠️ 以后如果开始频繁创建/销毁任务（ch5 fork），记得把它换成真正的分配器
//! （buddy 或 slab），否则 256KB 迟早漏光。

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::null_mut;

/// 堆大小：256KB（放在 .bss，不占镜像体积）
pub const KERNEL_HEAP_SIZE: usize = 0x4_0000;

static mut HEAP_SPACE: [u8; KERNEL_HEAP_SIZE] = [0; KERNEL_HEAP_SIZE];

/// bump 分配器
pub struct BumpAllocator {
    base: UnsafeCell<usize>,
    used: UnsafeCell<usize>,
    size: usize,
}

// 单核 + 只会在"没有并发"的上下文里分配，所以声明成 Sync
unsafe impl Sync for BumpAllocator {}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let used = unsafe { &mut *self.used.get() };
        let base = unsafe { *self.base.get() };
        if base == 0 {
            return null_mut();
        }
        // 对齐（layout.align() 一定是 2 的幂）
        let start = (*used + layout.align() - 1) & !(layout.align() - 1);
        if start + layout.size() > self.size {
            return null_mut();
        }
        *used = start + layout.size();
        (base + start) as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // bump 分配器不回收
    }
}

#[global_allocator]
static HEAP_ALLOCATOR: BumpAllocator = BumpAllocator {
    base: UnsafeCell::new(0),
    used: UnsafeCell::new(0),
    size: KERNEL_HEAP_SIZE,
};

/// 初始化堆（对照 upstream 的 `init_heap`）
pub fn init_heap() {
    let base = core::ptr::addr_of_mut!(HEAP_SPACE) as usize;
    unsafe {
        *HEAP_ALLOCATOR.base.get() = base;
    }
    crate::println!(
        "[mm] kernel heap: [{:#x}, {:#x}) {} KB（bump，不回收）",
        base,
        base + KERNEL_HEAP_SIZE,
        KERNEL_HEAP_SIZE / 1024
    );
}

/// 堆自测（对照 upstream 的 `heap_test`）
#[allow(unused)]
pub fn heap_test() {
    use alloc::boxed::Box;
    use alloc::vec::Vec;
    let a = Box::new(5);
    assert_eq!(*a, 5);
    drop(a);
    let mut v: Vec<usize> = Vec::new();
    for i in 0..500 {
        v.push(i);
    }
    for (i, val) in v.iter().take(500).enumerate() {
        assert_eq!(*val, i);
    }
    drop(v);
    println!("heap_test passed!");
}
