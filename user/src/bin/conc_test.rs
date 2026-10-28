#![no_std]
#![no_main]

//! app：conc_test（ch3 验证用）—— 同时跑两个 app，看 yield/时间片是否交错
//!
//! initproc 平时是"一次跑一个 app"，那样看不到 ch3 的任务交替。
//! 这里自己 fork 两个子进程分别 exec 01yield 和 02busy，然后等它们结束。

#[macro_use]
extern crate user_lib;

use user_lib::{exec, exit, fork, waitpid};

#[unsafe(no_mangle)]
fn main() -> i32 {
    println!("[ch3] 并发跑 01yield + 02busy（验证 yield 与时间片）");
    let p1 = fork();
    if p1 == 0 {
        exec("01yield\0", &["01yield\0"]);
        exit(-1);
    }
    let p2 = fork();
    if p2 == 0 {
        exec("02busy\0", &["02busy\0"]);
        exit(-1);
    }
    let mut c1 = 0i32;
    let mut c2 = 0i32;
    waitpid(p1 as usize, &mut c1);
    waitpid(p2 as usize, &mut c2);
    println!("[ch3] 两个子进程回收完毕（exit {} / {}）", c1, c2);
    0
}
