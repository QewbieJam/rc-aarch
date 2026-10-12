//! 用户态控制台（ch2-C）—— 对照 rCore ch2 的 `user/src/console.rs`
//!
//! 用户程序**不能**直接写 UART（那是内核/设备的特权），
//! 只能通过 `sys_write` 系统调用把字节交给内核去打印。

use core::fmt::{self, Write};

use crate::write;

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write(1, s.as_bytes());
        Ok(())
    }
}

/// 对应 rCore 的 `pub fn print(args: fmt::Arguments)`
pub fn print(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!($fmt $(, $($arg)+)?));
    }
}

#[macro_export]
macro_rules! println {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?));
    }
}
