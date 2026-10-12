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
    /// EL0 探针：eret 到 EL0 执行 el0_probe_stub（不返回）
    safe fn enter_el0(entry: usize, stack_top: usize);
    /// EL0 探针里那段小代码（只做一次 svc 然后停住）
    safe fn el0_probe_stub();
    /// 探针用的栈顶（trap.S 的 .bss.stack 里）
    static el0_probe_stack_top: u8;
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
            // ★★ RISC-V→AArch64 移植大坑（本次实测踩到）：
            //   rCore 要写 `cx.sepc += 4`，因为 RISC-V 的 sepc 指向 ecall **本身**；
            //   AArch64 的 ELR_EL1 在 SVC 异常时已经指向 svc 的**下一条指令**，
            //   这里再加 4 就会**跳过一条指令**（我们因此跳过了 stub 里的 `b .`，
            //   跑进后面的代码，最后在 EL0 执行到 hvc → EC=0x00 死循环刷屏）。
            //   ⇒ AArch64 这里**什么都不用改**。
            let from = if spsr & 0xf == 0 { "EL0" } else { "EL1" };
            // ABI：x8 = 调用号，x0/x1/x2 = 参数，返回值写回 x0
            //（对照 rCore：x[17] = 调用号，x[10..12] = 参数，x[10] = 返回值）
            let id = cx.x[8];
            let ret = crate::syscall::syscall(id, [cx.x[0], cx.x[1], cx.x[2]]);
            crate::println!(
                "[trap]   -> svc from {}: id={} 返回 {} (写回 x0)",
                from, id, ret
            );
            cx.x[0] = ret as usize;
        }
        ec::DABT_LOW | ec::DABT_CUR => {
            // 对应 rCore 的 StoreFault / StorePageFault（ch2-D 里改成 kill app）
            crate::println!("[trap]   -> Data Abort @ {:#x} (ch2-D 里改成 kill app)", far);
            infinite_loop();
        }
        ec::IABT_LOW | ec::IABT_CUR => {
            // ★ EL0 探针的关键判据：EL0 若不能取指（Device 内存 + EL0 禁止访问），
            //   就会立刻以 EC=0x20 落在这里，FAR = 那条 EL0 指令的地址
            crate::println!(
                "[trap]   -> Instruction Abort @ {:#x}  ← EL0 取指失败（ch2-C 的 app 只能跑 EL1）",
                far
            );
            infinite_loop(); // ch2-D 里改成 kill app 换下一个
        }
        ec::UNKNOWN => {
            // 对应 rCore 的 IllegalInstruction。
            // ★ EL0 执行 hvc 就是这一类（hvc 在 EL0 是未定义指令）
            crate::println!(
                "[trap]   -> Unknown/Illegal instruction @ ELR={:#x} (ch2-D 里改成 kill app)",
                elr
            );
            // 前进一条指令，避免"同一条非法指令 → 又回来"的死循环刷屏
            cx.elr_el1 += 4;
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

/// ch2-B 自测：内核态发起一次真正的系统调用（`svc`，x8=SYSCALL_WRITE）
pub fn syscall_test() {
    let msg = b"[syscall] sys_write: hello from a syscall!\n";
    let ret: usize;
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") crate::syscall::SYSCALL_WRITE, // 调用号
            in("x0") 1usize,                        // fd = stdout
            in("x1") msg.as_ptr() as usize,         // buf（内核 .rodata）
            in("x2") msg.len(),                     // len
            lateout("x0") ret,                      // 返回值写回 x0
        );
    }
    crate::println!("[syscall] sys_write 返回 {}", ret);

    // 调用号不在表里的情况：应当返回 -1（在 EL0 stub 里测第二个）
    let bad: isize;
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") 0xdeadusize,
            lateout("x0") bad,
        );
    }
    crate::println!("[syscall] 未知调用号返回 {}", bad);
}

/// ch2-C 前置探针：EL0 到底能不能跑？
pub fn probe_el0() -> ! {
    let entry = el0_probe_stub as *const () as usize;
    let stack_top = core::ptr::addr_of!(el0_probe_stack_top) as usize;
    crate::println!("[probe] 准备 eret 到 EL0：入口={:#x} 栈顶={:#x}", entry, stack_top);
    crate::println!("[probe] 如果看到 EC=0x15 说明 EL0 可用；EC=0x20 说明只能跑 EL1");
    enter_el0(entry, stack_top);
    unreachable!()
}

fn infinite_loop() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
