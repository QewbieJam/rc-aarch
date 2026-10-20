//! 内存管理（ch4）—— 对照 rCore ch4 的 `os/src/mm/mod.rs`
//!
//! 结构一一对应：
//!   mm/address.rs        地址/页号类型（VA 48 位、PA 40 位、4 级页表索引）
//!   mm/frame_allocator.rs 物理页帧分配器（栈式 + 回收栈）
//!   mm/heap_allocator.rs  内核堆（我们用 bump，upstream 用 buddy）
//!   mm/page_table.rs      页表项/页表/用户指针翻译（AArch64 描述符格式）
//!   mm/memory_set.rs      MapArea / MemorySet / 内核地址空间 / 从 ELF 建用户空间
//!
//! ★ AArch64 的"额外功课"：**打开 stage-1 MMU**
//!   RISC-V 只要写 `satp` 就生效，AArch64 还要配 MAIR/TCR/SCTLR 三件套：
//!     - MAIR_EL1：内存属性表（attr0 = Normal WB，attr1 = Device-nGnRnE）
//!     - TCR_EL1 ：地址空间大小/粒度/IPS（T0SZ=16 → 48 位 VA，TG0=4KB）
//!     - SCTLR_EL1：M=1 开 MMU、C=1 开 D-cache、I=1 开 I-cache
//!   这些寄存器都在 EL1，Jailhouse 不会拦（它只关心 stage-2）。

mod address;
mod frame_allocator;
mod heap_allocator;
mod memory_set;
mod page_table;

pub use address::{PhysAddr, PhysPageNum, StepByOne, VPNRange, VirtAddr, VirtPageNum};
pub use frame_allocator::{
    FrameBuf, FrameTracker, frame_alloc, frame_alloc_contig, frame_dealloc_range, frame_stats,
};
pub use memory_set::{MemorySet, kernel_token, remap_test};
pub use page_table::{
    PTEFlags, PageTable, PageTableEntry, UserBuffer, translated_byte_buffer, translated_refmut,
    translated_str,
};

/// 初始化堆 → 页帧分配器 → 内核地址空间 → 打开 MMU
/// （对照 upstream 的 `mm::init`，只是多了最后一步）
pub fn init() {
    heap_allocator::init_heap();
    frame_allocator::init_frame_allocator();
    memory_set::init_kernel_space();
    enable_mmu();
    crate::println!("[mm] stage-1 MMU 已打开（恒等映射，TTBR0）");
}

/// 用某个任务的页表读一个用户 VA 处的 u32（调试用：看崩溃指令到底是什么）
pub fn read_user_u32(token: usize, va: usize) -> Option<u32> {
    let page_table = PageTable::from_token(token);
    let vpn = VirtAddr::from(va).floor();
    let pte = page_table.translate(vpn)?;
    if !pte.is_valid() {
        return None;
    }
    let off = va & 0xfff;
    let bytes = pte.ppn().get_bytes_array();
    Some(u32::from_le_bytes([
        bytes[off],
        bytes[off + 1],
        bytes[off + 2],
        bytes[off + 3],
    ]))
}

/// 内核（以及 MMIO）在页表里的 L1 下标
///
/// 0xFA000000 / 0xFC000000 / 0xFE215040 / 0xFF841000 全都落在
/// `0xC0000000 ~ 0xFFFFFFFF` 这 1GB 里 → L1 index = (VA >> 30) & 0x1FF = 3。
pub const KERNEL_L1_INDEX: usize = 3;

/// 把一段地址的 D-cache 逐行清到 PoU（写代码之后、执行之前必须做）
///
/// ★★ 这里踩过一个坑：`DC CVAU` 一次只清**一条 cache line**（Cortex-A72 是 64 字节），
///   不是一整个页！最初只对页首地址调了一次 `dc cvau`，结果 4KB 里只有头 64 字节
///   被写回内存，后面的代码 I-fetch 读到的是**旧内容/零** → EC=0x00 "Unknown"
///   （表现为 app 跑到某个函数开头就报未定义指令，而反汇编看那条指令完全正常）。
///   所以必须按 64 字节步进，把整段都清一遍。
pub fn clean_dcache_range_to_pou(start: usize, len: usize) {
    // CTR_EL0.DminLine 在 A72 上是 64B；这里按 64 对齐向下扫，宁多勿漏
    let mut va = start & !0x3f;
    let end = start + len;
    while va < end {
        unsafe {
            core::arch::asm!("dc cvau, {v}", v = in(reg) va);
        }
        va += 64;
    }
}

/// 全部同步：等 D-cache 清理落地 + 清空 I-cache（任务建好之后调一次）
pub fn sync_icache_all() {
    unsafe {
        core::arch::asm!("dsb ish", "ic iallu", "dsb ish", "isb");
    }
}

/// MAIR_EL1：内存属性表
///
///   Attr0 = 0xFF → Normal, Outer/Inner Write-Back, Read/Write-Allocate
///   Attr1 = 0x00 → Device-nGnRnE（MMIO 用；和 Linux 的 ioremap 默认一致）
const MAIR_VALUE: u64 = 0x00FF;

/// TCR_EL1：翻译控制
///
/// ```text
/// T0SZ=16  → 低半区 48 位 VA（用户空间在 TTBR0）
/// TG0=4KB  → 4KB 粒度
/// IRGN0/ORGN0=WBWA, SH0=inner shareable
/// T1SZ=16, TG1=4KB → 高半区同样 48 位（本阶段没用 TTBR1，但把参数配好）
/// EPD1=1   → 关闭 TTBR1 的页表遍历（我们不用高半区，避免误走）
/// IPS=0b010→ 物理地址 40 位（覆盖 0xFA000000~0xFC000000）
/// ```
fn tcr_value() -> u64 {
    let t0sz = 16u64;
    let t1sz = 16u64;
    t0sz
        | (1 << 8)          // IRGN0 = WBWA
        | (1 << 10)         // ORGN0 = WBWA
        | (3 << 12)         // SH0 = inner shareable
        | (0 << 14)         // TG0 = 4KB
        | (t1sz << 16)
        | (1 << 23)         // EPD1 = 1（不走 TTBR1）
        | (1 << 24)         // IRGN1
        | (1 << 26)         // ORGN1
        | (3 << 28)         // SH1
        | (2 << 30)         // TG1 = 4KB
        | (2 << 32)         // IPS = 40 bit
}

/// SCTLR_EL1 的"推荐值"
///
/// 0x30D00800 = 那些 RES1 位（11/20/22/23/28/29），ARM 手册要求写 1；
/// 再或上 M(0)=MMU、C(2)=D-cache、I(12)=I-cache。
const SCTLR_VALUE: u64 = 0x30D0_0800 | 1 | (1 << 2) | (1 << 12);

/// 打开 stage-1 MMU（内核已经在 `new_kernel()` 里建好恒等映射）
fn enable_mmu() {
    let ttbr0 = kernel_token() as u64;
    unsafe {
        core::arch::asm!(
            "msr mair_el1, {mair}",
            "msr tcr_el1,  {tcr}",
            "msr ttbr0_el1, {ttbr0}",
            "msr ttbr1_el1, xzr",
            mair = in(reg) MAIR_VALUE,
            tcr = in(reg) tcr_value(),
            ttbr0 = in(reg) ttbr0,
        );
        // 改页表基址/属性之后必须同步，再清掉可能存在的旧 TLB/ICache 项
        core::arch::asm!(
            "dsb ish",
            "ic iallu",
            "tlbi vmalle1",
            "dsb ish",
            "isb",
        );
        // ★ 真正打开 MMU 的那条 msr：前后 PC 是连续的（恒等映射），所以不用跳板
        core::arch::asm!(
            "msr sctlr_el1, {sctlr}",
            "isb",
            sctlr = in(reg) SCTLR_VALUE,
        );
    }
}
