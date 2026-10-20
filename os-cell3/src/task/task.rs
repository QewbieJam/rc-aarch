//! 任务控制块（ch3）—— 对照 rCore ch3 的 `os/src/task/task.rs`
//!
//! 比 upstream 多存了一个 `task_cx` 之外的字段？没有 —— 保持最小：
//! 状态 + 切换上下文。ch5 的进程才需要页表、文件描述符这些东西。

use super::TaskContext;

/// 任务控制块（TCB）
#[derive(Copy, Clone)]
pub struct TaskControlBlock {
    /// 任务状态机：UnInit → Ready → Running → Exited
    pub task_status: TaskStatus,
    /// 换出/换入这个任务时要保存/恢复的上下文
    pub task_cx: TaskContext,
}

/// 任务状态（对照 rCore 的 `TaskStatus`）
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    /// 还没初始化（静态数组的初始值）
    UnInit,
    /// 准备好了，等着被调度
    Ready,
    /// 正在 CPU 上跑
    Running,
    /// 已退出，不会再被调度
    Exited,
}
