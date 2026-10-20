//! 任务/进程管理（ch5）—— 对照 rCore ch5 的 `os/src/task/mod.rs`
//!
//! ch5 的模块划分（和 upstream 一一对应）：
//!   task/context.rs    TaskContext（不变）
//!   task/switch.S/.rs  __switch（不变）
//!   task/task.rs       TaskControlBlock（Arc + inner）
//!   task/pid.rs        pid 分配 + 内核栈
//!   task/manager.rs    就绪队列（FIFO）
//!   task/processor.rs  Processor + run_tasks + schedule
//!   task/mod.rs        把上面几个粘起来：suspend / exit / initproc
//!
//! ★ 和 ch4 最大的行为变化：
//!   - 不再"数组里扫 Ready"，而是 **idle 循环 + 就绪队列**；
//!   - 进程退出 → **Zombie**（保留 TCB 等父进程 waitpid 回收），
//!     而不是直接从调度器消失；
//!   - 第一个进程固定是 `initproc`（pid 1），它退出就意味着系统该关机了。

mod context;
mod manager;
mod pid;
mod processor;
mod switch;

#[allow(clippy::module_inception)]
mod task;

use crate::loader::get_app_data_by_name;
use crate::sync::UPSafeCell;
use alloc::sync::Arc;
use task::{TaskControlBlock, TaskStatus};

pub use context::TaskContext;
pub use manager::add_task;
pub use processor::{
    current_task, current_task_id, current_user_token, run_tasks, schedule, take_current_task,
};

/// initproc 的 pid（它退出 = 系统关机）
pub const IDLE_PID: usize = 1;

/// 全局唯一的 initproc（也会被用来"托管"孤儿进程）
static INITPROC: UPSafeCell<Option<Arc<TaskControlBlock>>> = unsafe { UPSafeCell::new(None) };

/// 取 initproc 的 Arc
pub fn initproc() -> Arc<TaskControlBlock> {
    INITPROC
        .exclusive_access()
        .as_ref()
        .expect("initproc 还没创建")
        .clone()
}

/// 创建 initproc 并放进就绪队列（对照 upstream 的 `add_initproc`）
pub fn add_initproc() {
    let initproc = Arc::new(TaskControlBlock::new(
        get_app_data_by_name("initproc").expect("找不到 initproc 这个 app"),
    ));
    *INITPROC.exclusive_access() = Some(initproc.clone());
    add_task(initproc);
    // initproc 的代码是刚加载的（新物理页），同步一次 I-cache
    crate::mm::sync_icache_all();
}

/// 主动让出：Running → Ready，回就绪队列，切回调度器
pub fn suspend_current_and_run_next() {
    let task = take_current_task().expect("没有正在运行的进程");
    let mut inner = task.inner_exclusive_access();
    let task_cx_ptr = &mut inner.task_cx as *mut TaskContext;
    inner.task_status = TaskStatus::Ready;
    drop(inner);

    add_task(task);
    schedule(task_cx_ptr);
}

/// 退出当前进程：Running → Zombie，记录退出码，切回调度器
///
/// 返回类型是 `!`：Zombie 永远不会再被调度，所以这个函数不会返回。
pub fn exit_current_and_run_next(exit_code: i32) -> ! {
    let task = take_current_task().expect("没有正在运行的进程");
    let pid = task.getpid();

    // ★ initproc 退出 = 系统该关了（对应 upstream 的 IDLE_PID 判断）
    if pid == IDLE_PID {
        println!("[kernel] initproc 退出，exit_code={}，系统停机", exit_code);
        crate::trap::shutdown_with(exit_code);
    }

    let mut inner = task.inner_exclusive_access();
    inner.task_status = TaskStatus::Zombie;
    inner.exit_code = exit_code;

    // 孤儿进程托付给 initproc（对照 upstream 的 reparent 逻辑）
    {
        let init = initproc();
        let mut init_inner = init.inner_exclusive_access();
        for child in inner.children.iter() {
            child.inner_exclusive_access().parent = Some(Arc::downgrade(&init));
            init_inner.children.push(child.clone());
        }
    }
    inner.children.clear();
    // 用户页帧马上还回去（页表留着，等 waitpid 回收 TCB 时一起释放）
    inner.memory_set.recycle_data_pages();
    drop(inner);
    // 手动 drop：如果父进程已经不管我们了，这里就是最后一个 Arc
    drop(task);

    // 不再保存上下文，直接切回调度器
    let mut _unused = TaskContext::zero_init();
    schedule(&mut _unused as *mut TaskContext);
    panic!("Zombie 进程不应该被再次调度");
}

/// sbrk：改当前进程的 program break
pub fn change_program_brk(size: i32) -> Option<usize> {
    let task = current_task().expect("没有正在运行的进程");
    let mut inner = task.inner_exclusive_access();
    let old_break = inner.program_brk;
    let new_brk = inner.program_brk as isize + size as isize;
    if new_brk < inner.heap_bottom as isize {
        return None;
    }
    let heap_bottom = inner.heap_bottom;
    let result = if size < 0 {
        inner
            .memory_set
            .shrink_to(heap_bottom.into(), (new_brk as usize).into())
    } else {
        inner
            .memory_set
            .append_to(heap_bottom.into(), (new_brk as usize).into())
    };
    if result {
        inner.program_brk = new_brk as usize;
        let asid = task.asid;
        inner.memory_set.activate(asid);
        Some(old_break)
    } else {
        None
    }
}
