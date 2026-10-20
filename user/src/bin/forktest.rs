#![no_std]
#![no_main]

//! app：forktest（ch5）—— 对照 rCore ch5 的 `user/src/bin/forktest.rs`
//!
//! fork 出 5 个子进程，每个子进程打印后 exit 自己的序号，
//! 父进程逐个 waitpid 回收，最后确认"再 wait 就返回 -1"。

#[macro_use]
extern crate user_lib;

use user_lib::{exit, fork, getpid, wait};

const MAX_CHILD: usize = 5;

#[unsafe(no_mangle)]
fn main() -> i32 {
    println!("[forktest] 父进程 pid={} 开始 fork", getpid());
    for i in 0..MAX_CHILD {
        let pid = fork();
        if pid == 0 {
            // 子进程
            println!("[forktest] 我是第 {} 个孩子，pid={}", i, getpid());
            exit(i as i32);
        } else {
            println!("[forktest] fork 出孩子 pid={}（第 {} 个）", pid, i);
        }
        assert!(pid > 0);
    }

    let mut exit_code: i32 = 0;
    let mut total = 0i32;
    for _ in 0..MAX_CHILD {
        let pid = wait(&mut exit_code);
        assert!(pid > 0, "wait 提前返回了");
        total += exit_code;
        println!("[forktest] 回收 pid={}，exit_code={}", pid, exit_code);
    }
    if wait(&mut exit_code) > 0 {
        panic!("wait 回收到了多余的孩子！");
    }
    println!("[forktest] {} 个孩子全部回收，exit_code 之和 = {}", MAX_CHILD, total);
    println!("[forktest] pass.");
    0
}
