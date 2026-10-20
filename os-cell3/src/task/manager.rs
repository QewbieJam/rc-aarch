//! 就绪队列（ch5）—— 对照 rCore ch5 的 `os/src/task/manager.rs`
//!
//! ch4 的调度是"在固定数组里找下一个 Ready"；ch5 起进程是**动态创建/销毁**的，
//! 所以改成 upstream 的 **FIFO 就绪队列（VecDeque）**：
//!   - 新建 / 被抢占 / yield 的进程都 `add_task()` 进队尾；
//!   - 调度器 `fetch_task()` 从队头取。
//!
//! `Arc<TaskControlBlock>` 是引用计数的：进程退出后 TCB 还挂在父进程的 `children`
//! 里（Zombie），等 `waitpid` 回收时最后一个 Arc 被 drop，TCB（含页表/页帧/内核栈）
//! 才真正释放。就绪队列里存的也是 Arc，不会出现"队列里还留着已释放的 TCB"。

use super::TaskControlBlock;
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
        self.ready_queue.pop_front()
    }
}

static TASK_MANAGER: UPSafeCell<TaskManager> = unsafe { UPSafeCell::new(TaskManager::new()) };

/// pid → TCB（ch7：`kill(pid)` / 父子关系查找要用）
static PID2TCB: UPSafeCell<BTreeMap<usize, Arc<TaskControlBlock>>> =
    unsafe { UPSafeCell::new(BTreeMap::new()) };

/// 把任务放进就绪队列（同时登记 pid）
pub fn add_task(task: Arc<TaskControlBlock>) {
    PID2TCB
        .exclusive_access()
        .insert(task.getpid(), Arc::clone(&task));
    TASK_MANAGER.exclusive_access().add(task);
}

/// 按 pid 找任务（ch7 的 `kill` 用）
pub fn pid2task(pid: usize) -> Option<Arc<TaskControlBlock>> {
    PID2TCB.exclusive_access().get(&pid).map(Arc::clone)
}

/// 进程退出/回收时把 pid 从表里摘掉
pub fn remove_from_pid2task(pid: usize) {
    let mut map = PID2TCB.exclusive_access();
    if map.remove(&pid).is_none() {
        panic!("pid2task 里找不到 pid {}", pid);
    }
}

/// 从就绪队列取一个任务
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    TASK_MANAGER.exclusive_access().fetch()
}
