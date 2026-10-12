//! rc-aarch 阶段 2：异常 / 陷入框架（ch2-A）
//!
//! 对照 rCore-Tutorial-v3 ch2 的 `os/src/main.rs`：
//!   rCore:  clear_bss → logging::init → 打印各段 → trap::init() → batch::init() → run_next_app()
//!   我们:  clear_bss → 打印 → trap::init() → svc 自测（batch 留在 ch2-C）
//!
//! 模块划分刻意对齐 upstream，方便逐文件对照：
//!   os/src/console.rs      → src/console.rs        （✅ 已升级为带格式化的 print!/println!）
//!   os/src/trap/trap.S     → src/trap/trap.S       （向量表 + 现场保存/恢复）
//!   os/src/trap/context.rs → src/trap/context.rs   （TrapContext）
//!   os/src/trap/mod.rs     → src/trap/mod.rs       （init + trap_handler）

#![no_std]
#![no_main]

use core::arch::global_asm;

// ★ 顺序很重要：`#[macro_use]` 让 console.rs 里的 print!/println! 在**其后**的模块里可用
//   （rCore ch2 的 main.rs 也是这么写的）
#[macro_use]
mod console;
mod syscall;
mod trap;

// 把启动汇编嵌进二进制（对应 rCore 的 global_asm!(include_str!("entry.asm")))
global_asm!(include_str!("boot.S"));

// 链接脚本里定义的符号（对应 rCore 的 extern "C" { safe fn sbss(); ... }）
unsafe extern "C" {
    static __bss_start: u8;
    static __bss_end: u8;
}

/// 清零 .bss 段（对应 rCore 的 clear_bss()）
fn clear_bss() {
    let start = core::ptr::addr_of!(__bss_start) as usize;
    let end = core::ptr::addr_of!(__bss_end) as usize;
    for addr in start..end {
        unsafe { (addr as *mut u8).write_volatile(0) }
    }
}

/// Rust 入口（对应 rCore 的 rust_main）
#[unsafe(no_mangle)]
pub extern "C" fn rust_main() -> ! {
    clear_bss();

    console::print_str("[rc-aarch] stage 2: trap framework on Jailhouse cell3\n");

    // ① 装向量表：VBAR_EL1 ← __vector_table
    trap::init();

    // ② 自测：svc 触发一次同步异常，检查保存/恢复有没有把寄存器写坏
    trap::self_test();

    println!("[rc-aarch] ch2-A done: svc -> vector#4 -> trap_handler -> eret 全部正常");

    // ③ ch2-B：内核态发起真正的系统调用（svc，x8=调用号）
    trap::syscall_test();

    // ④ ch2-C 前置探针：试试能不能下到 EL0（已实测：能 ✔）
    trap::probe_el0()
}

/// panic 兜底（对应 rCore 的 lang_items.rs）
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[rc-aarch] PANIC: {}", info);
    loop {
        core::hint::spin_loop();
    }
}
