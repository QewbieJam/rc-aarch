//! 任务管理（ch4）—— 对照 rCore ch4 的 `os/src/task/mod.rs`
//!
//! 和 ch3 相比，调度算法**一点没变**（还是轮转），变的是"任务是什么"：
//!   ch3：任务 = 一个固定的物理地址槽位 + 静态栈；
//!   ch4：任务 = 一套**独立的页表**（MemorySet），物理页由 frame allocator 给。
//!
//! 于是 `__switch` 前后多了两件事：
//!   ① 切 TTBR0（`memory_set.activate(asid)`）—— 换地址空间
//!   ② 内核映射在**每个任务页表里都有**（共享 L1[3] 子树），所以切过去之后
//!      内核代码/内核栈照样能访问，不需要像 RISC-V 那样切回"内核 satp"。
//!
//! ★ `tasks` 从 `[TCB; MAX_APP_NUM]` 变成了 `Vec<TCB>`（对照 upstream），
//!   所以 TASK_MANAGER 改成"静态构造 + `init()` 里 push"，
//!   依然不引 `lazy_static`。

mod context;
mod switch;

#[allow(clippy::module_inception)]
mod task;

use crate::loader::{get_app_data, get_num_app};
use crate::sync::UPSafeCell;
use alloc::vec::Vec;
use switch::__switch;
use task::{TaskControlBlock, TaskStatus};

pub use context::TaskContext;

/// 任务管理器
pub struct TaskManager {
    inner: UPSafeCell<TaskManagerInner>,
}

struct TaskManagerInner {
    tasks: Vec<TaskControlBlock>,
    current_task: usize,
}

/// 全局唯一实例
static TASK_MANAGER: TaskManager = TaskManager {
    inner: unsafe { UPSafeCell::new(TaskManagerInner { tasks: Vec::new(), current_task: 0 }) },
};

/// 任务表是否已经建好（ch3-A 那种"没有任务的定时器自测"要用）
static mut TASKS_READY: bool = false;

/// 任务表就绪没？
pub fn is_ready() -> bool {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TASKS_READY)) }
}

impl TaskManager {
    /// 跑第一个任务
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let task0 = &mut inner.tasks[0];
        task0.task_status = TaskStatus::Running;
        let next_task_cx_ptr = &task0.task_cx as *const TaskContext;
        let asid = task0.asid;
        // ★ 先把 TTBR0 切到任务 0 的页表（内核映射共享，所以这条语句本身安全）
        task0.memory_set.activate(asid);
        drop(inner);

        let mut _unused = TaskContext::zero_init();
        unsafe {
            __switch(&mut _unused as *mut TaskContext, next_task_cx_ptr);
        }
        panic!("run_first_task: 不应该回到这里");
    }

    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Ready;
    }

    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].task_status = TaskStatus::Exited;
    }

    fn find_next_task(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        let num_app = inner.tasks.len();
        (current + 1..current + num_app + 1)
            .map(|id| id % num_app)
            .find(|id| inner.tasks[*id].task_status == TaskStatus::Ready)
    }

    fn get_current_token(&self) -> usize {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].token()
    }

    fn change_current_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].change_program_brk(size)
    }

    fn run_next_task(&self) {
        if let Some(next) = self.find_next_task() {
            let mut inner = self.inner.exclusive_access();
            let current = inner.current_task;
            inner.tasks[next].task_status = TaskStatus::Running;
            inner.current_task = next;

            let current_task_cx_ptr = &mut inner.tasks[current].task_cx as *mut TaskContext;
            let next_task_cx_ptr = &inner.tasks[next].task_cx as *const TaskContext;
            let asid = inner.tasks[next].asid;
            // ★ 换地址空间（TTBR0）+ 清 TLB，再换栈/换执行流
            inner.tasks[next].memory_set.activate(asid);
            drop(inner);

            unsafe {
                __switch(current_task_cx_ptr, next_task_cx_ptr);
            }
        } else {
            println!("[kernel] All applications completed!");
            crate::trap::shutdown();
        }
    }
}

/// 建任务表（对应 upstream 的 lazy_static 初始化块）
pub fn init() {
    let num_app = get_num_app();
    println!("[kernel] num_app = {}", num_app);
    let mut inner = TASK_MANAGER.inner.exclusive_access();
    for i in 0..num_app {
        println!("[kernel] 建任务 {}（ELF {} 字节）", i, get_app_data(i).len());
        inner.tasks.push(TaskControlBlock::new(get_app_data(i), i));
    }
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(TASKS_READY), true) };
    println!(
        "[kernel] task manager: {} 个任务就绪（每任务一套页表）",
        num_app
    );
}

/// 跑第一个任务
pub fn run_first_task() -> ! {
    TASK_MANAGER.run_first_task()
}

/// 主动让出
pub fn suspend_current_and_run_next() {
    TASK_MANAGER.mark_current_suspended();
    TASK_MANAGER.run_next_task();
}

/// 结束当前任务
pub fn exit_current_and_run_next() -> ! {
    TASK_MANAGER.mark_current_exited();
    TASK_MANAGER.run_next_task();
    panic!("exit_current_and_run_next: Exited 的任务不应该被再次调度");
}

/// 当前任务号（调试用）
pub fn current_task_id() -> usize {
    TASK_MANAGER.inner.exclusive_access().current_task
}

/// 当前任务的页表 token（sys_write 翻译用户指针时用）
pub fn current_user_token() -> usize {
    TASK_MANAGER.get_current_token()
}

/// sbrk：改当前任务的 program break
pub fn change_program_brk(size: i32) -> Option<usize> {
    TASK_MANAGER.change_current_program_brk(size)
}
