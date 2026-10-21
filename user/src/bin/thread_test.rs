#![no_std]
#![no_main]

//! app：thread_test（ch8）—— 线程创建/参数/tid/waittid
//!
//! 和 rCore ch8 的 `threads.rs`/`threads_arg.rs` 同一个意图，但我们的
//! `thread_create` 多一个 `stack_top` 参数（每个线程一块自己的用户栈）。

#[macro_use]
extern crate user_lib;

use user_lib::{exit, getpid, gettid, thread_create, waittid};

#[repr(align(16))]
struct Stack([u8; 4096]);

static mut STACKS: [Stack; 3] = [const { Stack([0; 4096]) }; 3];
static mut COUNTER: usize = 0;

extern "C" fn worker(arg: usize) -> ! {
    let tid = gettid();
    for _ in 0..100 {
        unsafe { COUNTER += 1 };
    }
    println!("[thread] tid={} arg={} 干完了", tid, arg);
    exit((arg + 1) as i32)
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    let pid = getpid();
    println!("[thread] 主线程 pid={} tid={}", pid, gettid());
    let mut tids = [0isize; 3];
    for i in 0..3 {
        let top = unsafe { (&raw mut STACKS[i]) as *mut u8 as usize + 4096 };
        tids[i] = thread_create(worker as usize, i, top);
        println!("[thread] 创建线程 tid={} (arg={})", tids[i], i);
        assert!(tids[i] > 0, "thread_create 失败: {}", tids[i]);
    }
    for i in 0..3 {
        let code = waittid(tids[i] as usize);
        println!("[thread] 回收 tid={} exit_code={}", tids[i], code);
        assert_eq!(code, (i + 1) as isize, "waittid 返回值不对");
    }
    let total = unsafe { COUNTER };
    assert_eq!(total, 300, "COUNTER 应该是 300，实际 {}", total);
    println!("[thread] 3 个线程共自增 {} 次 ✓", total);
    println!("[thread] thread_test passed.");
    0
}
