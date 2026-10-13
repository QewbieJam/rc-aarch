//! 控制台驱动（ch2 升级版）
//!
//! 对照 rCore ch2 的 `os/src/console.rs`：
//!   rCore： Stdout.write_str → sbi::console_putchar → ecall 求 RustSBI 代劳
//!   我们：  Stdout.write_str → uart_putc        → ① hvc 给 hypervisor ② 直写 mini-UART
//!
//! ch1 时我们只有一个 `print(&str)`，打不出数字 —— ch2 要打印 `ESR_EL1` / `ELR_EL1`
//! 这类十六进制值，所以这里升级成 rCore 那套 `core::fmt::Write` + `print!/println!`。
//!
//! 用法（和 rCore 完全一样）：
//!     println!("[trap] ESR_EL1={:#x}, ELR_EL1={:#x}", esr, elr);

use core::fmt::{self, Write};

/// mini-UART（0xFE215040）的数据寄存器（THR）与线路状态寄存器（LSR）
/// 对照 rCore：rCore 用 SBI 的 console_putchar（固件代劳），这里直接写 MMIO
const UART0_DR: *mut u8 = 0xFE215040 as *mut u8;
const UART0_LSR: *const u8 = (0xFE215040usize + 0x14) as *const u8; // 间距 4 字节，LSR=+0x14

/// 输出一个字节（两条通路）
pub fn uart_putc(c: u8) {
    unsafe {
        // ① hvc：进 hypervisor 虚拟控制台（串口 + /sys/devices/jailhouse/console 都能看到）
        //    x0 = 8 = JAILHOUSE_HC_DEBUG_CONSOLE_PUTC，x1 = 字符
        core::arch::asm!("hvc #0x4a48", in("x0") 8usize, in("x1") c as usize);
    }
    unsafe {
        // ② mini-UART 直写：★ 必须先等 THRE(bit5) 置位
        //    不等就连续写会丢字符（这正是 ch1 那次"只看到第一个 A、后面全丢"的原因）
        while core::ptr::read_volatile(UART0_LSR) & 0x20 == 0 {}
        core::ptr::write_volatile(UART0_DR, c);
    }
}

/// 输出字符串（无格式化，保留给早期启动/panic 用）
pub fn print_str(s: &str) {
    for b in s.bytes() {
        uart_putc(b);
    }
}

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        print_str(s);
        Ok(())
    }
}

/// 对应 rCore 的 `pub fn print(args: fmt::Arguments)`
pub fn print(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

/// 对应 rCore 的 `#[macro_export] macro_rules! print!`
#[macro_export]
macro_rules! print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!($fmt $(, $($arg)+)?));
    }
}

/// 对应 rCore 的 `#[macro_export] macro_rules! println!`
#[macro_export]
macro_rules! println {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?));
    }
}
