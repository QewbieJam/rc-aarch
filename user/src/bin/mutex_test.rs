#![no_std]
#![no_main]

//! app：mutex_test（ch8）—— 互斥锁保护共享计数器
//!
//! 4 个线程各把共享变量加 1000 次；锁住再改，最后必须正好是 4000。

#[macro_use]
extern crate user_lib;

use user_lib::{exit, mutex_create, mutex_lock, mutex_unlock, thread_create, waittid};

#[repr(align(16))]
struct Stack([u8; 4096]);

static mut STACKS: [Stack; 4] = [const { Stack([0; 4096]) }; 4];
static mut SHARED: usize = 0;

extern "C" fn adder(arg: usize) -> ! {
    let m = arg;
    for _ in 0..1000 {
        assert_eq!(mutex_lock(m), 0);
        unsafe { SHARED += 1 };
        assert_eq!(mutex_unlock(m), 0);
    }
    exit(0)
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    let m = mutex_create(true);
    assert!(m >= 0, "mutex_create 失败: {}", m);
    println!("[mutex] 创建互斥锁 id={}", m);
    let mut tids = [0isize; 4];
    for i in 0..4 {
        let top = unsafe { (&raw mut STACKS[i]) as *mut u8 as usize + 4096 };
        tids[i] = thread_create(adder as usize, m as usize, top);
        assert!(tids[i] > 0, "thread_create 失败: {}", tids[i]);
    }
    for i in 0..4 {
        assert_eq!(waittid(tids[i] as usize), 0);
    }
    let total = unsafe { SHARED };
    assert_eq!(total, 4000, "加锁后应为 4000，实际 {}", total);
    println!("[mutex] 4 线程 x 1000 次，SHARED={} ✓", total);
    println!("[mutex] mutex_test passed.");
    0
}
