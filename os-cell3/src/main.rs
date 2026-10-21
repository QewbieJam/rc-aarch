//! rc-aarch 阶段 5：进程（fork/exec/waitpid）—— ch5
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

// ★ ch4：页表/MapArea/任务表都要用 Vec、BTreeMap —— `alloc` 是 sysroot 自带的，
//   不算外部依赖；只要提供一个 #[global_allocator]（见 mm/heap_allocator.rs）就能用。
extern crate alloc;

// ★ 顺序很重要：`#[macro_use]` 让 console.rs 里的 print!/println! 在**其后**的模块里可用
#[macro_use]
mod console;
// ch2-C 的批处理：ch3 起不再参与调度，留着当对照（但不参与编译）
mod comm_region;
mod config;
mod fs;
mod gic;
mod loader;
mod mm;
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

    // ① 先装向量表：VBAR_EL1 ← __vector_table
    //   （放在开 MMU 之前：万一页表有洞，异常至少能落到我们自己的 handler 上）
    trap::init();

    // ② ch4：建内核地址空间并打开 stage-1 MMU（恒等映射，地址不变）
    mm::init();
    mm::remap_test();

    // ③ ch6：把内嵌的 app 写进 RAM 磁盘上的 easy-fs，然后挂载
    fs::init();
    println!("[fs] 根目录: {:?}", fs::list_files());

    // ④ ch6 正式流程：进程 + 文件系统
    println!("[rc-aarch] stage 6: 进程 + 文件系统（RAM 磁盘 easy-fs）on Jailhouse cell3");
    {
        let cpacr: usize;
        unsafe { core::arch::asm!("mrs {}, cpacr_el1", out(reg) cpacr) };
        println!("[fpu] CPACR_EL1={:#x} FPEN={}", cpacr, (cpacr >> 20) & 0b11);
    }

    // ⑤ 创建 pid=1 的 initproc，放进就绪队列
    task::add_initproc();

    // ⑥ 配 GIC + CNTV；★ 内核态**不放行** IRQ（到 EL0 后由 SPSR 自动放行）
    trap::init_timer_irq();

    // ⑦ 进入调度主循环：fetch 任务 → __switch 过去；任务切回来就继续 fetch
    task::run_tasks()
}

/// panic 兜底（对应 rCore 的 lang_items.rs）
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[rc-aarch] PANIC: {}", info);
    loop {
        core::hint::spin_loop();
    }
}
