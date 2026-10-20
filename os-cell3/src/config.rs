//! 内核常量（ch3）—— 对照 rCore ch3 的 `os/src/config.rs`
//!
//! 数值和 upstream 一一对应，只有 `APP_BASE_ADDRESS` 不同：
//!   rCore 是 0x80400000（QEMU virt 的 RAM 里挑的一块）
//!   我们是 **0xFA100000** —— cell 配置里 `mem_regions[1]`（31MB RWX）的起点，
//!   和内核镜像所在的 LOADABLE 区（0xFA000000 + 1MB）**严格分开**。
//!
//! ★ 三个地址必须自洽（复习 ch1 的教训）：
//!   链接基址 0xFA000000 == cell 的 virt_start == cpu_reset_address；
//!   app 基址 0xFA100000 落在 cell 的 RAM 区里。
//!   内核 .bss 一旦涨过 1MB 就会撞进 app 区 —— 所以下面这几个栈的大小要心里有数：
//!   16 个内核栈 × 8KB + 16 个用户栈 × 8KB ≈ 256KB，离 1MB 还有富余。

/// 页大小（4KB，和 AArch64 的 4KB granule 一致）
pub const PAGE_SIZE: usize = 0x1000;
/// 页内偏移位数
pub const PAGE_SIZE_BITS: usize = 12;

/// 每个 app 的用户栈（对照 rCore 的 `USER_STACK_SIZE`）
pub const USER_STACK_SIZE: usize = 4096 * 2;
/// 每个 app 的内核栈：trap 现场 + 内核调用链都在这上面（upstream 是 8KB）
pub const KERNEL_STACK_SIZE: usize = 4096 * 2;
/// 最多几个 app（upstream 是 4，我们定 16；受 .bss 大小和 cell RAM 区限制）
pub const MAX_APP_NUM: usize = 16;
/// 物理内存的上界 = cell 的 RAM 区末尾（0xFA100000 + 31MB）。
/// 页帧分配器只发 [ekernel, MEMORY_END) 之间的页。
pub const MEMORY_END: usize = 0xFC000000;
