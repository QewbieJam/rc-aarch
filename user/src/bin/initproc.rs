#![no_std]
#![no_main]

//! app：initproc（ch6）—— 对照 rCore ch6 的 `user/src/bin/initproc.rs`
//!
//! ch5 用它验证进程；ch6 起它还要验证**文件系统**：
//! 现在 `exec` 的 ELF 是从 easy-fs 里读出来的，不再是内核里内嵌的那份。

#[macro_use]
extern crate user_lib;

use user_lib::{exec, exit, fork, getpid, waitpid};

/// 依次跑三个 app：文件读写、cat 文件、进程全家桶
const APPS: [&str; 6] = [
    "filetest_simple\0",
    "cat_filea\0",
    "pipetest\0",
    "pipe_large_test\0",
    "sig_test\0",
    "forktest\0",
];

#[unsafe(no_mangle)]
fn main() -> i32 {
    println!("[initproc] pid={} 启动（exec 的 ELF 来自 easy-fs）", getpid());

    for app in APPS.iter() {
        let pid = fork();
        if pid == 0 {
            println!("[initproc] 子进程 exec {}", app);
            exec(app);
            println!("[initproc] exec {} 失败！", app);
            exit(-1);
        }
        let mut code = 0i32;
        let w = waitpid(pid as usize, &mut code);
        println!(
            "[initproc] 子进程 pid={} 结束，exit_code={}（waitpid 返回 {}）",
            pid, code, w
        );
    }

    println!("[initproc] 所有子进程回收完毕，退出");
    0
}
