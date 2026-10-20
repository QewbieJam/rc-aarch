//! Jailhouse 通信区（comm region）—— ch3
//!
//! 我们的 cell 配置里声明了一段 4KB 的 comm region（`virt 0xFC000000`），
//! **hypervisor 会往里面填 cell 的运行期信息**（cell 状态、控制台、GIC 基址…）。
//!
//! 对照 upstream 的 inmate 库 `inmates/lib/arm-common/gic-v2.c`：
//! 它也**不是**硬编码 GIC 地址，而是读 `comm_region->gicd_base / gicc_base`：
//!
//! ```c
//! gicc_v2_base = (void*)(unsigned long)comm_region->gicc_base;
//! gicd_v2_base = (void*)(unsigned long)comm_region->gicd_base;
//! ```
//!
//! 结构定义照抄 `include/arch/arm-common/asm/jailhouse_hypercall.h`（arm64 版）+
//! `include/jailhouse/hypercall.h` 的 `COMM_REGION_GENERIC_HEADER` +
//! `include/jailhouse/console.h` 的 `struct jailhouse_console`。
//!
//! ⚠️ C 里是 `__attribute__((packed))`，但我们这里所有字段**恰好都自然对齐**
//!   （u64 在 8 的倍数偏移上），所以用 `#[repr(C)]` 得到的偏移与 C 完全一致，
//!   而且避免了"访问 packed 字段产生未对齐引用"的麻烦。

/// comm region 的虚拟地址（= cell 配置里的 `virt_start`）
const COMM_REGION_BASE: usize = 0xFC000000;

/// 对照 `struct jailhouse_console`（32 字节）
#[repr(C)]
pub struct JailhouseConsole {
    pub address: u64,
    pub size: u32,
    pub console_type: u16,
    pub flags: u16,
    pub divider: u32,
    pub gate_nr: u32,
    pub clock_reg: u64,
}

/// 对照 arm64 的 `struct jailhouse_comm_region`
#[repr(C)]
pub struct CommRegion {
    pub signature: [u8; 6],
    pub revision: u16,
    pub cell_state: u32,
    pub msg_to_cell: u32,
    pub reply_from_cell: u32,
    pub flags: u32,
    pub console: JailhouseConsole,
    pub pci_mmconfig_base: u64,
    pub gic_version: u8,
    pub padding: [u8; 7],
    pub gicd_base: u64,
    pub gicc_base: u64,
    pub gicr_base: u64,
    pub vpci_irq_base: u32,
}

/// 取得 comm region 的只读引用
pub fn comm() -> &'static CommRegion {
    unsafe { &*(COMM_REGION_BASE as *const CommRegion) }
}

/// 打印 comm region 里的关键字段（启动时自检用）
pub fn dump() {
    let c = comm();
    let sig: [u8; 6] = c.signature;
    crate::println!(
        "[comm] sig={}{}{}{}{}{} rev={} cell_state={} gic_version={}",
        sig[0] as char,
        sig[1] as char,
        sig[2] as char,
        sig[3] as char,
        sig[4] as char,
        sig[5] as char,
        c.revision,
        c.cell_state,
        c.gic_version
    );
    crate::println!(
        "[comm] gicd={:#x} gicc={:#x} gicr={:#x} vpci_irq_base={}",
        c.gicd_base,
        c.gicc_base,
        c.gicr_base,
        c.vpci_irq_base
    );
}
