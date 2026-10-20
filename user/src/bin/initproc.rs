#![no_std]
#![no_main]

//! app：initproc（ch5）—— 对照 rCore ch5 的 `user/src/bin/initproc.rs`
//!
//! pid 1 的"祖先进程"：所有被它 fork 出来的子进程都在这里收尸。
//! 它退出的时候，内核就知道"系统里没活了"，停机。

#[macro_use]
extern crate user_lib;

use user_lib::{exec, exit, fork, getpid, waitpid};

#[unsafe(no_mangle)]
fn main() -> i32 {
    println!("[initproc] pid={} 启动", getpid());

    // ---- 子进程 1：exec forktest（fork/exit/wait 全家桶）----
    let pid = fork();
    if pid == 0 {
        println!("[initproc] 子进程 exec forktest");
        exec("forktest\0");
        println!("[initproc] exec forktest 失败！");
        exit(-1);
    }
    let mut code = 0i32;
    let w = waitpid(pid as usize, &mut code);
    println!(
        "[initproc] forktest pid={} 结束，exit_code={}（waitpid 返回 {}）",
        pid, code, w
    );

    // ---- 子进程 2：exec 03sbrk（验证 exec + sbrk 组合）----
    let pid2 = fork();
    if pid2 == 0 {
        println!("[initproc] 子进程 exec 03sbrk");
        exec("03sbrk\0");
        exit(-1);
    }
    let w2 = waitpid(pid2 as usize, &mut code);
    println!(
        "[initproc] 03sbrk pid={} 结束，exit_code={}（waitpid 返回 {}）",
        pid2, code, w2
    );

    // ---- 再验证一下"没有子进程时 wait 返回 -1" ----
    let none = waitpid(pid2 as usize, &mut code);
    println!("[initproc] 已经没有子进程了，waitpid 返回 {}", none);

    println!("[initproc] 所有子进程回收完毕，退出");
    0
}
