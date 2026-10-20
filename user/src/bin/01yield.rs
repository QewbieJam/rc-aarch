#![no_std]
#![no_main]

//! app_1：主动让出 CPU（ch3）—— 对照 rCore ch3 的 `user/src/bin/01yield.rs`
//!
//! 每打印一行就 `sys_yield()`，把 CPU 交给下一个任务。
//! 期望和 app_2（不 yield 的那个）交替出现：
//!   [app1] A0 → [app2] B0 → [app1] A1 → ...

#[macro_use]
extern crate user_lib;

use user_lib::yield_;

#[unsafe(no_mangle)]
fn main() -> i32 {
    for i in 0..5 {
        println!("[app1] A{}", i);
        // 让出 CPU：内核把本任务标成 Ready，__switch 到下一个任务
        yield_();
    }
    0
}
