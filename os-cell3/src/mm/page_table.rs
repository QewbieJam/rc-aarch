//! 页表项 / 页表 / 用户指针翻译（ch4）
//! —— 对照 rCore ch4 的 `os/src/mm/page_table.rs`（那边是 SV39，这里是 VMSAv8-64）
//!
//! ★★ 这是整个 ch4 移植里**最不"逐行对应"**的文件：两套页表格式完全不同。
//!
//! ```text
//!          RISC-V SV39                      AArch64 (4KB granule, 48-bit VA)
//!   索引   VPN[2], VPN[1], VPN[0]（3 级）     L0..L3 共 4 级
//!   有效位 PTE.V (bit 0)                     PTE[0] VALID（表项 0b11 / 无效 0）
//!   地址   PTE[53:10]（44 bit PPN）          PTE[47:12]（36 bit PA，IPS=40 时够用）
//!   权限   R/W/X/U/G/A/D (bit1..7)           AP[2:1] + UXN/PXN + AF，见下
//!   属性   ——                               AttrIndx[2:0] 选 MAIR_EL1 里的内存属性
//!   satp   = 8<<60 | root_ppn               TTBR0_EL1 = root_pa | ASID<<48
//! ```
//!
//! 权限位（stage 1，4KB 页，level 3 页描述符）：
//!
//! | 位     | 名字     | 含义 |
//! |--------|----------|------|
//! | [0]    | VALID    | 1 = 有效 |
//! | [1]    | TABLE    | 1 = 指向下一级页表（对叶子页也必须是 1，合起来 0b11）|
//! | [4:2]  | AttrIndx | 0 = Normal(WB)，1 = Device（由 MAIR_EL1 定义）|
//! | [6]    | AP[1]    | 1 = **EL0 可访问**（用户页的关键位）|
//! | [7]    | AP[2]    | 1 = 只读（EL1/EL0 都只读）|
//! | [9:8]  | SH       | 3 = inner shareable（多核一致性；我们单核，习惯性写上）|
//! | [10]   | AF       | Access Flag，**必须置 1**，否则第一次访问就 fault |
//! | [53]   | PXN      | 1 = EL1 不可执行 |
//! | [54]   | UXN      | 1 = **EL0 不可执行** |
//!
//! AP[2:1] 编码：00 = EL1 RW / EL0 无；01 = EL1 RW / EL0 RW；
//!              10 = EL1 RO / EL0 无；11 = EL1 RO / EL0 RO。
//!
//! ⚠️ 一个容易踩的坑：**EL0 的"能不能执行"只看 UXN，不看 AP**。
//!    也就是说 AP=00（EL0 不能读写）的页，只要 UXN=0，EL0 照样可以跳上去执行。
//!    所以内核页除了 AP 限制外，**还必须**置 UXN=1。

use super::{FrameTracker, PhysAddr, PhysPageNum, StepByOne, VirtAddr, VirtPageNum, frame_alloc};
use alloc::vec;
use alloc::vec::Vec;

/// 页表项标志位（AArch64 描述符的低位 + UXN/PXN 高位，所以用 u64 装）
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct PTEFlags(pub u64);

impl PTEFlags {
    /// 有效位
    pub const VALID: Self = Self(1 << 0);
    /// 指向下一级页表
    pub const TABLE: Self = Self(1 << 1);
    /// Access Flag：不清它，第一次访问就触发 Access Flag Fault
    pub const AF: Self = Self(1 << 10);
    /// inner shareable
    pub const SH_INNER: Self = Self(3 << 8);
    /// MAIR attr 0 = Normal Write-Back
    pub const ATTR_NORMAL: Self = Self(0 << 2);
    /// MAIR attr 1 = Device-nGnRnE
    pub const ATTR_DEVICE: Self = Self(1 << 2);
    /// EL1 读写、EL0 无权限
    pub const AP_EL1_RW: Self = Self(0 << 6);
    /// EL1 读写、**EL0 也可读写**
    pub const AP_EL0_RW: Self = Self(1 << 6);
    /// EL1 只读、EL0 无权限
    pub const AP_EL1_RO: Self = Self(2 << 6);
    /// EL0 不可执行
    pub const UXN: Self = Self(1 << 54);
    /// EL1 不可执行
    pub const PXN: Self = Self(1 << 53);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for PTEFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl core::ops::BitOrAssign for PTEFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// 页表项（对照 upstream 的 `PageTableEntry`）
#[derive(Copy, Clone)]
#[repr(C)]
pub struct PageTableEntry {
    pub bits: u64,
}

impl PageTableEntry {
    /// 叶子页描述符：物理地址 + 权限 + 描述符类型 0b11
    ///
    /// ★ 0b11 = VALID | bit1。在 L3 上 bit1=1 表示"这就是叶子（4KB 页）"；
    ///   在 L0~L2 上 bit1=1 表示"指向下一级页表"。两者描述符头一样，靠层级区分。
    pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        let pa: PhysAddr = ppn.into();
        Self {
            bits: pa.0 as u64 | flags.0 | 0b11,
        }
    }
    /// 指向下一级页表的描述符
    pub fn new_table(ppn: PhysPageNum) -> Self {
        let pa: PhysAddr = ppn.into();
        Self {
            bits: pa.0 as u64 | 0b11,
        }
    }
    pub fn empty() -> Self {
        Self { bits: 0 }
    }

    /// 物理页号（PTE[47:12]）
    pub fn ppn(&self) -> PhysPageNum {
        PhysPageNum(((self.bits >> 12) & ((1u64 << 36) - 1)) as usize)
    }
    pub fn is_valid(&self) -> bool {
        self.bits & 1 != 0
    }
    /// EL0 能不能读写（AP[1]）
    pub fn is_user(&self) -> bool {
        self.bits & (1 << 6) != 0
    }
    pub fn is_writable(&self) -> bool {
        self.bits & (1 << 7) == 0
    }
    /// EL1 能不能执行（PXN=0）
    pub fn executable_at_el1(&self) -> bool {
        self.bits & (1 << 53) == 0
    }
    /// EL0 能不能执行（UXN=0）
    pub fn executable_at_el0(&self) -> bool {
        self.bits & (1 << 54) == 0
    }
}

/// 页表（对照 upstream 的 `PageTable`）
pub struct PageTable {
    root_ppn: PhysPageNum,
    /// 本页表占用的所有物理页（page table 的中间层），Drop 时还回 frame allocator
    frames: Vec<FrameTracker>,
}

impl PageTable {
    /// 新建一张空页表（root 页清零）
    pub fn new() -> Self {
        let frame = frame_alloc().unwrap();
        Self {
            root_ppn: frame.ppn,
            frames: vec![frame],
        }
    }

    /// 用已有的根页号构造（只读遍历用，例如翻译用户指针时）
    pub fn from_token(token: usize) -> Self {
        // token 就是根页表的物理地址（低 48 位；高 16 位是 ASID）
        Self {
            root_ppn: PhysPageNum((token & 0x0000_FFFF_FFFF_F000) >> 12),
            frames: Vec::new(),
        }
    }

    /// 找到 vpn 对应的页表项；路上缺页表就分配（用于 map）
    fn find_pte_create(&mut self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        // ★ 4 级：i == 3 才是叶子
        for (i, idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[*idx];
            if i == 3 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                let frame = frame_alloc().unwrap();
                *pte = PageTableEntry::new_table(frame.ppn);
                self.frames.push(frame);
            }
            ppn = pte.ppn();
        }
        result
    }

    /// 找到 vpn 对应的页表项；中间缺页就直接返回 None（用于查询）
    fn find_pte(&self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for (i, idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[*idx];
            if i == 3 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                return None;
            }
            ppn = pte.ppn();
        }
        result
    }

    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        let pte = self.find_pte_create(vpn).unwrap();
        assert!(!pte.is_valid(), "vpn {:?} is mapped before mapping", vpn);
        *pte = PageTableEntry::new(ppn, flags);
    }

    #[allow(unused)]
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte(vpn).unwrap();
        assert!(pte.is_valid(), "vpn {:?} is invalid before unmapping", vpn);
        *pte = PageTableEntry::empty();
    }

    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_pte(vpn).map(|pte| *pte)
    }

    /// 给 `PageTable::from_token` 用的"令牌" = 根页表物理地址
    pub fn token(&self) -> usize {
        let pa: PhysAddr = self.root_ppn.into();
        pa.0
    }

    /// TTBR0_EL1 的值：TTBR0[63:48] = ASID，[47:1] = 根页表物理地址
    pub fn ttbr0(&self, asid: u16) -> u64 {
        (self.token() as u64) | ((asid as u64) << 48)
    }

    /// 把另一张页表的 L1 表项"共享"过来（不改页表内容，只挂同一个子树）
    ///
    /// 用途：每个任务页表都要能看到内核（0xC0000000~0xFFFFFFFF 全部在 L1[3]）。
    /// 这在 AArch64 上叫"共享页表子树"：任务页表 L1[3] 直接指向内核的 L2 表，
    /// 所以内核改自己的映射时，所有任务自动看到（我们内核映射是静态的，无所谓）。
    pub fn share_l1_entry(&mut self, src: &PageTable, l1_index: usize) {
        // 确保 self 的 L0[0] 存在
        let self_l0 = self.root_ppn.get_pte_array();
        if !self_l0[0].is_valid() {
            let frame = frame_alloc().unwrap();
            self_l0[0] = PageTableEntry::new_table(frame.ppn);
            self.frames.push(frame);
        }
        let src_l1_ppn = src.root_ppn.get_pte_array()[0].ppn();
        let entry = src_l1_ppn.get_pte_array()[l1_index];
        assert!(entry.is_valid(), "内核页表的 L1[{}] 不存在", l1_index);
        self_l0[0].ppn().get_pte_array()[l1_index] = entry;
    }

    /// 把 vpn 的映射删掉（sbrk 收缩时用）
    #[allow(unused)]
    pub fn unmap_range(&mut self, start: VirtPageNum, end: VirtPageNum) {
        let mut vpn = start;
        while vpn < end {
            if self.find_pte(vpn).map(|p| p.is_valid()).unwrap_or(false) {
                self.unmap(vpn);
            }
            vpn.step();
        }
    }
}

/// 把用户指针指向的一段内存翻译成内核能读写的切片（对照 upstream 的同名函数）
///
/// 为什么要这一步？RISC-V/AArch64 的内核和用户用的是**不同的虚拟地址空间**，
/// 用户给的指针在内核里直接解引用会读到别的东西。所以：
///   ① 用当前任务的页表把用户 VA 逐页翻译成物理页；
///   ② 物理页在内核里是恒等映射的，于是可以把「物理页 + 页内偏移」当成内核指针用。
///
/// ★ 跨页：一段 buffer 可能横跨两页，所以返回的是 `Vec<&mut [u8]>`（每页一段）。
/// ★ 安全：这里检查 PTE 的 AP[1]（EL0 可访问），不然用户传一个内核地址就能读内核。
pub fn translated_byte_buffer(token: usize, ptr: *const u8, len: usize) -> Vec<&'static mut [u8]> {
    let page_table = PageTable::from_token(token);
    let mut start = ptr as usize;
    let end = start + len;
    let mut v = Vec::new();
    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = start_va.floor();
        let pte = page_table
            .translate(vpn)
            .unwrap_or_else(|| panic!("用户指针 {:#x} 没有映射（页表 {:?}）", start, vpn));
        assert!(pte.is_user(), "用户指针 {:#x} 指向的是内核页，拒绝访问", start);
        let ppn = pte.ppn();
        vpn.step();
        let mut end_va: VirtAddr = vpn.into();
        end_va = core::cmp::min(end_va, VirtAddr::from(end));
        if end_va.page_offset() == 0 {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..]);
        } else {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..end_va.page_offset()]);
        }
        start = end_va.0;
    }
    v
}
