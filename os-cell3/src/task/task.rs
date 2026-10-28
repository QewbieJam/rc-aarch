//! 任务/进程控制块（ch8）—— 对照 rCore ch8 的 `os/src/task/task.rs`
//!
//! ch7 之前一个 TCB = 一个进程，自己拥有地址空间和 fd 表。
//! ch8 把"进程"和"线程"拆开（和 upstream 的 `ProcessControlBlock` /
//! `TaskControlBlock` 一一对应）：
//!
//! ```text
//!   ProcessControlBlock (PCB)          ← 一个进程一个
//!     pid / exit_code / is_zombie
//!     memory_set（页表 + 所有用户页）   ← 同进程的所有线程共享
//!     fd_table / heap_bottom / user_sp
//!     parent / children / threads: Vec<Arc<TCB>>
//!
//!   TaskControlBlock (TCB)             ← 一个线程一个
//!     tid / kernel_stack / asid
//!     process: Option<Arc<PCB>>        ← 指向共享的进程
//!     trap_cx_ptr（内核栈顶的用户现场）
//!     task_cx / task_status / 信号字段
//! ```
//!
//! ★ 和 upstream 的一个刻意差别：
//!   `thread_create` 多传一个 `stack_top`。upstream 让所有线程共用同一个用户栈顶
//!   （测试里线程函数很小，侥幸不炸）；我们显式给每个线程一块用户栈（app 里用
//!   `static` 数组），语义更干净，也让后面的 mutex/semaphore 测试敢开大循环。

use super::action::SignalActions;
use super::pid::{KernelStack, PidHandle, pid_alloc};
use super::signal::SignalFlags;
use super::TaskContext;
use crate::fs::{File, Stdout};
use crate::mm::MemorySet;
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefMut;

// ===========================================================================
// 进程控制块（PCB）
// ===========================================================================

/// 进程控制块：一个进程一份（地址空间、fd 表、父子关系都挂这里）
pub struct ProcessControlBlock {
    /// 进程号（Drop 时自动还回 pid 分配器）
    pub pid: PidHandle,
    inner: UPSafeCell<ProcessControlBlockInner>,
}

/// PCB 的可变部分
pub struct ProcessControlBlockInner {
    /// 地址空间（同进程的所有线程共享这一份）
    pub memory_set: MemorySet,
    /// 用户栈顶（thread_create 给新线程当 sp 用；exec 时更新）
    pub user_sp: usize,
    /// sbrk 的堆底 / 当前 program break
    pub heap_bottom: usize,
    pub program_brk: usize,
    /// 父进程（弱引用，避免父子互相持有）
    pub parent: Option<Weak<ProcessControlBlock>>,
    /// 子进程
    pub children: Vec<Arc<ProcessControlBlock>>,
    /// 进程退出码（Zombie 才有意义）
    pub exit_code: i32,
    /// 这个进程的所有线程（threads[0] 是主线程，tid == pid）
    pub threads: Vec<Arc<TaskControlBlock>>,
    /// 进程是否已退出（主线程退出后为 true）
    pub is_zombie: bool,
    /// 打开的文件表（0/1/2 = stdin/stdout/stderr）
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,
}

impl ProcessControlBlockInner {
    pub fn is_zombie(&self) -> bool {
        self.is_zombie
    }
    /// 找一个空闲 fd（0/1/2 保留给标准输入输出）
    pub fn alloc_fd(&mut self) -> usize {
        if let Some(fd) = (3..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            return fd;
        }
        self.fd_table.push(None);
        self.fd_table.len() - 1
    }
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
}

impl ProcessControlBlock {
    pub fn inner_exclusive_access(&self) -> RefMut<'_, ProcessControlBlockInner> {
        self.inner.exclusive_access()
    }
    pub fn getpid(&self) -> usize {
        self.pid.0
    }

    /// 进程退出：标记僵尸、回收用户页、把孤儿托付给 initproc
    pub fn exit(&self, exit_code: i32) {
        let children = {
            let mut inner = self.inner_exclusive_access();
            inner.is_zombie = true;
            inner.exit_code = exit_code;
            inner.memory_set.recycle_data_pages();
            for thread in inner.threads.iter() {
                thread.inner_exclusive_access().task_status = TaskStatus::Zombie;
            }
            core::mem::take(&mut inner.children)
        };
        if !children.is_empty() {
            let init = crate::task::initproc();
            let init_process = init.process();
            let mut init_inner = init_process.inner_exclusive_access();
            for child in children.iter() {
                child.inner_exclusive_access().parent = Some(Arc::downgrade(&init_process));
                init_inner.children.push(child.clone());
            }
        }
    }
}

// ===========================================================================
// 线程控制块（TCB）
// ===========================================================================

/// 线程控制块：一个线程一份
pub struct TaskControlBlock {
    /// 所属进程（`None` 只出现在内核的 init 线程；用户线程一定有）
    ///
    /// ★ 用 Weak 避免引用环：PCB.threads 持有 TCB，TCB 再强引用 PCB 就永远回收不掉。
    pub process: Option<Weak<ProcessControlBlock>>,
    /// 线程号（全局唯一；主线程的 tid == 进程 pid）
    pub tid: usize,
    /// 非主线程的 tid 所有权凭证（Drop 时把 tid 还回分配器；主线程为 None，
    /// 它的 tid 就是 PCB 的 pid）。★ 忘了持有它的话 tid 会被复用，多个线程
    /// 会拿到同一个 tid（ch8 调试时踩过）。
    pub tid_handle: Option<PidHandle>,
    /// 内核栈（Drop 时还页帧）
    #[allow(dead_code)]
    pub kernel_stack: KernelStack,
    /// TTBR0 用的 ASID
    pub asid: u16,
    inner: UPSafeCell<TaskControlBlockInner>,
}

/// TCB 的可变部分（线程私有的现场 + 信号）
pub struct TaskControlBlockInner {
    /// 内核栈顶那份 TrapContext 的地址（用户态现场）
    pub trap_cx_ptr: usize,
    /// 内核态切换上下文
    pub task_cx: TaskContext,
    pub task_status: TaskStatus,
    /// 线程退出码（`waittid` 用）
    pub exit_code: i32,

    // ---- ch7：信号（按线程保存；kill(pid) 会挂到主线程上）----
    pub signals: SignalFlags,
    pub signal_mask: SignalFlags,
    pub handling_sig: isize,
    pub signal_actions: SignalActions,
    pub killed: bool,
    pub frozen: bool,
    pub trap_ctx_backup: Option<TrapContext>,
}

impl TaskControlBlockInner {
    pub fn get_trap_cx(&self) -> &'static mut TrapContext {
        unsafe { &mut *(self.trap_cx_ptr as *mut TrapContext) }
    }
    pub fn is_zombie(&self) -> bool {
        self.task_status == TaskStatus::Zombie
    }
}

impl TaskControlBlock {
    pub fn inner_exclusive_access(&self) -> RefMut<'_, TaskControlBlockInner> {
        self.inner.exclusive_access()
    }

    /// 所属进程（用户线程必须有）
    pub fn process(&self) -> Arc<ProcessControlBlock> {
        match self.process.as_ref().expect("内核线程没有 process").upgrade() {
            Some(p) => p,
            None => {
                let status = self.inner_exclusive_access().task_status;
                crate::println!(
                    "[kernel] TCB tid={} status={:?} 的进程已经回收（调试信息）",
                    self.tid,
                    status
                );
                panic!("进程已经回收了")
            }
        }
    }

    pub fn getpid(&self) -> usize {
        self.process().getpid()
    }
    pub fn gettid(&self) -> usize {
        self.tid
    }

    fn asid_of(tid: usize) -> u16 {
        (tid & 0xff) as u16
    }

    /// 首次进入线程时的公共部分：新内核栈 + 初始 TrapContext + 登记进 PCB
    fn new_thread(
        process: Arc<ProcessControlBlock>,
        tid: usize,
        tid_handle: Option<PidHandle>,
        entry: usize,
        arg: usize,
        sp: usize,
    ) -> Arc<Self> {
        let kernel_stack = KernelStack::new();
        let mut cx = TrapContext::app_init_context(entry, sp);
        cx.x[0] = arg; // 新线程的第一个参数
        let trap_cx_ptr = kernel_stack.push_on_top(cx);
        let thread = Arc::new(Self {
            process: Some(Arc::downgrade(&process)),
            tid,
            tid_handle,
            kernel_stack,
            asid: Self::asid_of(tid),
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ptr,
                    task_cx: TaskContext::goto_trap_return(trap_cx_ptr),
                    task_status: TaskStatus::Ready,
                    exit_code: 0,
                    signals: SignalFlags::empty(),
                    signal_mask: SignalFlags::empty(),
                    handling_sig: -1,
                    signal_actions: SignalActions::default(),
                    killed: false,
                    frozen: false,
                    trap_ctx_backup: None,
                })
            },
        });
        process.inner_exclusive_access().threads.push(thread.clone());
        thread
    }

    /// 从 ELF 新建一个进程（= PCB + 一个主线程），对照 upstream 的 `TaskControlBlock::new`
    pub fn new(elf_data: &[u8]) -> Arc<Self> {
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        let pid_handle = pid_alloc();
        let tid = pid_handle.0;
        let process = Arc::new(ProcessControlBlock {
            pid: pid_handle,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    memory_set,
                    user_sp,
                    heap_bottom: user_sp,
                    program_brk: user_sp,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    threads: Vec::new(),
                    is_zombie: false,
                    fd_table: vec![
                        Some(Arc::new(crate::fs::Stdin)),
                        Some(Arc::new(Stdout)),
                        Some(Arc::new(Stdout)),
                    ],
                })
            },
        });

        let kernel_stack = KernelStack::new();
        // initproc 不需要命令行参数：x0 = argc = 0, x1 = argv = NULL
        let mut cx = TrapContext::app_init_context(entry_point, user_sp);
        cx.x[0] = 0;
        cx.x[1] = 0;
        let trap_cx_ptr = kernel_stack.push_on_top(cx);
        let thread = Arc::new(Self {
            process: Some(Arc::downgrade(&process)),
            tid,
            tid_handle: None,
            kernel_stack,
            asid: Self::asid_of(tid),
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ptr,
                    task_cx: TaskContext::goto_trap_return(trap_cx_ptr),
                    task_status: TaskStatus::Ready,
                    exit_code: 0,
                    signals: SignalFlags::empty(),
                    signal_mask: SignalFlags::empty(),
                    handling_sig: -1,
                    signal_actions: SignalActions::default(),
                    killed: false,
                    frozen: false,
                    trap_ctx_backup: None,
                })
            },
        });
        process.inner_exclusive_access().threads.push(thread.clone());
        crate::task::register_process(process);
        thread
    }

    /// 在同一个进程里新建一个线程（ch8 的 `sys_thread_create` 用）
    ///
    /// `entry`/`arg` 是用户 VA；`sp` 由用户给（每线程一块栈）。
    pub fn new_thread_in(
        process: Arc<ProcessControlBlock>,
        entry: usize,
        arg: usize,
        sp: usize,
    ) -> Arc<Self> {
        let tid_handle = pid_alloc();
        let tid = tid_handle.0;
        let thread = Self::new_thread(process, tid, Some(tid_handle), entry, arg, sp);
        crate::mm::sync_icache_all();
        thread
    }

    /// 换地址空间（run_tasks 切过去之前调用）
    pub fn activate(&self) {
        if let Some(process) = self.process.as_ref().and_then(|w| w.upgrade()) {
            let inner = process.inner_exclusive_access();
            inner.memory_set.activate(self.asid);
        }
    }

    /// 当前线程的页表 token（做用户指针翻译用）
    pub fn user_token(&self) -> usize {
        self.process().inner_exclusive_access().get_user_token()
    }

    /// 换成另一个 ELF 并传入命令行参数（对照 upstream ch7 的 `exec(elf_data, args)`）
    ///
    /// 只改**当前线程**的现场；同进程的其它线程会继续用旧现场（ch8 的测试不会在
    /// 多线程里 exec，这里记一笔：真要支持得把其它线程一并杀掉）。
    pub fn exec(&self, elf_data: &[u8], args: &[alloc::string::String]) {
        let (memory_set, mut user_sp, entry_point) = MemorySet::from_elf(elf_data);
        // ★ 堆区的起点是"没有压 argv 之前的栈顶"（from_elf 建的 0 长 MapArea 在那里）；
        //   压完 argv 后 user_sp 会降到堆区起点以下，不能再拿它当 heap_bottom，
        //   否则 sbrk 的 append_to 找不到对应 MapArea 会一直返回 -1（ch7 埋下、ch8 验收时暴露）。
        let heap_base = user_sp;

        // ---- 把 argv 压到用户栈上（布局见 ch7 的说明）----
        let mut argv_base = 0usize;
        if !args.is_empty() {
            user_sp -= (args.len() + 1) * core::mem::size_of::<usize>();
            argv_base = user_sp;
            for i in 0..=args.len() {
                *crate::mm::translated_refmut::<usize>(
                    memory_set.token(),
                    (argv_base + i * 8) as *mut usize,
                ) = 0;
            }
            for i in 0..args.len() {
                let bytes = args[i].as_bytes();
                user_sp -= bytes.len() + 1;
                *crate::mm::translated_refmut::<usize>(
                    memory_set.token(),
                    (argv_base + i * 8) as *mut usize,
                ) = user_sp;
                for (j, &c) in bytes.iter().enumerate() {
                    *crate::mm::translated_refmut::<u8>(
                        memory_set.token(),
                        (user_sp + j) as *mut u8,
                    ) = c;
                }
                *crate::mm::translated_refmut::<u8>(
                    memory_set.token(),
                    (user_sp + bytes.len()) as *mut u8,
                ) = 0;
            }
            user_sp &= !15;
        }

        {
            let process = self.process();
            let mut pcb = process.inner_exclusive_access();
            pcb.memory_set = memory_set;
            pcb.user_sp = user_sp;
            pcb.heap_bottom = heap_base;
            pcb.program_brk = heap_base;
        }
        let mut cx = TrapContext::app_init_context(entry_point, user_sp);
        cx.x[0] = args.len();
        cx.x[1] = argv_base;
        *self.inner_exclusive_access().get_trap_cx() = cx;
        self.activate();
    }

    /// 复制当前进程（对照 upstream 的 `fork`）：新 PCB + 一个新线程
    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        let parent_process = self.process();
        let (memory_set, user_sp, heap_bottom, program_brk, fd_table, parent_cx, sig_mask, sig_actions) = {
            let pcb = parent_process.inner_exclusive_access();
            let inner = self.inner_exclusive_access();
            (
                MemorySet::from_existed_user(&pcb.memory_set),
                pcb.user_sp,
                pcb.heap_bottom,
                pcb.program_brk,
                pcb.fd_table.clone(),
                *inner.get_trap_cx(),
                inner.signal_mask,
                inner.signal_actions.clone(),
            )
        };

        let pid_handle = pid_alloc();
        let tid = pid_handle.0;
        let process = Arc::new(ProcessControlBlock {
            pid: pid_handle,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    memory_set,
                    user_sp,
                    heap_bottom,
                    program_brk,
                    parent: Some(Arc::downgrade(&parent_process)),
                    children: Vec::new(),
                    exit_code: 0,
                    threads: Vec::new(),
                    is_zombie: false,
                    fd_table,
                })
            },
        });

        let kernel_stack = KernelStack::new();
        let mut cx = parent_cx;
        cx.x[0] = 0; // fork 在子进程里返回 0
        let trap_cx_ptr = kernel_stack.push_on_top(cx);
        let thread = Arc::new(Self {
            process: Some(Arc::downgrade(&process)),
            tid,
            tid_handle: None,
            kernel_stack,
            asid: Self::asid_of(tid),
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ptr,
                    task_cx: TaskContext::goto_trap_return(trap_cx_ptr),
                    task_status: TaskStatus::Ready,
                    exit_code: 0,
                    signals: SignalFlags::empty(),
                    signal_mask: sig_mask,
                    handling_sig: -1,
                    signal_actions: sig_actions,
                    killed: false,
                    frozen: false,
                    trap_ctx_backup: None,
                })
            },
        });
        process.inner_exclusive_access().threads.push(thread.clone());
        crate::task::register_process(process.clone());
        parent_process.inner_exclusive_access().children.push(process);
        crate::mm::sync_icache_all();
        thread
    }
}

/// 进程/线程状态（ch5 起：没有 Exited，退出就是 Zombie）
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    Ready,
    Running,
    Zombie,
}
