//! ARM generic timer —— ch3
//!
//! 对照 rCore ch3 的 `os/src/timer.rs`：那边用 `mtime` + SBI `set_timer`（求 M-mode 固件），
//! 我们直接用 **EL1 的虚拟定时器**（这也是 upstream inmate 库的做法：
//! `inmates/lib/arm-common/timing.c` 里 `timer_start()` 写的就是 `CNTV_TVAL_EL0` + `CNTV_CTL_EL0`）。
//!
//! 寄存器：
//!   CNTFRQ_EL0   系统计数器频率（RPi4 上是 54MHz）
//!   CNTVCT_EL0   虚拟计数器当前值（只读，64 位）
//!   CNTV_TVAL_EL0 倒计时：写进去 N，则 N 个 tick 后触发中断
//!   CNTV_CTL_EL0  bit0=ENABLE, bit1=IMASK, bit2=ISTATUS
//!
//! 中断号：**PPI 27**（虚拟定时器）；物理定时器是 PPI 30，hypervisor 自己用 CNTHP（EL2）。

use core::arch::asm;

/// 与 upstream 一致：每秒 100 次（10ms 一个时间片）
const TICKS_PER_SEC: usize = 100;
const MSEC_PER_SEC: usize = 1000;

fn read_cntfrq() -> usize {
    let v: usize;
    unsafe { asm!("mrs {}, cntfrq_el0", out(reg) v) };
    v
}

/// 读虚拟计数器（对照 rCore 的 `get_time()`，那边读 `mtime`）
pub fn get_time() -> usize {
    let v: usize;
    unsafe { asm!("mrs {}, cntvct_el0", out(reg) v) };
    v
}

/// 毫秒级时间（对照 rCore 的 `get_time_ms()`）
pub fn get_time_ms() -> usize {
    get_time() / (read_cntfrq() / MSEC_PER_SEC)
}

/// 设下一次定时器中断（对照 rCore 的 `set_next_trigger()`）
pub fn set_next_trigger() {
    let ticks = (read_cntfrq() / TICKS_PER_SEC) as u64;
    unsafe {
        asm!("msr cntv_tval_el0, {}", in(reg) ticks); // 倒计时 ticks 个周期
        asm!("msr cntv_ctl_el0, {}", in(reg) 1u64); // ENABLE=1, IMASK=0
    }
}

/// 读 CNTV_CTL_EL0（bit0=ENABLE, bit1=IMASK, bit2=ISTATUS）
#[allow(dead_code)]
pub fn read_ctl() -> u64 {
    let v: u64;
    unsafe { asm!("mrs {}, cntv_ctl_el0", out(reg) v) };
    v
}

/// 读 CNTV_TVAL_EL0（还剩多少 tick）
#[allow(dead_code)]
pub fn read_tval() -> u64 {
    let v: u64;
    unsafe { asm!("mrs {}, cntv_tval_el0", out(reg) v) };
    v
}

/// 关掉定时器（调试用）
#[allow(dead_code)]
pub fn disable() {
    unsafe { asm!("msr cntv_ctl_el0, {}", in(reg) 0u64) };
}

/// 打印一次频率信息（自检）
pub fn dump() {
    crate::println!(
        "[timer] CNTFRQ={} Hz（每 {} 个 tick 中断一次 → {} Hz）",
        read_cntfrq(),
        read_cntfrq() / TICKS_PER_SEC,
        TICKS_PER_SEC
    );
}
