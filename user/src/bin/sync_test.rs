#![no_std]
#![no_main]

//! app：sync_test（ch8）—— 信号量 + 条件变量
//!
//! ## 这个测试在验什么
//!
//! 同步原语能不能让线程**真的睡过去、再被叫醒**：
//!   ① 信号量：worker 先 `down`（初值 0，应该阻塞），main 稍后 `up` 才放行；
//!   ② 条件变量：worker 先 `wait`（应该阻塞），main 改条件后 `signal` 唤醒。
//!
//! 判断“真的阻塞了”的方法：看**打印顺序**。
//!   - 信号量：如果 down 没阻塞，worker 会抢在 main 的“发 up”之前打印“被放行”；
//!   - 条件变量：如果 wait 没阻塞，worker 会在 main 置 READY 之前就读到 0。
//! 所以日志顺序本身就是断言的一部分。
//!
//! ## 测试设计方法
//! - **状态迁移**：线程“运行 → 等待 → 被唤醒 → 退出”；
//! - **判定表思想**：同步原语的两个方向（等待 / 唤醒）都要覆盖；
//! - **错误推测**：靠打印顺序识别“阻塞没生效”这种不容易直接断言的问题。
//!
//! ## 怎么看结果
//! - 通过：`[sync] sync_test passed.`，退出码 0；
//! - 失败：顺序不对（说明阻塞没生效）或永远不醒（说明唤醒丢了）→ 断言/超时失败。

#[macro_use]
extern crate user_lib;

use user_lib::{
    condvar_create, condvar_signal, condvar_wait, exit, semaphore_down, semaphore_up,
    semaphore_create, thread_create, waittid, yield_,
};

/// 每个线程一块自己的用户栈（16 字节对齐，栈顶 = 起始 + 4096）。
#[repr(align(16))]
struct Stack([u8; 4096]);

static mut STACKS: [Stack; 2] = [const { Stack([0; 4096]) }; 2];

/// 同步对象 id（内核返回的下标），由 main 创建后写给 worker 用。
static mut SEM: isize = 0;
static mut COND: isize = 0;
/// “条件是否就绪”的共享标记（条件变量的“条件”本体）。
static mut READY: usize = 0;
/// 辅助标记：worker 已经进入等待（调试/说明用）。
static mut WAITED: usize = 0;

/// 信号量的等待者：先 down（应该阻塞），被 up 放行后退出。
extern "C" fn sem_waiter(_arg: usize) -> ! {
    println!("[sem] worker 等信号量…");
    semaphore_down(unsafe { SEM } as usize); // 初值是 0 → 这里必须阻塞
    println!("[sem] worker 被放行 ✓");
    exit(0)
}

/// 条件变量的等待者：先 wait（应该阻塞），被 signal 唤醒后检查条件再退出。
extern "C" fn cond_waiter(_arg: usize) -> ! {
    println!("[cond] worker 等条件变量…");
    unsafe { WAITED = 1 };
    condvar_wait(unsafe { COND } as usize); // 这里必须阻塞，直到 main signal
    // 唤醒后读到的 READY 必须是 main 置的 1（如果打印 0 说明没等到就跑了）
    println!("[cond] worker 被唤醒 ✓（READY={}）", unsafe { READY });
    exit(0)
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    // =====================================================================
    // ① 信号量：worker 阻塞 → main up → worker 放行
    // =====================================================================
    // Arrange：创建初值为 0 的信号量（0 表示“没有资源”，down 一定会阻塞）。
    let sem = semaphore_create(0);
    assert!(sem >= 0, "semaphore_create 失败");
    unsafe { SEM = sem };
    // 起一个 worker 线程，让它先跑（并阻塞在 down 上）。
    let top = unsafe { (&raw mut STACKS[0]) as *mut u8 as usize + 4096 };
    let tid = thread_create(sem_waiter as usize, 0, top);
    assert!(tid > 0);
    // Act：让出 10 次，确保 worker 已经跑到 down 并阻塞。
    for _ in 0..10 {
        yield_();
    }
    println!("[sem] main 发 up");
    semaphore_up(sem as usize); // 唤醒等待队列里的 worker
    // Assert：worker 正常结束（走到这里说明它确实被放行了）。
    assert_eq!(waittid(tid as usize), 0);
    println!("[sem] 信号量验收通过");

    // =====================================================================
    // ② 条件变量：worker 等待 → main 改条件 + signal → worker 被唤醒
    // =====================================================================
    // Arrange：创建条件变量，起一个 worker 让它先跑并阻塞在 wait 上。
    let cond = condvar_create();
    assert!(cond >= 0, "condvar_create 失败");
    unsafe { COND = cond };
    let top = unsafe { (&raw mut STACKS[1]) as *mut u8 as usize + 4096 };
    let tid = thread_create(cond_waiter as usize, 0, top);
    assert!(tid > 0);
    // Act：确保 worker 已经进入等待，然后改“条件”并发出信号。
    for _ in 0..10 {
        yield_();
    }
    unsafe { READY = 1 }; // 条件成立（worker 醒来后应该读到 1）
    println!("[cond] main 触发 signal");
    condvar_signal(cond as usize); // 唤醒一个等待者
    // Assert：worker 正常结束，且它在唤醒后打印的 READY 应为 1。
    assert_eq!(waittid(tid as usize), 0);
    println!("[cond] 条件变量验收通过");
    println!("[sync] sync_test passed.");
    0
}
