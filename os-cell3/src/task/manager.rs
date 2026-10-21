//! 就绪队列（ch5/ch8）—— 对照 rCore ch8 的 `os/src/task/manager.rs`
//!
//! ch8 起就绪队列里放的是**线程**（`Arc<TaskControlBlock>`）；进程退出后
//! 它的所有线程都标记成 Zombie，`fetch` 会把它们跳过。
//!
//! 另外维护 pid → **进程**（PCB）的映射，给 `kill(pid)` / `waitpid` 用。

use super::{ProcessControlBlock, TaskControlBlock, TaskStatus};
use crate::sync::UPSafeCell;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;

pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl TaskManager {
    const fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }
    fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        while let Some(task) = self.ready_queue.pop_front() {
            if task.inner_exclusive_access().task_status != TaskStatus::Zombie {
                return Some(task);
            }
        }
        None
    }
}

static TASK_MANAGER: UPSafeCell<TaskManager> = unsafe { UPSafeCell::new(TaskManager::new()) };

/// pid → 进程（ch7 的 kill、ch8 的 waitpid 用）
static PID2PCB: UPSafeCell<BTreeMap<usize, Arc<ProcessControlBlock>>> =
    unsafe { UPSafeCell::new(BTreeMap::new()) };

/// 登记进程（PCB 的强引用放这里；线程里只存 Weak，避免引用环）
///
/// ★ 必须在 `TaskControlBlock::new`/`fork` 返回之前调用：否则 PCB 的 Arc 在
///   函数返回时就归零了，等 add_task 再登记已经来不及（ch8 调试时踩过）。
pub fn register_process(process: Arc<ProcessControlBlock>) {
    PID2PCB
        .exclusive_access()
        .insert(process.getpid(), process);
}

/// 把新任务放进就绪队列
pub fn add_task(task: Arc<TaskControlBlock>) {
    TASK_MANAGER.exclusive_access().add(task);
}

/// 唤醒一个睡在锁/信号量/条件变量上的线程：只放回就绪队列，不重复登记
pub fn wakeup_task(task: Arc<TaskControlBlock>) {
    TASK_MANAGER.exclusive_access().add(task);
}

/// 按 pid 找进程
pub fn pid2process(pid: usize) -> Option<Arc<ProcessControlBlock>> {
    PID2PCB.exclusive_access().get(&pid).map(Arc::clone)
}

/// 进程退出/回收时把 pid 从表里摘掉
pub fn remove_from_pid2process(pid: usize) {
    let mut map = PID2PCB.exclusive_access();
    if map.remove(&pid).is_none() {
        panic!("pid2process 里找不到 pid {}", pid);
    }
}

/// 从就绪队列取一个任务
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    TASK_MANAGER.exclusive_access().fetch()
}
