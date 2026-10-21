#![no_std]
#![no_main]

//! app：sync_test（ch8）—— 信号量 + 条件变量
//!
//! 两段验收：
//!   ① 信号量：worker 先 `down` 阻塞，main 稍后 `up` 放行；
//!   ② 条件变量：worker 等条件，main 改条件后 `signal` 唤醒。

#[macro_use]
extern crate user_lib;

use user_lib::{
    condvar_create, condvar_signal, condvar_wait, exit, semaphore_down, semaphore_up,
    semaphore_create, thread_create, waittid, yield_,
};

#[repr(align(16))]
struct Stack([u8; 4096]);

static mut STACKS: [Stack; 2] = [const { Stack([0; 4096]) }; 2];

static mut SEM: isize = 0;
static mut COND: isize = 0;
static mut READY: usize = 0;
static mut WAITED: usize = 0;

extern "C" fn sem_waiter(_arg: usize) -> ! {
    println!("[sem] worker 等信号量…");
    semaphore_down(unsafe { SEM } as usize);
    println!("[sem] worker 被放行 ✓");
    exit(0)
}

extern "C" fn cond_waiter(_arg: usize) -> ! {
    println!("[cond] worker 等条件变量…");
    unsafe { WAITED = 1 };
    condvar_wait(unsafe { COND } as usize);
    println!("[cond] worker 被唤醒 ✓（READY={}）", unsafe { READY });
    exit(0)
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    // ① 信号量
    let sem = semaphore_create(0);
    assert!(sem >= 0, "semaphore_create 失败");
    unsafe { SEM = sem };
    let top = unsafe { (&raw mut STACKS[0]) as *mut u8 as usize + 4096 };
    let tid = thread_create(sem_waiter as usize, 0, top);
    assert!(tid > 0);
    for _ in 0..10 {
        yield_();
    }
    println!("[sem] main 发 up");
    semaphore_up(sem as usize);
    assert_eq!(waittid(tid as usize), 0);
    println!("[sem] 信号量验收通过");

    // ② 条件变量
    let cond = condvar_create();
    assert!(cond >= 0, "condvar_create 失败");
    unsafe { COND = cond };
    let top = unsafe { (&raw mut STACKS[1]) as *mut u8 as usize + 4096 };
    let tid = thread_create(cond_waiter as usize, 0, top);
    assert!(tid > 0);
    for _ in 0..10 {
        yield_();
    }
    unsafe { READY = 1 };
    println!("[cond] main 触发 signal");
    condvar_signal(cond as usize);
    assert_eq!(waittid(tid as usize), 0);
    println!("[cond] 条件变量验收通过");
    println!("[sync] sync_test passed.");
    0
}
