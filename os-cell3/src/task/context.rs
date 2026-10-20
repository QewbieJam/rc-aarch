//! `TaskContext`：**任务切换时真正保存/恢复的那部分寄存器**（ch3）
//!
//! 对照 rCore ch3 的 `os/src/task/context.rs`。
//!
//! ★★★ 最容易搞混的一点：`TaskContext` 和 `TrapContext` 是两个完全不同的东西
//!
//!   |                | TrapContext（trap/context.rs） | TaskContext（这里）            |
//!   |----------------|-------------------------------|-------------------------------|
//!   | 什么时候用      | 用户态 ↔ 内核态（异常/中断）    | 内核态里**任务之间**切换        |
//!   | 存了什么        | x0..x30 + sp_el0 + elr + spsr  | ra + sp + callee-saved 寄存器  |
//!   | 谁保存的        | trap.S 的 SAVE_REGS（硬件入口） | switch.S 的 `__switch`（软件）  |
//!   | 恢复后去哪      | `eret` 回被打断的那一行        | `ret` 回 `__switch` 的下一条   |
//!
//! AArch64 的"callee-saved"和 RISC-V 不一样，这是移植时**必须重算**的地方：
//!   RISC-V: s0~s11（12 个）
//!   AArch64 AAPCS64: x19~x28（10 个）+ x29(fp)，共 11 个；x30(lr) 单独放进 `ra`。
//!   调用者保存的 x0~x18 不用管 —— 编译器在调用 `__switch` 前后会自己处理。
//!
//! 另外 AArch64 还有 d8~d15（浮点/NEON 的 callee-saved），本阶段内核不用浮点，
//! 先不保存；ch4 之后如果内核里出现 f32/f64（例如打印浮点），必须补上。

/// 任务切换上下文（对应 rCore 的 `TaskContext`）
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct TaskContext {
    /// 返回地址（x30/lr）：第一次运行 = `__restore_sp`，之后 = `__switch` 的下一条
    ra: usize,
    /// 内核栈指针（SP_EL1）。第一次运行 = 该任务内核栈上的 TrapContext 地址
    sp: usize,
    /// callee-saved：s[0..10] = x19..x28，s[10] = x29(fp)
    s: [usize; 11],
}

impl TaskContext {
    /// 全零（对照 rCore 的 `zero_init()`）：给 `run_first_task` 里那个"假上下文"用
    pub const fn zero_init() -> Self {
        Self {
            ra: 0,
            sp: 0,
            s: [0; 11],
        }
    }

    /// 构造"第一次被调度"的上下文
    /// （ch3 里叫 `goto_restore`，ch4 起 upstream 改名 `goto_trap_return`，我们跟上）
    ///
    /// 语义：`__switch` 把 sp 换成 `kstack_ptr`、`ret` 跳到 `__restore_sp`，
    /// 而 `__restore_sp` 恰好把"当前的 sp"当成 TrapContext 恢复现场并 `eret` 到 EL0。
    /// 于是任务就从 `app_init_context` 里写好的入口/栈开始跑起来了。
    pub fn goto_trap_return(kstack_ptr: usize) -> Self {
        unsafe extern "C" {
            /// trap.S 里定义：`RESTORE_REGS; eret`
            safe fn __restore_sp();
        }
        Self {
            ra: __restore_sp as *const () as usize,
            sp: kstack_ptr,
            s: [0; 11],
        }
    }
}
