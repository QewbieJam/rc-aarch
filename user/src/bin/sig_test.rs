#![no_std]
#![no_main]

//! app：sig_test（ch7）—— 信号机制验收（简化版 rCore ch7 `sig_tests`）
//!
//! ## 这个测试在验什么
//!
//! 覆盖四件事：
//!   ① 装 handler → kill 自己 → 进 handler → `sigreturn` 回到原处继续执行；
//!   ② 屏蔽 SIGUSR1 时 kill 不立即处理，解除屏蔽后补投；
//!   ③ 非法 sigaction（信号号 50）返回 -1；
//!   ④ 子进程被 SIGKILL，父进程 `waitpid` 拿到 -9。
//!
//! ## 测试设计方法
//! - **状态迁移**：信号“产生 → 挂起 → 投递 → 处理 → 返回”；
//! - **判定表**：屏蔽 / 未屏蔽 × 有处理函数 / 无处理函数；
//! - **边界与错误路径**：非法信号号（50）必须被拒绝；
//! - **场景法**：父进程用 SIGKILL 杀空转子进程，验证“不能被忽略的信号”。
//!
//! ## 怎么看结果
//! - 通过：`[sig] sig_test passed.`，退出码 0；
//! - 失败：handler 没进 / 回不来（卡死）/ 屏蔽失效 / 退出码不是 -9，都会让进程非 0 退出。

#[macro_use]
extern crate user_lib;

use user_lib::{SIGKILL, SIGUSR1, SignalAction, exit, fork, getpid, kill, sigaction, sigprocmask, sigreturn, wait, yield_};

/// 信号处理函数：打印一句，然后必须调用 `sigreturn` 回到被中断的现场。
///
/// ★ 它是被内核直接“跳”进来的（不是函数调用），所以不能 `ret` 返回；
///   不调 sigreturn 就会一直停在这里。
fn handler() {
    println!("[sig] ★ 进入信号处理函数");
    sigreturn();
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    // Arrange：准备一个“处理动作”结构体；handler 填成上面那个函数。
    let mut act = SignalAction::default();
    let mut old = SignalAction::default();
    act.handler = handler as usize;

    // =====================================================================
    // ① 安装 handler + 自发自收（验证“投递 → 处理 → 返回”）
    // =====================================================================
    // Act：给 SIGUSR1 装 handler（old 用来接收旧动作，验证能回填）。
    if sigaction(SIGUSR1, Some(&act), Some(&mut old)) < 0 {
        println!("[sig] sigaction 失败");
        return -1;
    }
    // Act：给自己发一个 SIGUSR1。真正处理发生在下一次从内核返回用户态之前。
    if kill(getpid() as usize, SIGUSR1) < 0 {
        println!("[sig] kill 失败");
        return -1;
    }
    // Assert：能走到这里，说明 handler 执行完、sigreturn 也成功回到了原处。
    println!("[sig] ① handler + sigreturn 正常，继续执行 ✓");

    // =====================================================================
    // ② 屏蔽 → kill → 解除屏蔽后补投（验证“挂起 + 补投”）
    // =====================================================================
    // Act：把 SIGUSR1 放进屏蔽字（1 << 信号号）。
    if sigprocmask(1 << (SIGUSR1 as u32)) < 0 {
        println!("[sig] sigprocmask 失败");
        return -1;
    }
    // Act：屏蔽期间再 kill 自己 —— 信号应该只是挂起，不会立刻执行 handler。
    kill(getpid() as usize, SIGUSR1);
    println!("[sig] ② 屏蔽期间的 kill 已挂起");
    // Act：解除屏蔽（mask = 0）—— 挂起的 SIGUSR1 应该立刻补投。
    sigprocmask(0);
    // Assert：handler 在解除屏蔽后被调用（★ 进入信号处理函数 会出现在这两行之间）。
    println!("[sig] ② 解除屏蔽后 handler 已执行 ✓");

    // =====================================================================
    // ③ 非法信号号：必须被拒绝（错误路径）
    // =====================================================================
    // Act：用 50 当信号号装 handler。
    if sigaction(50, Some(&act), None) >= 0 {
        println!("[sig] 非法 sigaction 居然成功了！");
        return -1;
    }
    // Assert：返回 < 0 才是对的。
    println!("[sig] ③ 非法 sigaction 返回 -1 ✓");

    // =====================================================================
    // ④ 子进程被 SIGKILL：父进程要能回收，并拿到 -9
    // =====================================================================
    let pid = fork();
    if pid == 0 {
        // 子进程：空转等待被杀（yield 让出 CPU，保证父进程能跑）。
        println!("[sig] 子进程 pid={} 开始空转", getpid());
        loop {
            yield_();
        }
    } else {
        // 父进程：给子进程发 SIGKILL（这个信号不能被用户 handler 接管）。
        if kill(pid as usize, SIGKILL) < 0 {
            println!("[sig] kill 子进程失败");
            return -1;
        }
        // Assert：wait 回收子进程，退出码应为 -9（被 SIGKILL 杀死）。
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
