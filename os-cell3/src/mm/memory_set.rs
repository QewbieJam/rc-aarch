//! `MapArea` / `MemorySet`（ch4）—— 对照 rCore ch4 的 `os/src/mm/memory_set.rs`
//!
//! 概念和 upstream 完全一样：
//!   - `MapArea`  = 一段**连续虚拟页** + 映射方式（Identical 恒等 / Framed 新分配页帧）
//!   - `MemorySet` = 一个任务的整套地址空间（一张页表 + 若干 MapArea）
//!
//! ★★ AArch64 与 RISC-V 最大的结构性差异：**不需要 trampoline**
//!
//!   RISC-V 只有一个 `satp`，用户态和内核态共用它。陷阱进来时还跑在用户页表上，
//!   所以必须把一小段 `__alltraps` 代码映射到用户空间的最高页（TRAMPOLINE），
//!   在那边"先切 satp 再跳内核"。用户栈顶的 TRAP_CONTEXT 页也是这么来的。
//!
//!   AArch64 有 **TTBR0（低半区，用户）和 TTBR1（高半区，内核）两套基址**。
//!   我们这里只用了 TTBR0，但把内核的恒等映射挂进了**每个任务页表的 L1[3]**
//!   （0xC0000000~0xFFFFFFFF 这一段，内核/MMIO 全在里面）——
//!   于是陷阱进来时**内核代码和数据本来就是可访问的**，`sp` 也还是本任务的内核栈，
//!   完全不用切页表、不用 trampoline，TrapContext 也就留在内核栈上。
//!
//!   代价：内核和用户共享同一个低半区地址空间，用户 VA 只能落在
//!   0x0 ~ 0xC0000000 里（我们给 app 用 0x10000 起），并且内核页必须
//!   置 `UXN=1`（否则 EL0 可以直接跳上去执行——见 page_table.rs 的提醒）。

use super::{
    FrameTracker, PhysAddr, PhysPageNum, PTEFlags, PageTable, PageTableEntry, VirtAddr, VirtPageNum,
};
use super::{StepByOne, VPNRange, frame_alloc};
use crate::config::{MEMORY_END, PAGE_SIZE, USER_STACK_SIZE};
use crate::sync::UPSafeCell;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

// 内核镜像各段的边界（linker.ld 里定义）
unsafe extern "C" {
    safe fn __text_start();
    safe fn __rodata_start();
    safe fn __data_start();
    safe fn __sbss();
    safe fn __kernel_end();
}

/// 取链接脚本符号的地址（对应 main.rs 里的 `linker_symbol_addr!`）
macro_rules! sym {
    ($f:path) => {
        $f as *const () as usize
    };
}

/// ★ 步骤 1 的临时妥协：app 还在 0xFA100000（内核 RAM 区）里以 EL0 运行，
///   所以 frame pool 这段必须允许 EL0 取指（UXN=0）。
///   步骤 2 把 app 搬进用户页之后改成 false（内核 RAM 对 EL0 彻底关上）。
const FRAME_POOL_EXEC_AT_EL0: bool = false;

/// 内核地址空间（全局唯一；对照 upstream 的 `KERNEL_SPACE`）
static KERNEL_SPACE: UPSafeCell<Option<MemorySet>> = unsafe { UPSafeCell::new(None) };

/// 地址空间（对照 upstream 的 `MemorySet`）
pub struct MemorySet {
    page_table: PageTable,
    areas: Vec<MapArea>,
}

impl MemorySet {
    pub fn new_bare() -> Self {
        Self {
            page_table: PageTable::new(),
            areas: Vec::new(),
        }
    }

    /// 给 `PageTable::from_token` / TTBR0 用的令牌（= 根页表物理地址）
    pub fn token(&self) -> usize {
        self.page_table.token()
    }

    /// 把当前地址空间装进 TTBR0_EL1（对照 upstream 的 `activate`）
    ///
    /// AArch64 上是三件事：写 TTBR0 → `dsb ish` → 清 TLB → `isb`。
    /// 我们没用 ASID 复用（每次全清），所以这里 asid 固定传 0 也行；
    /// 但保留参数，ch5+ 想用 ASID 时不用改接口。
    pub fn activate(&self, asid: u16) {
        let ttbr0 = self.page_table.ttbr0(asid);
        unsafe {
            core::arch::asm!(
                "msr ttbr0_el1, {ttbr0}",
                "isb",
                "tlbi vmalle1",
                "dsb ish",
                // ★ ARMv8 的要求：同一个 VA 换到新的物理页之后，I-cache 里可能还留着
                //   旧页的指令。我们的所有 app 都链接在同一段 VA（0x10000 起）上，
                //   所以每次换地址空间都必须让 I-cache 失效，否则 exec 之后可能
                //   执行到上一个 app 的代码（ch7+ 实测的偶发崩溃，见 docs/07 第十一节）。
                "ic iallu",
                "dsb ish",
                "isb",
                ttbr0 = in(reg) ttbr0,
            );
        }
    }

    /// 插入一段"新分配页帧"的区域（对照 upstream 的 `insert_framed_area`）
    #[allow(dead_code)]
    pub fn insert_framed_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission,
    ) {
        self.push(
            MapArea::new(start_va, end_va, MapType::Framed, permission),
            None,
        );
    }

    fn push(&mut self, mut map_area: MapArea, data: Option<&[u8]>) {
        map_area.map(&mut self.page_table);
        if let Some(data) = data {
            map_area.copy_data(&self.page_table, data);
        }
        self.areas.push(map_area);
    }

    /// 内核地址空间（对照 upstream 的 `new_kernel`）
    ///
    /// 全部是**恒等映射**（VA == PA）：内核链接在 0xFA000000，开 MMU 之后
    /// 地址不变，所以打开 MMU 的那条指令前后 PC 是连续的，不用跳板。
    pub fn new_kernel() -> Self {
        let mut memory_set = Self::new_bare();

        let text_start = sym!(__text_start);
        let rodata_start = sym!(__rodata_start);
        let data_start = sym!(__data_start);
        let bss_start = sym!(__sbss);
        let kernel_end = sym!(__kernel_end);

        println!("[mm] stext   = {:#x}", text_start);
        println!("[mm] srodata = {:#x}", rodata_start);
        println!("[mm] sdata   = {:#x}", data_start);
        println!("[mm] sbss    = {:#x}", bss_start);
        println!("[mm] ekernel = {:#x}", kernel_end);

        // .text + .vectors：只读可执行（EL1）
        memory_set.push(
            MapArea::new(
                text_start.into(),
                rodata_start.into(),
                MapType::Identical,
                MapPermission::R | MapPermission::X,
            ),
            None,
        );
        // .rodata：只读
        memory_set.push(
            MapArea::new(
                rodata_start.into(),
                data_start.into(),
                MapType::Identical,
                MapPermission::R,
            ),
            None,
        );
        // .data + .bss：可读写、不可执行
        memory_set.push(
            MapArea::new(
                data_start.into(),
                kernel_end.into(),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );
        // 页帧池：[ekernel, MEMORY_END) —— 分配器发出去的页要能直接读写
        let pool_perm = if FRAME_POOL_EXEC_AT_EL0 {
            // 步骤 1 的临时妥协：app 还跑在 0xFA100000，必须让 EL0 能取指
            MapPermission::R | MapPermission::W | MapPermission::X | MapPermission::U
        } else {
            MapPermission::R | MapPermission::W
        };
        memory_set.push(
            MapArea::new(
                kernel_end.into(),
                MEMORY_END.into(),
                MapType::Identical,
                pool_perm,
            ),
            None,
        );
        // 设备 / 共享内存（对照 upstream 的 MMIO 列表）
        //   0xFC000000   comm region（hypervisor 填的通信区，Normal 内存）
        //   0xFE215000   mini UART（Device）
        //   0xFF841000   GICD（Device；访问会被 stage-2 转给 hypervisor 处理）
        //   0xFF842000   GICV（Device；hypervisor 映射的虚拟 CPU 接口）
        let mmio: [(usize, usize); 4] = [
            (0xFC000000, 0x1000),
            (0xFE215000, 0x1000),
            (0xFF841000, 0x1000),
            (0xFF842000, 0x1000),
        ];
        for (i, &(addr, size)) in mmio.iter().enumerate() {
            let mut perm = MapPermission::R | MapPermission::W;
            if i != 0 {
                perm |= MapPermission::DEVICE;
            }
            memory_set.push(
                MapArea::new(addr.into(), (addr + size).into(), MapType::Identical, perm),
                None,
            );
        }
        memory_set
    }

    /// 从 ELF 构造一个用户地址空间（对照 upstream 的 `from_elf`）
    ///
    /// 返回 `(memory_set, user_sp, entry_point)`。
    /// 和 upstream 的差别：**没有 trampoline 页、没有 TRAP_CONTEXT 页**
    /// （理由见文件头），用户栈仍然是内核替它映射好的。
    pub fn from_elf(elf_data: &[u8]) -> (Self, usize, usize) {
        let mut memory_set = Self::new_bare();
        let elf = crate::loader::Elf::parse(elf_data);

        let mut max_end_vpn = VirtPageNum(0);
        for ph in elf.load_segments() {
            assert_eq!(ph.vaddr % PAGE_SIZE, 0, "PT_LOAD 的 vaddr 必须页对齐");
            let start_va: VirtAddr = ph.vaddr.into();
            let end_va: VirtAddr = (ph.vaddr + ph.memsz).into();
            let mut map_perm = MapPermission::U;
            if ph.flags & 0x4 != 0 {
                map_perm |= MapPermission::R;
            }
            if ph.flags & 0x2 != 0 {
                map_perm |= MapPermission::W;
            }
            if ph.flags & 0x1 != 0 {
                map_perm |= MapPermission::X;
            }
            let map_area = MapArea::new(start_va, end_va, MapType::Framed, map_perm);
            max_end_vpn = map_area.vpn_range.get_end();
            memory_set.push(map_area, Some(&elf_data[ph.offset..ph.offset + ph.filesz]));
        }

        // 用户栈：在最后一个段之后空一页（guard page），再放 USER_STACK_SIZE
        let max_end_va: VirtAddr = max_end_vpn.into();
        let mut user_stack_bottom: usize = max_end_va.into();
        user_stack_bottom += PAGE_SIZE;
        let user_stack_top = user_stack_bottom + USER_STACK_SIZE;
        memory_set.push(
            MapArea::new(
                user_stack_bottom.into(),
                user_stack_top.into(),
                MapType::Framed,
                MapPermission::R | MapPermission::W | MapPermission::U,
            ),
            None,
        );
        // 给 sbrk 留一个"空区域"（起点 = 栈顶），以后 append_to 往里加页
        memory_set.push(
            MapArea::new(
                user_stack_top.into(),
                user_stack_top.into(),
                MapType::Framed,
                MapPermission::R | MapPermission::W | MapPermission::U,
            ),
            None,
        );

        // ★★ 关键一步：把内核的 L1[3] 子树挂到这个任务的页表里
        //    （0xC0000000~0xFFFFFFFF：内核镜像、页帧池、comm、UART、GIC）
        //    没有它，eret 到 EL0 之后任何一次陷阱进门都会立刻缺页。
        {
            let kernel = KERNEL_SPACE.exclusive_access();
            let kernel_pt = &kernel
                .as_ref()
                .expect("内核地址空间还没初始化（mm::init 要先跑）")
                .page_table;
            memory_set
                .page_table
                .share_l1_entry(kernel_pt, crate::mm::KERNEL_L1_INDEX);
        }

        (memory_set, user_stack_top, elf.entry)
    }

    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.page_table.translate(vpn)
    }

    /// 从另一个用户地址空间**深拷贝**一份（ch5 的 `fork`）
    ///
    /// 对照 upstream 的 `MemorySet::from_existed_user`：
    /// 每个用户页都重新分配物理页，把内容逐字节复制过去（不是写时复制，简单直接）。
    pub fn from_existed_user(user_space: &MemorySet) -> Self {
        let mut memory_set = Self::new_bare();
        for area in user_space.areas.iter() {
            let mut new_area = MapArea::new(
                area.vpn_range.get_start().into(),
                area.vpn_range.get_end().into(),
                area.map_type,
                area.map_perm,
            );
            new_area.map(&mut memory_set.page_table);
            for vpn in area.vpn_range {
                if let Some(src) = area.data_frames.get(&vpn) {
                    let dst_ppn = memory_set.page_table.translate(vpn).unwrap().ppn();
                    let src_bytes = src.ppn.get_bytes_array();
                    let dst_bytes = dst_ppn.get_bytes_array();
                    dst_bytes.copy_from_slice(src_bytes);
                    // 子进程会执行这些页里的代码 → 逐行清 D-cache（ch4 的坑 2）
                    crate::mm::clean_dcache_range_to_pou(PhysAddr::from(dst_ppn).0, PAGE_SIZE);
                }
            }
            memory_set.areas.push(new_area);
        }
        // 内核映射同样要挂进来
        {
            let kernel = KERNEL_SPACE.exclusive_access();
            let kernel_pt = &kernel
                .as_ref()
                .expect("内核地址空间还没初始化")
                .page_table;
            memory_set
                .page_table
                .share_l1_entry(kernel_pt, crate::mm::KERNEL_L1_INDEX);
        }
        memory_set
    }

    /// 进程退出时把用户页帧还回去（页表项先留着，反正这个地址空间不再用）
    /// —— 对照 upstream 的 `recycle_data_pages`
    pub fn recycle_data_pages(&mut self) {
        self.areas.iter_mut().for_each(|area| area.data_frames.clear());
    }

    /// sbrk 收缩（对照 upstream 的 `shrink_to`）
    #[allow(unused)]
    pub fn shrink_to(&mut self, start: VirtAddr, new_end: VirtAddr) -> bool {
        if let Some(area) = self
            .areas
            .iter_mut()
            .find(|area| area.vpn_range.get_start() == start.floor())
        {
            area.shrink_to(&mut self.page_table, new_end.ceil());
            true
        } else {
            false
        }
    }

    /// sbrk 扩张（对照 upstream 的 `append_to`）
    pub fn append_to(&mut self, start: VirtAddr, new_end: VirtAddr) -> bool {
        if let Some(area) = self
            .areas
            .iter_mut()
            .find(|area| area.vpn_range.get_start() == start.floor())
        {
            area.append_to(&mut self.page_table, new_end.ceil());
            true
        } else {
            false
        }
    }
}

/// 一段连续虚拟页的映射（对照 upstream 的 `MapArea`）
pub struct MapArea {
    vpn_range: VPNRange,
    data_frames: BTreeMap<VirtPageNum, FrameTracker>,
    map_type: MapType,
    map_perm: MapPermission,
}

impl MapArea {
    pub fn new(
        start_va: VirtAddr,
        end_va: VirtAddr,
        map_type: MapType,
        map_perm: MapPermission,
    ) -> Self {
        let start_vpn: VirtPageNum = start_va.floor();
        let end_vpn: VirtPageNum = end_va.ceil();
        Self {
            vpn_range: VPNRange::new(start_vpn, end_vpn),
            data_frames: BTreeMap::new(),
            map_type,
            map_perm,
        }
    }

    fn map_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        let ppn: PhysPageNum;
        match self.map_type {
            MapType::Identical => {
                ppn = PhysPageNum(vpn.0);
            }
            MapType::Framed => {
                let frame = frame_alloc().unwrap();
                ppn = frame.ppn;
                self.data_frames.insert(vpn, frame);
            }
        }
        let pte_flags = self.map_perm.to_pte_flags();
        page_table.map(vpn, ppn, pte_flags);
    }

    #[allow(unused)]
    fn unmap_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        if self.map_type == MapType::Framed {
            self.data_frames.remove(&vpn);
        }
        page_table.unmap(vpn);
    }

    fn map(&mut self, page_table: &mut PageTable) {
        for vpn in self.vpn_range {
            self.map_one(page_table, vpn);
        }
    }

    #[allow(unused)]
    fn unmap(&mut self, page_table: &mut PageTable) {
        for vpn in self.vpn_range {
            self.unmap_one(page_table, vpn);
        }
    }

    #[allow(unused)]
    fn shrink_to(&mut self, page_table: &mut PageTable, new_end: VirtPageNum) {
        for vpn in VPNRange::new(new_end, self.vpn_range.get_end()) {
            self.unmap_one(page_table, vpn)
        }
        self.vpn_range = VPNRange::new(self.vpn_range.get_start(), new_end);
    }

    fn append_to(&mut self, page_table: &mut PageTable, new_end: VirtPageNum) {
        for vpn in VPNRange::new(self.vpn_range.get_end(), new_end) {
            self.map_one(page_table, vpn)
        }
        self.vpn_range = VPNRange::new(self.vpn_range.get_start(), new_end);
    }

    /// 把 app 的字节拷进刚分配好的页帧（对照 upstream 的 `copy_data`）
    ///
    /// `data` 从区域起始页开始放（所以要页对齐），长度可以短于区域大小
    /// （.bss 部分靠 `frame_alloc` 的清零）。
    fn copy_data(&mut self, page_table: &PageTable, data: &[u8]) {
        assert_eq!(self.map_type, MapType::Framed);
        let mut start: usize = 0;
        let mut current_vpn = self.vpn_range.get_start();
        let len = data.len();
        loop {
            let src = &data[start..len.min(start + PAGE_SIZE)];
            let ppn = page_table.translate(current_vpn).unwrap().ppn();
            let dst = &mut ppn.get_bytes_array()[..src.len()];
            dst.copy_from_slice(src);
            // ★ 开了 cache 之后，写进去的代码要先清 D-cache 到 PoU（逐行！），
            //   否则 EL0 取指（读 PoU）可能看到旧内容。
            crate::mm::clean_dcache_range_to_pou(PhysAddr::from(ppn).0, src.len());
            start += PAGE_SIZE;
            if start >= len {
                break;
            }
            current_vpn.step();
        }
    }
}

/// 映射方式（对照 upstream 的 `MapType`）
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum MapType {
    /// 恒等映射：虚拟页号 == 物理页号（内核/MMIO 用）
    Identical,
    /// 新分配页帧（用户空间用）
    Framed,
}

/// 映射权限（对照 upstream 的 `MapPermission`，但多了 `DEVICE`）
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct MapPermission(pub u8);

impl MapPermission {
    pub const R: Self = Self(1 << 1);
    pub const W: Self = Self(1 << 2);
    pub const X: Self = Self(1 << 3);
    pub const U: Self = Self(1 << 4);
    /// Device 内存（MMIO）：走 MAIR attr1，而不是 Normal
    pub const DEVICE: Self = Self(1 << 5);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// 翻译成 AArch64 页表项标志（对照 upstream 里 `PTEFlags::from_bits(perm.bits)`）
    pub fn to_pte_flags(self) -> PTEFlags {
        let mut f = PTEFlags::AF | PTEFlags::SH_INNER;
        f = f
            | if self.contains(Self::DEVICE) {
                PTEFlags::ATTR_DEVICE
            } else {
                PTEFlags::ATTR_NORMAL
            };
        if self.contains(Self::U) {
            // 用户页：EL0 可读写；可执行与否看 UXN
            f = f | PTEFlags::AP_EL0_RW | PTEFlags::PXN;
            if !self.contains(Self::X) {
                f = f | PTEFlags::UXN;
            }
        } else {
            // 内核页：EL0 一律不可访问 + 不可执行
            f = f | PTEFlags::UXN;
            if self.contains(Self::W) {
                f = f | PTEFlags::AP_EL1_RW;
            } else {
                f = f | PTEFlags::AP_EL1_RO;
            }
            if !self.contains(Self::X) {
                f = f | PTEFlags::PXN;
            }
        }
        f
    }
}

impl core::ops::BitOr for MapPermission {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl core::ops::BitOrAssign for MapPermission {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// 初始化内核地址空间（对照 upstream 的 `mm::init` 里的 `KERNEL_SPACE.activate()`）
pub fn init_kernel_space() {
    let mut space = KERNEL_SPACE.exclusive_access();
    *space = Some(MemorySet::new_kernel());
    space.as_ref().unwrap().activate(0);
}

/// 取内核页表的 token（trap/task 里切回内核空间时用；ch4 用不到但先留着）
#[allow(unused)]
pub fn kernel_token() -> usize {
    KERNEL_SPACE.exclusive_access().as_ref().unwrap().token()
}

/// 权限自测（对照 upstream 的 `remap_test`）
///
/// 检查：.text 不可写、.rodata 不可写、.data 不可执行。
pub fn remap_test() {
    let space = KERNEL_SPACE.exclusive_access();
    let space = space.as_ref().unwrap();
    let mid_text: VirtAddr = ((sym!(__text_start) + sym!(__rodata_start)) / 2).into();
    let mid_data: VirtAddr = ((sym!(__data_start) + sym!(__sbss)) / 2).into();
    let text_pte = space.translate(mid_text.floor()).unwrap();
    let data_pte = space.translate(mid_data.floor()).unwrap();
    assert!(!text_pte.is_writable(), "remap_test: .text 居然可写！");
    assert!(text_pte.executable_at_el1(), "remap_test: .text 不可执行！");
    assert!(data_pte.is_writable(), "remap_test: .data 不可写！");
    assert!(!data_pte.executable_at_el1(), "remap_test: .data 居然可执行！");
    println!("[mm] remap_test passed: .text RX / .data RW&XN");
}
