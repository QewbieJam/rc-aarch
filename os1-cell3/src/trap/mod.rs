//! Trap 处理（阶段 2 / ch2-A）
//!
//! 对照 rCore ch2 的 `os/src/trap/mod.rs`：
//!   rCore: `init()` 写 stvec；`trap_handler(cx)` 读 scause 分类
//!   我们:  `init()` 写 VBAR_EL1；`trap_handler(cx)` 读 ESR_EL1.EC 分类
//!
//! ★ 结构上最大的差别：
//!   RISC-V 只有一个入口，全靠 `scause` 一个寄存器分辨"是什么事"（最高位=中断）；
//!   AArch64 是**先按落在哪个向量项分类**（同步/IRQ/FIQ/SError × 4 种来源），
//!   再用 `ESR_EL1.EC`（Exception Class）细分**具体原因**。

mod context;

pub use context::TrapContext;

use core::arch::{asm, global_asm};

global_asm!(include_str!("trap.S"));

unsafe extern "C" {
    /// 向量表（trap.S 里定义，linker.ld 保证 2KB 对齐）
    safe fn __vector_table();
    /// 自测：返回"被写坏的寄存器个数"（0 = 全对）
    safe fn self_test_svc() -> usize;
}

/// 初始化：把向量表地址写进 VBAR_EL1（对应 rCore 的 `stvec::write(...)`）
pub fn init() {
    let addr = __vector_table as *const () as usize;
    debug_assert_eq!(addr & 0x7ff, 0, "向量表必须 2KB 对齐");
    unsafe {
        // ★ 写 VBAR_EL1 之后必须 isb，否则后面的指令可能还用旧向量表
        asm!("msr vbar_el1, {addr}", "isb", addr = in(reg) addr);
    }
}

// ---------------------------------------------------------------------------
// 读系统寄存器的小工具（Rust 里没法直接读，必须走 asm）
// ---------------------------------------------------------------------------
fn read_esr_el1() -> usize {
    let v: usize;
    unsafe { asm!("mrs {}, esr_el1", out(reg) v) };
    v
}
fn read_elr_el1() -> usize {
    let v: usize;
    unsafe { asm!("mrs {}, elr_el1", out(reg) v) };
    v
}
fn read_far_el1() -> usize {
    let v: usize;
    unsafe { asm!("mrs {}, far_el1", out(reg) v) };
    v
}
fn read_spsr_el1() -> usize {
    let v: usize;
    unsafe { asm!("mrs {}, spsr_el1", out(reg) v) };
    v
}

/// ESR_EL1 的 Exception Class（对照 rCore 里 match `scause` 的那一堆分支）
#[allow(dead_code)]
mod ec {
    pub const UNKNOWN: usize = 0x00; // 未定义指令等
    pub const ILL: usize = 0x0e; // 非法执行状态
    pub const SVC64: usize = 0x15; // ★ AArch64 的 svc —— 我们的 syscall 入口
    pub const HVC64: usize = 0x16; // 给 hypervisor 用的那条，不该落到这里
    pub const SYS64: usize = 0x18; // mrs/msr 被 trap（ch4 开页表后会遇到）
    pub const IABT_LOW: usize = 0x20; // 取指异常（来自更低 EL）
    pub const IABT_CUR: usize = 0x21; // 取指异常（当前 EL）
    pub const DABT_LOW: usize = 0x24; // 数据访问异常（来自更低 EL）
    pub const DABT_CUR: usize = 0x25; // 数据访问异常（当前 EL）
}

/// 异常处理入口（由 trap.S 的 sync_handler 调用）
///
/// 参数就是我们刚存好的那份现场；改它 = 改"eret 回去时"的寄存器
#[unsafe(no_mangle)]
pub extern "C" fn trap_handler(cx: &mut TrapContext) {
    let esr = read_esr_el1();
    let elr = read_elr_el1();
    let far = read_far_el1();
    let spsr = read_spsr_el1();
    let kind = esr >> 26; // ESR_EL1[31:26] = EC

    crate::println!(
        "[trap] EC={:#04x} ESR={:#018x} ELR={:#x} FAR={:#x} SPSR={:#x} (from EL{})",
        kind,
        esr,
        elr,
        far,
        spsr,
        if spsr & 0xf == 0 { 0 } else { 1 }
    );

    match kind {
        ec::SVC64 => {
            // ★ svc 是 4 字节定长指令：返回时必须 +4，否则 eret 回去又执行一次 svc
            //   （对应 rCore 的 `cx.sepc += 4`）
            cx.elr_el1 += 4;
            let id = cx.x[8]; // 调用号放 x8（Linux ABI 习惯）
            crate::println!("[trap]   -> syscall id={} (ch2-B 实现分发)", id);
        }
        ec::DABT_LOW | ec::DABT_CUR => {
            // 对应 rCore 的 StoreFault / StorePageFault：打印后杀掉 app（ch2-D）
            crate::println!("[trap]   -> Data Abort @ {:#x} (ch2-D 里改成 kill app)", far);
        }
        ec::IABT_LOW | ec::IABT_CUR => {
            crate::println!("[trap]   -> Instruction Abort @ {:#x}", far);
        }
        ec::UNKNOWN => {
            // 对应 rCore 的 IllegalInstruction
            crate::println!("[trap]   -> Unknown/Illegal instruction (ch2-D 里改成 kill app)");
        }
        _ => {
            crate::println!("[trap]   -> 尚未处理，先停在这里");
            infinite_loop();
        }
    }
}

/// 没有被实现的向量项（IRQ/FIQ/SError 等）落这里：打印诊断后死循环
#[unsafe(no_mangle)]
pub extern "C" fn trap_unhandled() -> ! {
    let esr = read_esr_el1();
    let elr = read_elr_el1();
    let far = read_far_el1();
    let spsr = read_spsr_el1();
    crate::println!(
        "[trap] !! unhandled vector: EC={:#04x} ESR={:#018x} ELR={:#x} FAR={:#x} SPSR={:#x}",
        esr >> 26,
        esr,
        elr,
        far,
        spsr
    );
    infinite_loop()
}

/// ch2-A 验收：故意执行一次 `svc`，检查前后寄存器有没有被写坏
pub fn self_test() {
    crate::println!("[trap] self test: 26 个寄存器 svc 前后完整性检查 ...");
    let bad = self_test_svc();
    if bad == 0 {
        crate::println!("[trap] self test PASSED (0 个寄存器被写坏)");
    } else {
        crate::println!("[trap] self test FAILED: {} 个寄存器被写坏", bad);
    }
}

fn infinite_loop() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
