//! rc-aarch 阶段 3：任务切换 + 时间片轮转（ch3）
//!
//! 对照 rCore-Tutorial-v3 ch3 的 `os/src/main.rs`：
//!   rCore:  clear_bss → logging/device_tree → trap::init() → loader::load_apps()
//!           → trap::enable_timer_interrupt() → timer::set_next_trigger()
//!           → task::run_first_task()
//!   我们:  clear_bss → trap::init() → loader::load_apps() → task::init()
//!           → trap::init_timer_irq()（GIC + CNTV）→ task::run_first_task()
//!
//! 模块划分刻意对齐 upstream，方便逐文件对照：
//!   os/src/console.rs        → src/console.rs        （hvc + 直写 mini-UART）
//!   os/src/trap/trap.S       → src/trap/trap.S       （向量表 + 现场保存/恢复 + __restore_sp）
//!   os/src/trap/context.rs   → src/trap/context.rs   （TrapContext）
//!   os/src/trap/mod.rs       → src/trap/mod.rs       （init + trap_handler + irq_dispatch）
//!   os/src/loader.rs         → src/loader.rs         （KernelStack/UserStack + load_apps）
//!   os/src/task/context.rs   → src/task/context.rs   （TaskContext）
//!   os/src/task/switch.S     → src/task/switch.S     （__switch）
//!   os/src/task/switch.rs    → src/task/switch.rs    （global_asm! 包装）
//!   os/src/task/task.rs      → src/task/task.rs      （TaskControlBlock/TaskStatus）
//!   os/src/task/mod.rs       → src/task/mod.rs       （TaskManager + 轮转调度）
//!   os/src/timer.rs          → src/timer.rs          （CNTV 虚拟定时器）
//!   os/src/syscall/process.rs→ src/syscall/process.rs（exit/yield/get_time）
//!
//! ★ 和 rCore 最大的结构差别：**中断是独立入口**。
//!   RISC-V 只有一个 `trap_handler`，靠 scause 最高位分"中断/异常"；
//!   AArch64 的向量表把 IRQ（#5/#9）和同步异常（#4/#8）放在不同槽位，
//!   所以中断走 `irq_dispatch`，系统调用/异常走 `trap_handler`。

#![no_std]
#![no_main]

use core::arch::global_asm;

// ★ 顺序很重要：`#[macro_use]` 让 console.rs 里的 print!/println! 在**其后**的模块里可用
#[macro_use]
mod console;
// ch2-C 的批处理：ch3 起不再参与调度，留着当对照（关闭死代码警告）
#[allow(dead_code)]
mod batch;
mod comm_region;
mod config;
mod gic;
mod loader;
mod sync;
mod syscall;
mod task;
mod timer;
mod trap;

// 把启动汇编嵌进二进制（对应 rCore 的 global_asm!(include_str!("entry.asm")))
global_asm!(include_str!("boot.S"));
// app 的二进制（build.rs 生成；对应 rCore 的 global_asm!(include_str!("link_app.S")))
global_asm!(include_str!("link_app.S"));

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

    // ① 装向量表：VBAR_EL1 ← __vector_table
    trap::init();

    // ★ ch3-A 的定时器回归测试：true 时只验证"IRQ 能进 EL1 并能清理"，不跑任务。
    //   曾经用一个"不 ack/不 EOI"的 minimal handler 调试，会活锁，见 docs/03。
    const CH3_TIMER_TEST: bool = false;
    if CH3_TIMER_TEST {
        console::print_str("[rc-aarch] ch3-A: timer IRQ test（quiet 模式）\n");
        trap::enable_timer();
        println!("[timer] 中断已开，等 tick（最多 300ms）...");
        let t0 = timer::get_time();
        while trap::ticks() < 20 && timer::get_time().wrapping_sub(t0) < 54_000_000 * 3 / 10 {
            core::hint::spin_loop();
        }
        println!(
            "[timer] 结果：{} 次 tick（来自 EL0 的 {} 次）；DAIF={:#x}",
            trap::ticks(),
            trap::ticks_from_el0(),
            trap::read_daif_pub()
        );
        trap::cleanup_timer();
        println!("[timer] ch3-A 通过：IRQ 能投递到 EL1，且板子没挂");
        loop {
            core::hint::spin_loop();
        }
    }

    // ② ch3 正式流程
    println!("[rc-aarch] stage 3: task switch + 时间片轮转 on Jailhouse cell3");

    // ③ 把 app 从内核镜像拷到各自的运行地址（0xFA100000 + i*0x20000）
    loader::load_apps();

    // ④ 建任务表：每个任务一份内核栈 + 一份初始 TrapContext
    task::init();

    // ⑤ 配 GIC + CNTV；★ 内核态**不放行** IRQ（到 EL0 后由 SPSR 自动放行）
    trap::init_timer_irq();

    // ⑥ 换到任务 0，eret 到 EL0 —— 从这里开始就进入"用户态 + 时间片"了
    task::run_first_task()
}

/// panic 兜底（对应 rCore 的 lang_items.rs）
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[rc-aarch] PANIC: {}", info);
    loop {
        core::hint::spin_loop();
    }
}
