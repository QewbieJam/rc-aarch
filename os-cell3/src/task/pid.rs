//! pid 分配器 + 内核栈（ch5）—— 对照 rCore ch5 的 `os/src/task/pid.rs`
//!
//! 两件事：
//!   ① `PidHandle`：pid 的所有权凭证，Drop 时自动回收 pid（对照 upstream 完全一致）；
//!   ② `KernelStack`：每个进程一块内核栈，Drop 时自动还回页帧。
//!
//! ★ 和 upstream 的差别（AArch64 侧）：
//!   upstream 用 `kernel_stack_position(pid)` 在**内核地址空间的高地址**里给每个 pid
//!   留一个固定 VA 槽位（TRAMPOLINE - pid*8KB），靠页表映射过去。因为它的陷阱入口
//!   在 trampoline 上，内核栈必须是固定 VA。
//!   我们把内核栈直接放在**页帧池**里（恒等映射，物理地址就是 VA），所以：
//!     - 不需要固定 VA 槽位；
//!     - 需要**物理连续**的两页 → `frame_alloc_contig(2)`；
//!     - 栈顶 = 起始物理地址 + 8KB。
//!   这样内核栈也是**可回收**的（ch4 结尾列过这个 TODO）。

use super::super::config::{KERNEL_STACK_SIZE, PAGE_SIZE};
use crate::mm::{PhysAddr, PhysPageNum, frame_alloc_contig, frame_dealloc_range};
use crate::sync::UPSafeCell;
use alloc::vec::Vec;

/// pid 分配器（对照 upstream 的 `PidAllocator`）
pub struct PidAllocator {
    current: usize,
    recycled: Vec<usize>,
}

impl PidAllocator {
    const fn new() -> Self {
        Self {
            current: 1,
            recycled: Vec::new(),
        }
    }
    fn alloc(&mut self) -> PidHandle {
        match self.recycled.pop() {
            Some(pid) => PidHandle(pid),
            None => {
                self.current += 1;
                PidHandle(self.current - 1)
            }
        }
    }
    fn dealloc(&mut self, pid: usize) {
        assert!(pid < self.current);
        assert!(
            !self.recycled.iter().any(|p| *p == pid),
            "pid {} 被重复回收！",
            pid
        );
        self.recycled.push(pid);
    }
}

static PID_ALLOCATOR: UPSafeCell<PidAllocator> = unsafe { UPSafeCell::new(PidAllocator::new()) };

/// pid 的所有权凭证：谁持有它，pid 就不会被复用
pub struct PidHandle(pub usize);

impl Drop for PidHandle {
    fn drop(&mut self) {
        PID_ALLOCATOR.exclusive_access().dealloc(self.0);
    }
}

/// 分配一个 pid
pub fn pid_alloc() -> PidHandle {
    PID_ALLOCATOR.exclusive_access().alloc()
}

const KSTACK_PAGES: usize = KERNEL_STACK_SIZE / PAGE_SIZE;

/// 一个进程的内核栈（物理连续的 KSTACK_PAGES 页）
pub struct KernelStack {
    start_ppn: PhysPageNum,
}

impl KernelStack {
    /// 申请一块内核栈并清零
    pub fn new() -> Self {
        let start_ppn = frame_alloc_contig(KSTACK_PAGES).expect("内核栈页帧耗尽");
        // 连续页的"清零"不能靠 FrameTracker::new（那是单页），自己来
        for i in 0..KSTACK_PAGES {
            let page = PhysPageNum(start_ppn.0 + i);
            for b in page.get_bytes_array() {
                *b = 0;
            }
        }
        Self { start_ppn }
    }

    /// 栈顶地址（恒等映射：物理地址就是内核 VA）
    pub fn get_top(&self) -> usize {
        PhysAddr::from(self.start_ppn).0 + KERNEL_STACK_SIZE
    }

    /// 往栈顶压一个值，返回它的地址
    pub fn push_on_top<T>(&self, value: T) -> usize {
        let ptr = (self.get_top() - core::mem::size_of::<T>()) as *mut T;
        unsafe {
            ptr.write_volatile(value);
        }
        ptr as usize
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        frame_dealloc_range(self.start_ppn, KSTACK_PAGES);
    }
}
