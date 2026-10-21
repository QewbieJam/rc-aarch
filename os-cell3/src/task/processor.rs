//! 处理器 / 调度主循环（ch5）—— 对照 rCore ch5 的 `os/src/task/processor.rs`
//!
//! ch5 的调度模型和 ch4 不一样了：
//!
//! ```text
//! ch4：run_first_task → 任务里一直跑；时间片到了就 __switch 到"下一个任务"
//! ch5：run_tasks（空闲控制流）循环 fetch_task：
//!         有任务 → __switch(idle, task)
//!         任务里 yield/被抢占/退出 → __switch(task, idle) 回到循环里继续 fetch
//! ```
//!
//! `idle_task_cx` 就是"调度器自己的上下文"，它保存在 PROCESSOR 静态里
//! （在启动栈上跑），所以任务切回 idle 时一定会回到 `run_tasks` 的循环里。
//!
//! ★ 调度器跑在 EL1，`PSTATE.I=1`（陷阱进来时硬件置的），所以 idle 循环里不会被
//!   定时器打断 —— 也不需要：这时候没有 EL0 任务在跑，没有"抢占"可言。

use super::manager::fetch_task;
use super::switch::__switch;
use super::task::{TaskControlBlock, TaskStatus};
use super::TaskContext;
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::sync::Arc;

/// 当前 CPU 的"正在运行"状态
pub struct Processor {
    current: Option<Arc<TaskControlBlock>>,
    /// 调度器的上下文（从任务切回来时回到 run_tasks 的循环）
    idle_task_cx: TaskContext,
}

impl Processor {
    const fn new() -> Self {
        Self {
            current: None,
            idle_task_cx: TaskContext::zero_init(),
        }
    }
    fn get_idle_task_cx_ptr(&mut self) -> *mut TaskContext {
        &mut self.idle_task_cx as *mut _
    }
    fn take_current(&mut self) -> Option<Arc<TaskControlBlock>> {
        self.current.take()
    }
    fn current(&self) -> Option<Arc<TaskControlBlock>> {
        self.current.as_ref().map(Arc::clone)
    }
}

static PROCESSOR: UPSafeCell<Processor> = unsafe { UPSafeCell::new(Processor::new()) };

/// 调度主循环（对照 upstream 的 `run_tasks`）
pub fn run_tasks() -> ! {
    loop {
        let mut processor = PROCESSOR.exclusive_access();
        if let Some(task) = fetch_task() {
            let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
            let asid = task.asid;
            // 访问即将运行的任务
            let mut task_inner = task.inner_exclusive_access();
            let next_task_cx_ptr = &task_inner.task_cx as *const TaskContext;
            task_inner.task_status = TaskStatus::Running;
            // ★ 换地址空间（TTBR0）—— 就在切过去之前做
            task_inner.memory_set.activate(asid);
            drop(task_inner);
            processor.current = Some(task);
            drop(processor);
            unsafe {
                __switch(idle_task_cx_ptr, next_task_cx_ptr);
            }
        }
    }
}

/// 把当前任务取走（Processor.current 变 None）
pub fn take_current_task() -> Option<Arc<TaskControlBlock>> {
    PROCESSOR.exclusive_access().take_current()
}

/// 当前任务（克隆一个 Arc）
pub fn current_task() -> Option<Arc<TaskControlBlock>> {
    PROCESSOR.exclusive_access().current()
}

/// 当前进程 pid（调试用）
pub fn current_task_id() -> usize {
    current_task().map(|t| t.getpid()).unwrap_or(0)
}

/// 当前任务的页表 token
pub fn current_user_token() -> usize {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .get_user_token()
}

/// 当前任务的 TrapContext（在内核栈顶）
pub fn current_trap_cx() -> &'static mut TrapContext {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .get_trap_cx()
}

/// 切回调度器（对照 upstream 的 `schedule`）
pub fn schedule(switched_task_cx_ptr: *mut TaskContext) {
    let mut processor = PROCESSOR.exclusive_access();
    let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
    drop(processor);
    unsafe {
        __switch(switched_task_cx_ptr, idle_task_cx_ptr);
    }
}
