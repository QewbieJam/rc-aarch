//! 任务/进程控制块（ch5）—— 对照 rCore ch5 的 `os/src/task/task.rs`
//!
//! ch5 把 ch4 的"裸 TCB"升级成真正的**进程**：
//!   - `Arc<TaskControlBlock>`：可以同时被就绪队列、父进程的 children 列表持有；
//!   - 内部用 `UPSafeCell` 保护可变状态（对应 upstream 的 `inner`）；
//!   - 增加 pid、内核栈、父子关系、退出码、Zombie 状态；
//!   - 会 `fork()`（复制地址空间）和 `exec()`（换掉地址空间）。
//!
//! ★ 和 upstream 的差别（都是因为 AArch64 不需要 trampoline）：
//!   1) upstream 有 `trap_cx_ppn`：TrapContext 放在用户空间固定 VA 的页里。
//!      我们的 TrapContext 就在**内核栈顶**，所以只存一个 `trap_cx_ptr`。
//!      好处：fork 时直接 `*child_cx = *parent_cx` 复制寄存器现场，
//!      exec 时直接覆盖，不需要页表换算。
//!   2) upstream 用 `KernelStack::new(&pid)` 映射固定 VA；我们用页帧池里的连续两页
//!      （见 pid.rs）。

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

/// 任务控制块：`pid` / 内核栈 是不变的，其它都在 `inner` 里
pub struct TaskControlBlock {
    /// 进程号（Drop 时自动回收）
    pub pid: PidHandle,
    /// 内核栈（Drop 时自动还页帧；这里"只持有不使用"，栈顶地址在 inner.trap_cx_ptr 里）
    #[allow(dead_code)]
    pub kernel_stack: KernelStack,
    /// TTBR0 用的 ASID
    pub asid: u16,
    inner: UPSafeCell<TaskControlBlockInner>,
}

/// TCB 的可变部分（对照 upstream 的 `TaskControlBlockInner`）
pub struct TaskControlBlockInner {
    /// 内核栈顶那份 TrapContext 的地址（用户态现场）
    pub trap_cx_ptr: usize,
    /// 内核态切换上下文
    pub task_cx: TaskContext,
    pub task_status: TaskStatus,
    /// 地址空间
    pub memory_set: MemorySet,
    /// sbrk 的堆底 / 当前 program break
    pub heap_bottom: usize,
    pub program_brk: usize,
    /// 父进程（弱引用，避免父子互相持有）
    pub parent: Option<Weak<TaskControlBlock>>,
    /// 子进程
    pub children: Vec<Arc<TaskControlBlock>>,
    /// 退出码（Zombie 才有意义）
    pub exit_code: i32,
    /// 打开的文件表（0=stdin 1=stdout 2=stderr，3 起是文件/管道）
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,

    // ---- ch7：信号 ----
    /// 待处理信号
    pub signals: SignalFlags,
    /// 屏蔽字（这些信号先不处理）
    pub signal_mask: SignalFlags,
    /// 正在处理哪个信号（-1 = 没有）
    pub handling_sig: isize,
    /// 每个信号的处理动作
    pub signal_actions: SignalActions,
    /// 被 SIGKILL 杀掉
    pub killed: bool,
    /// 被 SIGSTOP 冻住
    pub frozen: bool,
    /// 进信号处理函数前备份的 TrapContext（sigreturn 时恢复）
    pub trap_ctx_backup: Option<TrapContext>,
}

impl TaskControlBlockInner {
    /// 取用户态现场（就在内核栈顶，恒等映射，随时可访问）
    pub fn get_trap_cx(&self) -> &'static mut TrapContext {
        unsafe { &mut *(self.trap_cx_ptr as *mut TrapContext) }
    }
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
    pub fn is_zombie(&self) -> bool {
        self.task_status == TaskStatus::Zombie
    }

    /// 找一个空闲 fd（0/1/2 保留给标准输入输出）
    pub fn alloc_fd(&mut self) -> usize {
        if let Some(fd) = (3..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            return fd;
        }
        self.fd_table.push(None);
        self.fd_table.len() - 1
    }
}

impl TaskControlBlock {
    pub fn inner_exclusive_access(&self) -> RefMut<'_, TaskControlBlockInner> {
        self.inner.exclusive_access()
    }

    /// 从 ELF 新建一个进程（对照 upstream 的 `TaskControlBlock::new`）
    pub fn new(elf_data: &[u8]) -> Self {
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        let pid = pid_alloc();
        let kernel_stack = KernelStack::new();
        // 在内核栈顶压初始 TrapContext（initproc 不需要命令行参数）
        let mut cx = TrapContext::app_init_context(entry_point, user_sp);
        cx.x[0] = 0; // argc
        cx.x[1] = 0; // argv
        let trap_cx_ptr = kernel_stack.push_on_top(cx);
        Self {
            asid: Self::asid_of(pid.0),
            pid,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ptr,
                    task_cx: TaskContext::goto_trap_return(trap_cx_ptr),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    heap_bottom: user_sp,
                    program_brk: user_sp,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    signals: SignalFlags::empty(),
                    signal_mask: SignalFlags::empty(),
                    handling_sig: -1,
                    signal_actions: SignalActions::default(),
                    killed: false,
                    frozen: false,
                    trap_ctx_backup: None,
                    fd_table: vec![
                        Some(Arc::new(crate::fs::Stdin)),
                        Some(Arc::new(Stdout)),
                        Some(Arc::new(Stdout)),
                    ],
                })
            },
        }
    }

    fn asid_of(pid: usize) -> u16 {
        // ASID 在 TCR_EL1.AS=0 时是 8 位；我们每次切换都 tlbi vmalle1，所以
        // ASID 实际上不参与 TLB 匹配，取低 8 位即可（pid 从 1 开始，不会为 0）
        (pid & 0xff) as u16
    }

    /// 换成另一个 ELF 并传入命令行参数（对照 upstream ch7 的 `exec(elf_data, args)`）
    ///
    /// 参数压栈布局（和 upstream 一致，栈从高往低长）：
    /// ```text
    ///   [高]  argv[argc]  = NULL
    ///         argv[argc-1] -> 最后一个字符串
    ///         ...
    ///         argv[0]      -> 第一个字符串
    ///   [低]  "arg0\0" "arg1\0" ...
    /// ```
    /// 入口处 x0 = argc，x1 = argv（AArch64 的 a0/a1；upstream 是 x[10]/x[11]）。
    pub fn exec(&self, elf_data: &[u8], args: &[alloc::string::String]) {
        let (memory_set, mut user_sp, entry_point) = MemorySet::from_elf(elf_data);

        // ---- 把 argv 压到用户栈上 ----
        let mut argv_base = 0usize;
        if !args.is_empty() {
            // ① 先留出指针数组的位置（argc+1 个 usize，最后一个放 NULL）
            user_sp -= (args.len() + 1) * core::mem::size_of::<usize>();
            argv_base = user_sp;
            for i in 0..=args.len() {
                *crate::mm::translated_refmut::<usize>(
                    memory_set.token(),
                    (argv_base + i * 8) as *mut usize,
                ) = 0;
            }
            // ② 依次把字符串压到更低的地址，并回填指针
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
            // ③ AAPCS64 要求进入函数时 SP 16 字节对齐
            user_sp &= !15;
        }

        let mut inner = self.inner_exclusive_access();
        inner.memory_set = memory_set;
        inner.heap_bottom = user_sp;
        inner.program_brk = user_sp;
        // 用全新的 TrapContext 覆盖栈顶那份（入口/栈都换了）
        let mut cx = TrapContext::app_init_context(entry_point, user_sp);
        cx.x[0] = args.len(); // argc
        cx.x[1] = argv_base;  // argv
        *inner.get_trap_cx() = cx;
        // ★ 新页表要立刻生效：exec 之后不再经过调度器，直接 eret 回用户态
        inner.memory_set.activate(self.asid);
    }

    /// 复制当前进程（对照 upstream 的 `fork`）
    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        let mut parent_inner = self.inner_exclusive_access();
        // ① 深拷贝地址空间（每个用户页都是新分配的物理页）
        let memory_set = MemorySet::from_existed_user(&parent_inner.memory_set);
        // ② 新 pid + 新内核栈
        let pid = pid_alloc();
        let kernel_stack = KernelStack::new();
        // ③ 把父进程的寄存器现场复制到子进程内核栈顶，fork 在子进程里返回 0
        let trap_cx_ptr = kernel_stack.push_on_top(*parent_inner.get_trap_cx());
        unsafe {
            (*(trap_cx_ptr as *mut TrapContext)).x[0] = 0;
        }
        let child = Arc::new(Self {
            asid: Self::asid_of(pid.0),
            pid,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ptr,
                    task_cx: TaskContext::goto_trap_return(trap_cx_ptr),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    heap_bottom: parent_inner.heap_bottom,
                    program_brk: parent_inner.program_brk,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    // ★ fork：待处理信号清空，但屏蔽字和处理动作继承
                    signals: SignalFlags::empty(),
                    signal_mask: parent_inner.signal_mask,
                    handling_sig: -1,
                    signal_actions: parent_inner.signal_actions.clone(),
                    killed: false,
                    frozen: false,
                    trap_ctx_backup: None,
                    // ★ fork 继承父进程的文件描述符（共享同一个 OSInode，偏移也共享）
                    fd_table: parent_inner.fd_table.clone(),
                })
            },
        });
        parent_inner.children.push(child.clone());
        // ④ 子进程的代码页是新物理页，必须同步 I-cache（ch4 的坑 2）
        crate::mm::sync_icache_all();
        child
    }

    pub fn getpid(&self) -> usize {
        self.pid.0
    }
}

/// 进程状态（ch5 起：没有 Exited，退出就是 Zombie）
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    Ready,
    Running,
    Zombie,
}
