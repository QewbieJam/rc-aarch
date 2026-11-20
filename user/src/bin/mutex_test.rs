#![no_std]
#![no_main]

//! app：mutex_test（ch8）—— 互斥锁保护共享计数器
//!
//! ## 这个测试在验什么
//!
//! 4 个线程各把同一个共享变量加 1000 次；每次“加”都在锁里做。
//! 如果锁是正确的，最后必须**正好**是 4000；少一个都说明有竞态（两个线程同时改）。
//!
//! ## 测试设计方法
//! - **并发组合**：线程数 4 × 每线程 1000 次，放大竞争窗口；
//! - **判定表思想**：lock 成功 / 失败两条路径都断言；
//! - **错误推测**：不加锁时结果必然小于 4000，用“精确等于 4000”当判据。
//!
//! ## 怎么看结果
//! - 通过：`[mutex] mutex_test passed.`，退出码 0；
//! - 失败：`assert_eq!(total, 4000)` 触发 panic → 非 0 退出，验收脚本判 FAIL。

#[macro_use]
extern crate user_lib;

use user_lib::{exit, mutex_create, mutex_lock, mutex_unlock, thread_create, waittid};

/// 每个线程一块自己的用户栈（16 字节对齐）。
#[repr(align(16))]
struct Stack([u8; 4096]);

static mut STACKS: [Stack; 4] = [const { Stack([0; 4096]) }; 4];
/// 被 4 个线程共同修改的共享计数器（同一个进程的地址空间，所以直接可见）。
static mut SHARED: usize = 0;

/// 线程入口：拿锁 → 自增 → 放锁，重复 1000 次。
///
/// `arg` 是互斥锁的 id（main 创建后传进来）。
extern "C" fn adder(arg: usize) -> ! {
    let m = arg;
    for _ in 0..1000 {
        // Act：加锁（blocking=true，抢不到就睡，不返回失败）。
        assert_eq!(mutex_lock(m), 0);
        // 临界区：只有持有锁的线程能改 SHARED。
        unsafe { SHARED += 1 };
        // Act：放锁（会唤醒一个等待者）。
        assert_eq!(mutex_unlock(m), 0);
    }
    exit(0)
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    // Arrange：创建阻塞式互斥锁，id 作为参数传给每个线程。
    let m = mutex_create(true);
    assert!(m >= 0, "mutex_create 失败: {}", m);
    println!("[mutex] 创建互斥锁 id={}", m);

    // Act：起 4 个线程，每个跑 1000 次“锁内自增”。
    let mut tids = [0isize; 4];
    for i in 0..4 {
        let top = unsafe { (&raw mut STACKS[i]) as *mut u8 as usize + 4096 };
        tids[i] = thread_create(adder as usize, m as usize, top);
        assert!(tids[i] > 0, "thread_create 失败: {}", tids[i]);
    }

    // 清理：等 4 个线程全部正常退出。
    for i in 0..4 {
        assert_eq!(waittid(tids[i] as usize), 0);
    }

    // Assert：4 × 1000 = 4000，一个都不能少。
    let total = unsafe { SHARED };
    assert_eq!(total, 4000, "加锁后应为 4000，实际 {}", total);
    println!("[mutex] 4 线程 x 1000 次，SHARED={} ✓", total);
    println!("[mutex] mutex_test passed.");
    0
}
