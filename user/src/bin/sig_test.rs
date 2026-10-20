#![no_std]
#![no_main]

//! app：sig_test（ch7）—— 简化版 rCore ch7 `sig_tests`
//!
//! 覆盖四件事：
//!   ① 装 handler → kill 自己 → 进 handler → sigreturn 回到原处继续执行；
//!   ② 屏蔽 SIGUSR1 时 kill 不立即处理，解除屏蔽后补上；
//!   ③ 非法 sigaction（信号号 50）返回 -1；
//!   ④ 子进程被 SIGKILL，父进程 waitpid 拿到 -9。

#[macro_use]
extern crate user_lib;

use user_lib::{SIGKILL, SIGUSR1, SignalAction, exit, fork, getpid, kill, sigaction, sigprocmask, sigreturn, wait, yield_};

/// 信号处理函数：打印一句，然后 sigreturn 回到被中断的现场
fn handler() {
    println!("[sig] ★ 进入信号处理函数");
    sigreturn();
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    let mut act = SignalAction::default();
    let mut old = SignalAction::default();
    act.handler = handler as usize;

    // ① 安装 + 自发自收
    if sigaction(SIGUSR1, Some(&act), Some(&mut old)) < 0 {
        println!("[sig] sigaction 失败");
        return -1;
    }
    if kill(getpid() as usize, SIGUSR1) < 0 {
        println!("[sig] kill 失败");
        return -1;
    }
    println!("[sig] ① handler + sigreturn 正常，继续执行 ✓");

    // ② 屏蔽 → kill → 解除屏蔽后补投
    if sigprocmask(1 << (SIGUSR1 as u32)) < 0 {
        println!("[sig] sigprocmask 失败");
        return -1;
    }
    kill(getpid() as usize, SIGUSR1);
    println!("[sig] ② 屏蔽期间的 kill 已挂起");
    sigprocmask(0);
    println!("[sig] ② 解除屏蔽后 handler 已执行 ✓");

    // ③ 非法信号号
    if sigaction(50, Some(&act), None) >= 0 {
        println!("[sig] 非法 sigaction 居然成功了！");
        return -1;
    }
    println!("[sig] ③ 非法 sigaction 返回 -1 ✓");

    // ④ 子进程被 SIGKILL
    let pid = fork();
    if pid == 0 {
        println!("[sig] 子进程 pid={} 开始空转", getpid());
        loop {
            yield_();
        }
    } else {
        if kill(pid as usize, SIGKILL) < 0 {
            println!("[sig] kill 子进程失败");
            return -1;
        }
        let mut code = 0i32;
        let w = wait(&mut code);
        println!("[sig] ④ 子进程 pid={} 被 SIGKILL，waitpid={} code={}", pid, w, code);
        if code != -9 {
            println!("[sig] 子进程退出码不是 -9！");
            return -1;
        }
    }

    println!("[sig] sig_test passed.");
    0
}
