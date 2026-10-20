//! 任务控制块（ch4）—— 对照 rCore ch4 的 `os/src/task/task.rs`
//!
//! ch4 的 TCB 比 ch3 多了两样东西：
//!   ① `memory_set`：每个任务**一套自己的页表**（ch3 是所有任务共用一套物理地址）
//!   ② `heap_bottom` / `program_brk`：给 `sys_sbrk`（214）用的堆边界
//!
//! 和 upstream 的差别：
//!   - upstream 还有 `trap_cx_ppn`（TrapContext 放在用户空间的 TRAP_CONTEXT 页里，
//!     因为它的陷阱入口在 trampoline 上用固定 VA 找 TrapContext）。
//!     我们的 TrapContext 就在**内核栈顶**（恒等映射，随时可访问），
//!     所以不需要那个字段，也不需要 `get_trap_cx()`。
//!   - upstream 在 `new()` 里还往 KERNEL_SPACE 里插一段内核栈；我们用 .bss 静态数组。

use super::TaskContext;
use crate::loader::kernel_stack;
use crate::mm::{MemorySet, VirtAddr};
use crate::trap::TrapContext;

/// 任务控制块
pub struct TaskControlBlock {
    pub task_status: TaskStatus,
    /// 内核态切换上下文（ra/sp/callee-saved）
    pub task_cx: TaskContext,
    /// 这个任务独占的地址空间（页表 + 映射区）
    pub memory_set: MemorySet,
    /// TTBR0 里的 ASID（给每个任务一个非 0 值，方便以后做 TLB 优化）
    pub asid: u16,
    /// 堆起点（= 用户栈顶，对照 upstream 的 `heap_bottom`）
    pub heap_bottom: usize,
    /// 当前 program break（对照 upstream 的 `program_brk`）
    pub program_brk: usize,
}

impl TaskControlBlock {
    /// 从一个 ELF 建任务（对照 upstream 的 `TaskControlBlock::new`）
    pub fn new(elf_data: &[u8], app_id: usize) -> Self {
        // ① 建地址空间：ELF 各段 + 用户栈（物理页由 frame allocator 给）
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        // ② 在本任务的内核栈顶压一份初始 TrapContext，并让 task_cx 指向它
        let trap_cx_addr =
            kernel_stack(app_id).push_context(TrapContext::app_init_context(entry_point, user_sp));

        Self {
            task_status: TaskStatus::Ready,
            task_cx: TaskContext::goto_trap_return(trap_cx_addr),
            memory_set,
            asid: app_id as u16 + 1,
            heap_bottom: user_sp,
            program_brk: user_sp,
        }
    }

    pub fn token(&self) -> usize {
        self.memory_set.token()
    }

    /// 改 program break（对照 upstream 的 `change_program_brk`）
    ///
    /// 返回旧的 brk；失败（越界/找不到区域）返回 None。
    pub fn change_program_brk(&mut self, size: i32) -> Option<usize> {
        let old_break = self.program_brk;
        let new_brk = self.program_brk as isize + size as isize;
        if new_brk < self.heap_bottom as isize {
            return None;
        }
        let result = if size < 0 {
            self.memory_set
                .shrink_to(VirtAddr(self.heap_bottom), VirtAddr(new_brk as usize))
        } else {
            self.memory_set
                .append_to(VirtAddr(self.heap_bottom), VirtAddr(new_brk as usize))
        };
        if result {
            self.program_brk = new_brk as usize;
            // 页表变了，当前任务的 TLB 得清一下（我们没用 ASID 精细失效，直接全清）
            self.memory_set.activate(self.asid);
            Some(old_break)
        } else {
            None
        }
    }
}

/// 任务状态（ch4 去掉了 `UnInit`：任务是在 Vec 里现建的，没有"未初始化"阶段）
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    Ready,
    Running,
    Exited,
}
