//! 任务管理（ch3）—— 对照 rCore ch3 的 `os/src/task/mod.rs`
//!
//! 这个文件是 ch3 的核心：所有任务的状态都在 `TaskManager` 里，
//! 切换任务的唯一入口是 `switch::__switch`（汇编）。
//!
//! ★ 和 upstream 的差别只有一处：upstream 用 `lazy_static!` 在第一次访问时
//!   初始化 `TASK_MANAGER`；我们没有 `lazy_static` 依赖（ch2 就刻意避开了），
//!   改成"`const fn` 静态构造 + `init()` 显式填表"，语义等价：
//!
//!   lazy_static::TASK_MANAGER  = { get_num_app(); 给每个任务建 task_cx; ... }
//!   → `static TASK_MANAGER`（全 UnInit）+ `task::init()`（填 task_cx / Ready）
//!
//! ★★ 调度算法（ch3）：**简单轮转（round-robin）**
//!   - 每个任务最多跑一个时间片（`timer.rs` 的 10ms），时间片到了由定时器中断
//!     打断 → `suspend_current_and_run_next()`；
//!   - 主动让出：`sys_yield` → 同一个函数；
//!   - 退出：`sys_exit` / 异常 → `exit_current_and_run_next()`（Exited 不再调度）。
//!
//! ⚠️ 注意 `__switch` 的"控制流陷阱"（upstream 注释里也反复提醒）：
//!   调用 `__switch(a, b)` 之后，代码看起来"接着往下执行"，但实际上是
//!   **将来某个时刻被换回来时**才继续。所以：
//!     - 切换前必须 `drop()` 掉所有 `exclusive_access()` 的借用；
//!     - 切换点前后不要假设局部变量还有效（它们在内核栈上，栈是跟着任务走的）。

mod context;
mod switch;

#[allow(clippy::module_inception)]
mod task;

use crate::config::MAX_APP_NUM;
use crate::loader::{get_num_app, init_app_cx};
use crate::sync::UPSafeCell;
use switch::__switch;
use task::{TaskControlBlock, TaskStatus};

pub use context::TaskContext;

/// 任务管理器（对照 rCore 的 `TaskManager`）
pub struct TaskManager {
    /// 用 `UPSafeCell` 把内部状态藏起来，把借用检查推迟到运行时
    inner: UPSafeCell<TaskManagerInner>,
}

/// 任务管理器的内部状态（对照 rCore 的 `TaskManagerInner`）
pub struct TaskManagerInner {
    /// 所有任务（静态数组：这个阶段 app 个数是编译期就定死的）
    tasks: [TaskControlBlock; MAX_APP_NUM],
    /// 当前正在跑的任务号
    current_task: usize,
    /// 实际有几个任务
    num_app: usize,
}

/// 任务表是否已经初始化好（ch3-A 的定时器自测路径里没有任务，中断处理要能区分）
static mut TASKS_READY: bool = false;

/// 任务表就绪没？（给 irq_dispatch 判断"要不要切任务"）
pub fn is_ready() -> bool {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TASKS_READY)) }
}

/// 全局唯一实例（对照 upstream 的 `lazy_static! { TASK_MANAGER }`）
static TASK_MANAGER: TaskManager = TaskManager::new();

impl TaskManager {
    /// 编译期构造：全部 UnInit，等 `init()` 填
    const fn new() -> Self {
        Self {
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks: [TaskControlBlock {
                        task_status: TaskStatus::UnInit,
                        task_cx: TaskContext::zero_init(),
                    }; MAX_APP_NUM],
                    current_task: 0,
                    num_app: 0,
                })
            },
        }
    }
}

impl TaskManager {
    /// 跑第一个任务（对照 rCore 的 `run_first_task`）
    ///
    /// `__switch` 从这里"一去不回"：换到任务 0 的栈上，`ret` 到 `__restore_sp`，
    /// 再 `eret` 到 EL0。
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let task0 = &mut inner.tasks[0];
        task0.task_status = TaskStatus::Running;
        let next_task_cx_ptr = &task0.task_cx as *const TaskContext;
        drop(inner);

        let mut _unused = TaskContext::zero_init();
        unsafe {
            __switch(&mut _unused as *mut TaskContext, next_task_cx_ptr);
        }
        panic!("run_first_task: 不应该回到这里");
    }

    /// 当前任务：Running → Ready（对照 `mark_current_suspended`）
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Ready;
    }

    /// 当前任务：Running → Exited（对照 `mark_current_exited`）
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Exited;
    }

    /// 从当前任务往后找第一个 Ready 的（对照 `find_next_task`）
    fn find_next_task(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        let num_app = inner.num_app;
        (current + 1..current + num_app + 1)
            .map(|id| id % num_app)
            .find(|id| inner.tasks[*id].task_status == TaskStatus::Ready)
    }

    /// 切到下一个任务；一个 Ready 都没有 = 全部跑完（对照 `run_next_task`）
    fn run_next_task(&self) {
        if let Some(next) = self.find_next_task() {
            let mut inner = self.inner.exclusive_access();
            let current = inner.current_task;
            inner.tasks[next].task_status = TaskStatus::Running;
            inner.current_task = next;

            let current_task_cx_ptr = &mut inner.tasks[current].task_cx as *mut TaskContext;
            let next_task_cx_ptr = &inner.tasks[next].task_cx as *const TaskContext;
            drop(inner); // ★ 换出去之前必须放掉借用

            unsafe {
                __switch(current_task_cx_ptr, next_task_cx_ptr);
            }
            // 将来被换回来时从这里继续
        } else {
            println!("[kernel] All applications completed!");
            crate::trap::shutdown();
        }
    }
}

/// 初始化任务表（对应 upstream 的 lazy_static 初始化块）
pub fn init() {
    let num_app = get_num_app();
    assert!(num_app <= MAX_APP_NUM, "app 太多（改 config::MAX_APP_NUM）");

    let mut inner = TASK_MANAGER.inner.exclusive_access();
    inner.num_app = num_app;
    for i in 0..num_app {
        // 每个任务的内核栈顶压一份"初始 TrapContext"，sp 指向它
        inner.tasks[i].task_cx = TaskContext::goto_restore(init_app_cx(i));
        inner.tasks[i].task_status = TaskStatus::Ready;
    }
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(TASKS_READY), true) };
    println!("[kernel] task manager: {} 个任务就绪", num_app);
}

/// 跑第一个任务（`main.rs` 的最后一句）
pub fn run_first_task() -> ! {
    TASK_MANAGER.run_first_task()
}

/// 主动让出当前任务（`sys_yield` 用）
pub fn suspend_current_and_run_next() {
    TASK_MANAGER.mark_current_suspended();
    TASK_MANAGER.run_next_task();
}

/// 结束当前任务并切走（`sys_exit` 和异常处理用）
///
/// 返回类型是 `!`：Exited 的任务不会再被调度，所以这个函数**永远不会返回**。
pub fn exit_current_and_run_next() -> ! {
    TASK_MANAGER.mark_current_exited();
    TASK_MANAGER.run_next_task();
    panic!("exit_current_and_run_next: Exited 的任务不应该被再次调度");
}
