//! 语言项（ch2-C）—— 对照 rCore ch2 的 `user/src/lang_items.rs`
//!
//! 差别：上游 panic 后 `loop {}`（在有定时器抢占的章节里会被换下去）；
//! 我们 ch2 的批处理是"app 自己说退出才换下一个"，所以 panic 后直接
//! `sys_exit(127)`，让内核把这次崩溃当成 app 退出、切下一个 app。

use crate::exit;

#[panic_handler]
fn panic_handler(panic_info: &core::panic::PanicInfo) -> ! {
    crate::println!("[app] Panicked: {}", panic_info);
    exit(127);
    loop {
        core::hint::spin_loop();
    }
}
