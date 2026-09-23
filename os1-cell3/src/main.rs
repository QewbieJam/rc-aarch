//! rc-aarch 阶段 1：AArch64 裸机 Hello World
//!
//! 这个文件合并了 rCore ch1 的三个文件（简化版）：
//!   rCore main.rs        → 本文件的 rust_main / clear_bss
//!   rCore console.rs     → 本文件的 uart_putc / print（但不用 SBI，直接写 MMIO）
//!   rCore lang_items.rs  → 本文件的 panic_handler
//!
//! 关键区别（RISC-V → AArch64）：
//!   rCore 打印：  ecall 请求 RustSBI（M-mode 固件）代劳
//!   rc-aarch：    直接写 PL011 UART 寄存器（0x0900_0000），没有固件帮忙

#![no_std]
#![no_main]

use core::arch::global_asm;

// 把启动汇编嵌进二进制（对应 rCore 的 global_asm!(include_str!("entry.asm")))
global_asm!(include_str!("boot.S"));

/// mini-UART（0xFE215040）的数据寄存器（THR）与线路状态寄存器（LSR）
/// 对照 rCore：rCore 用 SBI 的 console_putchar（固件代劳），这里直接写 MMIO
const UART0_DR: *mut u8 = 0xFE215040 as *mut u8;
const UART0_LSR: *const u8 = (0xFE215040usize + 0x14) as *const u8; // 间距 4 字节，LSR=+0x14

/// 输出一个字节（两条通路：① Jailhouse 虚拟控制台 ② mini-UART）
fn uart_putc(c: u8) {
    unsafe {
        // ① hvc：进 hypervisor 控制台（串口 + /sys/devices/jailhouse/console 都能看到）
        core::arch::asm!("hvc #0x4a48", in("x0") 8usize, in("x1") c as usize);
    }
    unsafe {
        // ② mini-UART 直写：★ 必须先等 THRE(bit5) 置位
        //    不等就连续写会丢字符（这正是之前"只看到第一个 A、后面全丢"的原因）
        while core::ptr::read_volatile(UART0_LSR) & 0x20 == 0 {}
        core::ptr::write_volatile(UART0_DR, c);
    }
}

/// 输出字符串（对应 rCore 的 console::print，但没有格式化）
fn print(s: &str) {
    for b in s.bytes() {
        uart_putc(b);
    }
}

// 链接脚本里定义的符号（对应 rCore 的 extern "C" { safe fn sbss(); ... }）
unsafe extern "C" {
    static __bss_start: u8;
    static __bss_end: u8;
}

/// 清零 .bss 段（对应 rCore 的 clear_bss()）
fn clear_bss() {
    let start = core::ptr::addr_of!(__bss_start) as usize;
    let end = core::ptr::addr_of!(__bss_end) as usize;
    for addr in start..end {
        unsafe { (addr as *mut u8).write_volatile(0) }
    }
}

/// Rust 入口（对应 rCore 的 rust_main）
#[unsafe(no_mangle)]
pub extern "C" fn rust_main() -> ! {
    clear_bss();

    print("[rc-aarch] Hello, AArch64!\n");
    print("[rc-aarch] stage 1: boot + linker + bss + UART OK\n");

    loop {
        core::hint::spin_loop();
    }
}

/// panic 兜底（对应 rCore 的 lang_items.rs）
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    print("[rc-aarch] PANIC!\n");
    loop {
        core::hint::spin_loop();
    }
}
