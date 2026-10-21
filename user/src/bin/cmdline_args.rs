#![no_std]
#![no_main]

//! app：cmdline_args（ch7）—— 对照 rCore ch7 的同名 app
//!
//! 验证 exec 的命令行参数：内核把 argv 压到用户栈，`_start` 解析后传给 main。

#[macro_use]
extern crate user_lib;

#[unsafe(no_mangle)]
pub fn main(argc: usize, argv: &[&str]) -> i32 {
    println!("[cmdline] argc = {}", argc);
    for (i, arg) in argv.iter().enumerate() {
        println!("[cmdline] argv[{}] = {}", i, arg);
    }
    // 简单自检：argv[0] 应该是程序名
    if !argv.is_empty() && argv[0] == "cmdline_args" {
        println!("[cmdline] argv[0] 是程序名 ✓");
    }
    0
}
