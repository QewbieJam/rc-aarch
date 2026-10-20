#![no_std]
#![no_main]

//! app_2：**不主动让出**，验证时间片抢占（ch3）
//!
//! 对照 rCore ch3 的 `user/src/bin/02power.rs` 一类的"纯计算"app：
//! 整个 main 里一次 `sys_yield` 都不调，只靠定时器每 10ms 把 CPU 抢走。
//! 如果 ch3 的时间片轮转是对的，它和 app_1 的输出会**交错**出现；
//! 如果抢占没生效，它会把 5 行一口气打印完才轮到别人。

#[macro_use]
extern crate user_lib;

use user_lib::get_time;

#[unsafe(no_mangle)]
fn main() -> i32 {
    for i in 0..5 {
        // 忙等 ~20ms（两个时间片），给定时器足够的机会打断我们
        let t0 = get_time();
        while get_time() - t0 < 20 {
            core::hint::spin_loop();
        }
        println!("[app2] B{}", i);
    }
    0
}
