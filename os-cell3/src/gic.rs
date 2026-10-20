//! GICv2（树莓派是 GIC-400）—— ch3
//!
//! 对照 upstream 的 inmate 库 `inmates/lib/arm-common/gic-v2.c`（写法几乎一一对应）：
//!
//! ```c
//! static int gic_v2_init(void) {
//!     gicc_v2_base = (void*)(unsigned long)comm_region->gicc_base;
//!     gicd_v2_base = (void*)(unsigned long)comm_region->gicd_base;
//!     mmio_write32(gicc_v2_base + GICC_CTLR, GICC_CTLR_GRPEN1);   // = 1
//!     mmio_write32(gicc_v2_base + GICC_PMR,  GICC_PMR_DEFAULT);   // = 0xf0
//!     mmio_write32(gicd_v2_base + GICD_CTLR, GICD_CTLR_ENABLE);   // = 1
//! }
//! static void gic_v2_enable(unsigned int irqn) {
//!     mmio_write32(gicd_v2_base + GICD_ISENABLER + ((irqn >> 3) & ~0x3), 1 << (irqn & 0x1f));
//! }
//! static u32  gic_v2_read_ack(void) { return mmio_read32(gicc_v2_base + GICC_IAR) & 0x3ff; }
//! static void gic_v2_write_eoi(u32 irqn) { mmio_write32(gicc_v2_base + GICC_EOIR, irqn); }
//! ```
//!
//! ★ 在 Jailhouse 里中断是这样到我们手上的：
//!   物理中断先到 EL2（`HCR_EL2.IMO=1`）→ hypervisor 查 GICD 判断归哪个 cell →
//!   用 **GICv2 的 list register（`GICH_LR`）注入成虚拟中断** → 我们的 `VBAR_EL1` IRQ 项被触发。
//!   我们只需要像普通 GIC 驱动那样：**使能 IRQ → 读 IAR 拿 INTID → 处理 → 写 EOIR**。

use crate::comm_region::comm;

// 分发器（GICD）寄存器偏移
const GICD_CTLR: usize = 0x0000;
const GICD_ISENABLER: usize = 0x0100;

// CPU 接口（GICC）寄存器偏移
const GICC_CTLR: usize = 0x0000;
const GICC_PMR: usize = 0x0004;
const GICC_IAR: usize = 0x000C;
const GICC_EOIR: usize = 0x0010;

/// 虚拟定时器（CNTV）的 PPI 号 —— 见 `timer.rs`
pub const IRQ_TIMER: u32 = 27;
/// GIC 用 1023 表示"没有中断可确认"（spurious）
const IRQ_SPURIOUS: u32 = 1023;

fn gicd() -> usize {
    comm().gicd_base as usize
}
fn gicc() -> usize {
    comm().gicc_base as usize
}

fn wr(addr: usize, v: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, v) }
}
fn rd(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

/// 把某个中断分到 **Group 1**（写 GICD_IGROUPRn）
///
/// ★ ch3 实测踩到的坑：复位后所有中断都是 **Group 0**，而 Jailhouse 的 CPU 接口
///   只使能了 Group 1（`gicv2_cpu_init()`：`GICC_CTLR = GRPEN1|EOImode`）→
///   物理中断会**一直 pending 在分发器里没人取**（实测 `GICD_ISPENDR0` bit27 恒为 1，
///   而 CNTV 已经到点、GICD 也使能了）。
///   把中断改到 Group 1 后，hypervisor 才能收到它并注入给我们。
pub fn set_group1(irq: u32) {
    let off = 0x0080 + ((irq as usize >> 5) * 4); // GICD_IGROUPRn
    let bit = 1u32 << (irq & 0x1f);
    let cur = rd(gicd() + off);
    wr(gicd() + off, cur | bit); // 读-改-写（hypervisor 会按 cell 的 irq_bitmap 掩码）
}

/// 初始化 CPU 接口 + 分发器，并打开定时器 PPI
pub fn init() {
    crate::println!(
        "[gic] gicd={:#x} gicc={:#x} —— 初始化（CTLR/PMR/ISENABLER）",
        gicd(),
        gicc()
    );
    wr(gicc() + GICC_CTLR, 1); // 打开 group1（= inmate 库的 GICC_CTLR_GRPEN1）
    wr(gicc() + GICC_PMR, 0xF0); // 放行所有优先级（= GICC_PMR_DEFAULT）
    wr(gicd() + GICD_CTLR, 1); // 分发器总开关
    enable(IRQ_TIMER); // 使能 PPI 27
    set_group1(IRQ_TIMER); // ★ 并把它分到 Group 1（否则 hypervisor 收不到）
}

/// 使能某个中断（对照 inmate 库的 `gic_v2_enable`）
pub fn enable(irq: u32) {
    let off = GICD_ISENABLER + ((irq as usize >> 3) & !0x3);
    wr(gicd() + off, 1 << (irq & 0x1f));
}

/// 读 IAR 拿中断号（对照 `gic_v2_read_ack`）
pub fn ack() -> u32 {
    rd(gicc() + GICC_IAR) & 0x3ff
}

/// 写 EOIR 结束中断（对照 `gic_v2_write_eoi`）
pub fn eoi(irq: u32) {
    wr(gicc() + GICC_EOIR, irq);
}

/// 关闭某个中断（写 GICD_ICENABLERn）—— 退出 cell 前清理用
pub fn disable(irq: u32) {
    let off = 0x0180 + ((irq as usize >> 5) * 4); // GICD_ICENABLERn
    wr(gicd() + off, 1 << (irq & 0x1f));
}

/// 把某个中断恢复成 Group 0（复位值）—— 退出 cell 前清理用
pub fn set_group0(irq: u32) {
    let off = 0x0080 + ((irq as usize >> 5) * 4);
    let cur = rd(gicd() + off);
    wr(gicd() + off, cur & !(1u32 << (irq & 0x1f)));
}

/// 读 GICD_ISENABLER0（PPI/SGI 的使能位）—— 调试用
#[allow(dead_code)]
pub fn read_isenabler0() -> u32 {
    rd(gicd() + GICD_ISENABLER)
}

/// 读 GICD_ISPENDR0（PPI/SGI 的 pending 位）—— 调试用
#[allow(dead_code)]
pub fn read_ispendr0() -> u32 {
    rd(gicd() + 0x0200)
}

/// 读 GICD_IGROUPR0（PPI/SGI 的分组）—— 调试用
#[allow(dead_code)]
pub fn read_igroupr0() -> u32 {
    rd(gicd() + 0x0080)
}

pub fn is_spurious(irq: u32) -> bool {
    irq >= IRQ_SPURIOUS
}
