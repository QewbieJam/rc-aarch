//! TrapContext：`trap.S` ↔ Rust 之间的 **ABI**
//!
//! 对照 rCore ch2 的 `os/src/trap/context.rs`（那边是 `x[32] + sstatus + sepc`）。
//!
//! ★ 字段顺序**必须**和 trap.S 里 SAVE_REGS/RESTORE_REGS 的 `stp/ldp` 偏移一一对应，
//!   否则寄存器会被悄悄写坏（程序还照跑 —— 这是最难查的 bug）。
//!
//! 布局（每个 usize = 8 字节，共 272 + 512 + 16 = 800 字节）：
//! ```text
//! 偏移   字段         来源（asm 侧）
//!   0    x[0..31]     x0 .. x30        （★ x31 就是 xzr，不用存）
//! 248    sp_el0       mrs sp_el0       （app 的栈指针，对应 rCore 的 x[2]）
//! 256    elr_el1      mrs elr_el1      （返回地址，对应 rCore 的 sepc）
//! 264    spsr_el1     mrs spsr_el1     （返回时的 PSTATE，对应 rCore 的 sstatus）
//! 272    q[0..32]     q0 .. q31       （SIMD/FP 寄存器，stp q 一次 32 字节）
//! 784    fpcr         mrs fpcr         （FP 控制寄存器）
//! 792    fpsr         mrs fpsr         （FP 状态寄存器）
//! ```
//!
//! ★★ 为什么必须保存 q 寄存器（ch7 实测踩到的大坑）：
//!   AArch64 的 **EL0 和 EL1 共用同一套 SIMD/FP 寄存器**。rCore 跑 RISC-V 时
//!   用户程序根本不用浮点，所以 upstream 的 TrapContext 只存通用寄存器；
//!   但我们的用户程序是 rustc 编出来的 AArch64 代码，LLVM 会**自动向量化**
//!   （neon 在 aarch64-unknown-none 里默认开启），而内核自己也是同一种代码，
//!   `memcpy`/格式化输出都会用 q0~q7。
//!   于是出现这种诡异现象：用户循环把常量先放进 v0~v3、中间调一次 `close()`
//!   → 内核用 SIMD 把 v0 写坏 → 回到用户后整段循环算出全错的数据，而且
//!   **不崩溃、只是数据悄悄错掉**。修法只有两条：要么每次 trap 存/恢复，
//!   要么内核完全禁用 SIMD（做不到，core/compiler_builtins 是预编译的）。
//!   这里选前者，代价是每次 trap 多 528 字节的访存。

/// 目标特权级开关。
/// ★ ch2-C 实测结论：**EL0 可用**（探针见 trap::probe_el0），所以 app 忠实跑 EL0，
///   与 rCore 的 U-mode 语义一致。证据：EC=0x15 from EL0 连续两次往返，
///   SPSR=0x3c0（M=0b0000 = EL0t）。
pub const APP_RETURN_TO_EL0: bool = true;

/// 中断/异常现场（rCore 里叫 TrapContext）
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TrapContext {
    /// x0 ~ x30（x31 是 xzr 无需保存）
    pub x: [usize; 31],
    /// app 的栈指针（EL0 用）
    pub sp_el0: usize,
    /// 返回地址（= rCore 的 sepc）
    pub elr_el1: usize,
    /// 返回时的 PSTATE（= rCore 的 sstatus）
    pub spsr_el1: usize,
    /// SIMD/FP 寄存器 v0~v31（每个 16 字节，用 u128 保证 16 字节对齐）
    pub q: [u128; 32],
    /// 浮点控制寄存器（舍入模式、flush-to-zero 等）
    pub fpcr: u64,
    /// 浮点状态寄存器（异常标志位）
    pub fpsr: u64,
}

#[allow(dead_code)]
impl TrapContext {
    /// 设置栈指针（对应 rCore 的 `set_sp`）
    pub fn set_sp(&mut self, sp: usize) {
        self.sp_el0 = sp;
    }

    /// 构造"要给 app 用的初始现场"（对应 rCore 的 `app_init_context`）
    ///
    /// ⚠️ **不要**在这里 `mrs spsr_el1`：在非 trap 路径下（比如第一次启动 app）
    ///    读到的 SPSR_EL1 是上一次异常留下的**陈旧值**。所以这里从零构造：
    ///    `DAIF` = 屏蔽 D/A/F、**放行 I（IRQ）** + 目标 EL。
    ///    ch3 起必须放行 I，否则 app 在 EL0 永远不会被定时器打断（没法抢占）。
    pub fn app_init_context(entry: usize, sp: usize) -> Self {
        // ch3：app 必须能被定时器抢占 → 不能屏蔽 I（IRQ）
        //   DAIF = D(bit9) A(bit8) I(bit7) F(bit6)
        //   屏蔽 D/A/F、放行 I → 0b1101
        let mut spsr_el1 = 0b1101 << 6;
        // SPSR_EL1.M[3:0]：0b0000 = EL0t，0b0101 = EL1h（用 SP_ELx）
        spsr_el1 |= if APP_RETURN_TO_EL0 { 0b0000 } else { 0b0101 };
        Self {
            x: [0; 31],
            sp_el0: sp,
            elr_el1: entry, // app 入口 = rCore 的 sepc
            spsr_el1,
            q: [0; 32],
            fpcr: 0,
            fpsr: 0,
        }
    }
}
