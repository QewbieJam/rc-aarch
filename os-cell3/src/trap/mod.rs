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
    #[allow(dead_code)]
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
/// ch3：每次 svc 都打印太吵（控制台只有 2KB），前 8 次之后就不再逐条打印
const SVC_LOG_LIMIT: usize = 8;
static mut SVC_LOGGED: usize = 0;

#[unsafe(no_mangle)]
pub extern "C" fn trap_handler(cx: &mut TrapContext) {
    let esr = read_esr_el1();
    let elr = read_elr_el1();
    let far = read_far_el1();
    let spsr = read_spsr_el1();
    let kind = esr >> 26; // ESR_EL1[31:26] = EC

    let log = unsafe {
        let p = core::ptr::addr_of_mut!(SVC_LOGGED);
        let n = core::ptr::read_volatile(p);
        if n < SVC_LOG_LIMIT {
            core::ptr::write_volatile(p, n + 1);
            true
        } else {
            false
        }
    };
    if log {
        crate::println!(
            "[trap] EC={:#04x} ESR={:#018x} ELR={:#x} FAR={:#x} SPSR={:#x} (from EL{})",
            kind,
            esr,
            elr,
            far,
            spsr,
            if spsr & 0xf == 0 { 0 } else { 1 }
        );
    }

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
            if log {
                crate::println!(
                    "[trap]   -> svc from {}: id={} 返回 {} (写回 x0)",
                    from, id, ret
                );
            }
            cx.x[0] = ret as usize;
        }
        ec::DABT_LOW | ec::DABT_CUR => {
            // 对应 rCore 的 StoreFault / StorePageFault（ch2-D 里改成 kill app）
            crate::println!("[trap]   -> Data Abort @ {:#x}", far);
            fault_from(far, spsr);
        }
        ec::IABT_LOW | ec::IABT_CUR => {
            crate::println!("[trap]   -> Instruction Abort @ {:#x}", far);
            fault_from(far, spsr);
        }
        ec::UNKNOWN => {
            // 对应 rCore 的 IllegalInstruction。
            // ★ EL0 执行 hvc 就是这一类（hvc 在 EL0 是未定义指令）
            crate::println!(
                "[trap]   -> Unknown/Illegal instruction @ ELR={:#x} (ch2-D 里改成 kill app)",
                elr
            );
            // ⚠️ 这里**不能**像 RISC-V 那样靠 `+= 4` 越过非法指令来"忽略"它，
            //    因为我们要把 app 杀掉换下一个（见 fault_from）
            fault_from(elr, spsr);
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
#[allow(dead_code)]
pub fn self_test() {
    crate::println!("[trap] self test: 26 个寄存器 svc 前后完整性检查 ...");
    let bad = self_test_svc();
    if bad == 0 {
        crate::println!("[trap] self test PASSED (0 个寄存器被写坏)");
    } else {
        crate::println!("[trap] self test FAILED: {} 个寄存器被写坏", bad);
    }
}

/// 异常发生后的处理：来自 EL0 的异常 → 杀掉当前任务换下一个；
/// 来自 EL1 的异常 → 是内核自己的 bug，停下来别装作没事
///
/// ★ ch3 的变化：upstream 的 trap_handler 在 ch3 里把 `run_next_app()` 改成了
///   `exit_current_and_run_next()`，我们照做 —— ch2 的 `batch::run_next_app`
///   从此不再参与调度（文件留着当对照）。
fn fault_from(addr: usize, spsr: usize) -> ! {
    if spsr & 0xf == 0 {
        crate::println!("[trap] EL0 的 app 异常于 {:#x} → 杀掉它，跑下一个任务", addr);
        crate::task::exit_current_and_run_next();
    } else {
        crate::println!("[trap] 内核自己（EL1）异常于 {:#x} → 停机", addr);
        shutdown();
    }
}

// ---------------------------------------------------------------------------
// ch3：中断（IRQ）路径
// ---------------------------------------------------------------------------

// ★★ ch3 实测的**第四个坑（最隐蔽的一个）**：这个 cell 里**不能用 core::sync::atomic**
//
//   AArch64 在 **stage-1 MMU 关闭**（SCTLR_EL1.M = 0）时，把所有数据访问都当成
//   **Device-nGnRnE** 内存。而架构规定 exclusive 访问（LDXR/STXR 这对指令，
//   也就是 ARM 上 LL/SC 的实现）**不支持 Device 内存** —— STXR 会**永远失败**。
//   Rust 的 `AtomicUsize::fetch_add` 在 aarch64-unknown-none（generic，非 LSE）上
//   正好编译成 `ldxr/stxr; cbnz retry` 循环 → **只要一调用就死循环**
//   （不限于中断上下文，普通 EL1 代码里一样死；我们两种都实测过）。
//
//   现场：控制台停在 "等 tick"，`jailhouse cell list` 仍是 running，
//   hypervisor 计数器（/sys/devices/jailhouse/cells/N/statistics/）完全冻结 ——
//   查了很久才定位到是 fetch_add（详见 docs/03 的「坑 1」）。
//
//   对策：单 CPU（cell 只有 CPU3）+ 中断在 handler 里本来就屏蔽（PSTATE.I=1），
//   所以根本不需要原子性，用 volatile 读写就够。**ch3 及以后都别用 atomic**，
//   等 ch4 打开 MMU、内存变成 Normal 属性之后再说。
use core::ptr::{addr_of, addr_of_mut};

static mut TICKS: usize = 0;
static mut TICKS_FROM_EL0: usize = 0;

#[inline]
fn ticks_load(p: *const usize) -> usize {
    unsafe { core::ptr::read_volatile(p) }
}

#[inline]
fn ticks_add(p: *mut usize) -> usize {
    unsafe {
        let v = core::ptr::read_volatile(p);
        core::ptr::write_volatile(p, v.wrapping_add(1));
        v
    }
}

/// IRQ 分发（由 trap.S 的 irq_handler 调用）
///
/// ★ 与 rCore 的结构差别：rCore 只有 `trap_handler` 一个入口，靠 `scause` 的最高位
///   区分"中断/异常"；AArch64 的向量表已经把 IRQ 和 Sync 分到不同槽位了
///   （#5/#9 vs #4/#8），所以这里是**独立的**处理函数。
///
/// ★★ 必须 ack + EOI：
///   Jailhouse 用 GICH_LR（list register）把物理中断注入成虚拟中断。guest 不读
///   GICC_IAR / 不写 GICC_EOIR，那条 LR 就一直 active，eret 回 EL1 的瞬间同一条
///   中断立刻又进来 → CPU 永远在 handler↔eret 之间打转，主流程一行都跑不到
///   （现象：控制台停在"等 tick"，cell 状态仍是 running）。
#[unsafe(no_mangle)]
pub extern "C" fn irq_dispatch(_cx: &mut TrapContext) {
    let irq = crate::gic::ack();
    if crate::gic::is_spurious(irq) {
        return;
    }
    match irq {
        crate::gic::IRQ_TIMER => {
            // 重装定时器（对照 rCore 的 `set_next_trigger()`）
            crate::timer::set_next_trigger();
            let n = ticks_add(addr_of_mut!(TICKS)) + 1;
            let from_el0 = read_spsr_el1() & 0xf == 0;
            if from_el0 {
                ticks_add(addr_of_mut!(TICKS_FROM_EL0));
            }
            // 控制台只有 2KB：只打第 1 次（以及每 100 次），别把证据挤掉
            if n == 1 || n % 100 == 0 {
                crate::println!(
                    "[irq] 第 {} 次 tick（来自 {}）",
                    n,
                    if from_el0 { "EL0" } else { "EL1" }
                );
            }
        }
        _ => {
            crate::println!("[irq] 未预期的中断号 {}", irq);
        }
    }
    // ★ 必须 EOI（对照 inmate 库的 gic_v2_write_eoi）
    crate::gic::eoi(irq);

    // ★ ch3：时间片到了 —— 当前任务 Running → Ready，换下一个（对照 rCore 在
    //   trap_handler 里 `Trap::Interrupt(SupervisorTimer) => suspend_current_and_run_next()`）
    //   注意 EOI 必须在切换**之前**：先把中断真正撤销，再去跑别的任务。
    if irq == crate::gic::IRQ_TIMER && crate::task::is_ready() {
        crate::task::suspend_current_and_run_next();
    }
}

pub fn ticks() -> usize {
    ticks_load(addr_of!(TICKS))
}

pub fn ticks_from_el0() -> usize {
    ticks_load(addr_of!(TICKS_FROM_EL0))
}

pub fn read_daif_pub() -> usize {
    read_daif()
}

fn read_daif() -> usize {
    let v: usize;
    unsafe { asm!("mrs {}, daif", out(reg) v) };
    v
}

/// ch3-A 诊断：定时器到点没？GIC 里 pending 没？CPU 放行中断没？
#[allow(dead_code)]
pub fn diagnose() {
    println!(
        "[diag] DAIF={:#x}（bit7=I，0=已放行）  CNTV_CTL={:#x}（bit2=ISTATUS）  TVAL={}",
        read_daif(),
        crate::timer::read_ctl(),
        crate::timer::read_tval()
    );
    println!(
        "[diag] GICD: ISENABLER0={:#010x} ISPENDR0={:#010x} IGROUPR0={:#010x}",
        crate::gic::read_isenabler0(),
        crate::gic::read_ispendr0(),
        crate::gic::read_igroupr0()
    );
}

/// 读一次 GICC_IAR（会 ack 掉一个 pending 中断）—— 只在需要时才调
#[allow(dead_code)]
pub fn ack_test() {
    let iar = crate::gic::ack();
    println!("[diag] GICC_IAR → {}（1023=无 pending；27=定时器）", iar);
}

/// ★ ch3 实测教训：**在 cell 里打开了中断/定时器，退出前必须清理**
///
/// 现象（UART 实拍）：cell 被销毁 → CPU3 还给 root cell → Linux `secondary_start_kernel`
/// 把 CPU3 上线 → 在自己的 `el1_irq` 里吃到 **SError（code 0xbf000002）** →
/// `Kernel panic - not syncing: Asynchronous SError Interrupt` → 整板失联（只能断电）。
///
/// 所以：关 CNTV、屏蔽 DAIF.I、关 PPI 27、把 Group 恢复成复位值。
pub fn cleanup_timer() {
    crate::timer::disable();
    unsafe {
        asm!("msr daifset, #2");
    }
    crate::gic::disable(crate::gic::IRQ_TIMER);
    crate::gic::set_group0(crate::gic::IRQ_TIMER);
    println!("[timer] 已清理：CNTV=off + DAIF.I=1 + PPI27 关闭 + Group 恢复 0");
}

/// ch3-A 安全第一步：**不开中断**，纯轮询看三件事：
///   ① 定时器到点了没（CNTV_CTL.ISTATUS）
///   ② GIC 分发器看到 pending 没（GICD_ISPENDR0 bit27）
///   ③ PPI 27 的使能/分组状态
/// 全程 DAIF.I 保持屏蔽 → 不会进中断 handler → 不可能打死 hypervisor
#[allow(dead_code)]
pub fn timer_poll_test() {
    crate::comm_region::dump();
    crate::timer::dump();
    crate::gic::init();
    crate::timer::set_next_trigger();
    for round in 1..=3 {
        let start = crate::timer::get_time();
        while crate::timer::get_time().wrapping_sub(start) < 54_000_000 / 20 {
            core::hint::spin_loop();
        }
        println!("[poll] 第 {} 轮（DAIF.I 仍屏蔽）", round);
        diagnose();
    }
    println!("[poll] 轮询结束 —— 若 ISTATUS=1 且 ISPENDR bit27=1，说明定时器已到点、GIC 已 pending");
}

/// ch3-A：打开定时器中断（GIC 初始化 + 编程 CNTV + 清 DAIF.I）
pub fn enable_timer() {
    crate::comm_region::dump(); // 先看看 hypervisor 给我们填了什么
    crate::timer::dump();
    crate::gic::init();
    crate::timer::set_next_trigger();
    unsafe {
        // DAIF 里只放行 I（IRQ）：bit7 = I
        asm!("msr daifclr, #2");
    }
    println!("[timer] 中断已使能");
}

/// ch3 正式路径：只把 GIC + CNTV 配好，**不在内核里放行 IRQ**
///
/// 为什么要留一手？因为紧接着就要 `__switch` 到任务 0、`eret` 到 EL0。
/// 内核态（EL1）里我们不想被 tick 打断（那会儿 current_task 还没个准），
/// 而 `eret` 到 EL0 时 PSTATE 来自 TrapContext 的 SPSR —— `app_init_context`
/// 里已经把 I 清掉了，所以**到用户态自然就开中断**了。
pub fn init_timer_irq() {
    crate::comm_region::dump();
    crate::timer::dump();
    crate::gic::init();
    crate::timer::set_next_trigger();
    println!("[timer] 定时器已武装；IRQ 在内核态保持屏蔽，eret 到 EL0 后自动放行");
}

/// 停机（裸机 cell 没有关机能力，只能死循环等外部处理）
pub fn shutdown() -> ! {
    crate::println!("[trap] shutdown：进入死循环（ch3 之后可以改成报状态给 hypervisor）");
    infinite_loop()
}

/// ch2-B 自测：内核态发起一次真正的系统调用（`svc`，x8=SYSCALL_WRITE）
#[allow(dead_code)]
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

/// ch2-C 前置探针：EL0 到底能不能跑？（已被真正的 app 取代，留着当工具）
#[allow(dead_code)]
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
