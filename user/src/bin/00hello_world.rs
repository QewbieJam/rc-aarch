#![no_std]
#![no_main]

//! app_0：hello world（ch2-C 第一个跑通的 app）
//! 对照 rCore ch2 的 `user/src/bin/00hello_world.rs`：
//!   RISC-V 版把 "Hello, world!" 交给 SBI 打印；我们交给内核的 sys_write 打印。

#[macro_use]
extern crate user_lib;

/// 每个 app 都必须提供 main（对应 rCore 的 `#[no_mangle] fn main() -> i32`）
#[unsafe(no_mangle)]
fn main() -> i32 {
    println!("Hello, world from EL0 (user mode)!");
    0
}
