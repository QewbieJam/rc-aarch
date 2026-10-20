//! 物理地址 / 虚拟地址 / 页号（ch4）—— 对照 rCore ch4 的 `os/src/mm/address.rs`
//!
//! 和 upstream（SV39）一一对应，只是位宽换成 AArch64 的：
//!
//! ```text
//!              rCore (SV39)          rc-aarch (AArch64, 4KB granule, 48-bit VA)
//!   虚拟地址   39 bit                48 bit（TCR_EL1.T0SZ = 16）
//!   物理地址   56 bit                40 bit（TCR_EL1.IPS = 0b010）
//!   页大小     4KB                   4KB
//!   页表级数   3 级（L0/L1/L2）       4 级（L0/L1/L2/L3）
//!   每级索引   9 bit                 9 bit（512 项 × 8 字节 = 4KB）
//! ```
//!
//! ★ 为什么是 4 级：48 位 VA = 9+9+9+9 + 12 位页内偏移。
//!   RISC-V 的 SV39 是 3 级（9+9+9+12 = 39）。
//!   级数不同 ⇒ `indexes()` 返回 4 个数而不是 3 个，`find_pte*` 也要多走一层。

use super::PageTableEntry;

/// 物理地址宽度（TCR_EL1.IPS = 0b010 → 40 bit，足够覆盖 cell 的 0xFA000000~0xFC000000）
pub const PA_WIDTH: usize = 40;
/// 虚拟地址宽度（TCR_EL1.T0SZ = 16 → 48 bit）
pub const VA_WIDTH: usize = 48;
/// 页号宽度
pub const PPN_WIDTH: usize = PA_WIDTH - crate::config::PAGE_SIZE_BITS;
pub const VPN_WIDTH: usize = VA_WIDTH - crate::config::PAGE_SIZE_BITS;

/// 物理地址
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PhysAddr(pub usize);

/// 虚拟地址
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct VirtAddr(pub usize);

/// 物理页号
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct PhysPageNum(pub usize);

/// 虚拟页号
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct VirtPageNum(pub usize);

impl core::fmt::Debug for VirtAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "VA:{:#x}", self.0)
    }
}
impl core::fmt::Debug for VirtPageNum {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "VPN:{:#x}", self.0)
    }
}
impl core::fmt::Debug for PhysAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PA:{:#x}", self.0)
    }
}
impl core::fmt::Debug for PhysPageNum {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PPN:{:#x}", self.0)
    }
}

impl From<usize> for PhysAddr {
    fn from(v: usize) -> Self {
        Self(v & ((1 << PA_WIDTH) - 1))
    }
}
impl From<usize> for PhysPageNum {
    fn from(v: usize) -> Self {
        Self(v & ((1 << PPN_WIDTH) - 1))
    }
}
impl From<usize> for VirtAddr {
    fn from(v: usize) -> Self {
        Self(v & ((1 << VA_WIDTH) - 1))
    }
}
impl From<usize> for VirtPageNum {
    fn from(v: usize) -> Self {
        Self(v & ((1 << VPN_WIDTH) - 1))
    }
}
impl From<PhysAddr> for usize {
    fn from(v: PhysAddr) -> Self {
        v.0
    }
}
impl From<PhysPageNum> for usize {
    fn from(v: PhysPageNum) -> Self {
        v.0
    }
}
impl From<VirtAddr> for usize {
    fn from(v: VirtAddr) -> Self {
        v.0
    }
}
impl From<VirtPageNum> for usize {
    fn from(v: VirtPageNum) -> Self {
        v.0
    }
}

impl VirtAddr {
    pub fn floor(&self) -> VirtPageNum {
        VirtPageNum(self.0 / crate::config::PAGE_SIZE)
    }
    pub fn ceil(&self) -> VirtPageNum {
        if self.0 == 0 {
            VirtPageNum(0)
        } else {
            VirtPageNum((self.0 - 1 + crate::config::PAGE_SIZE) / crate::config::PAGE_SIZE)
        }
    }
    pub fn page_offset(&self) -> usize {
        self.0 & (crate::config::PAGE_SIZE - 1)
    }
    pub fn aligned(&self) -> bool {
        self.page_offset() == 0
    }
}
impl From<VirtAddr> for VirtPageNum {
    fn from(v: VirtAddr) -> Self {
        assert_eq!(v.page_offset(), 0);
        v.floor()
    }
}
impl From<VirtPageNum> for VirtAddr {
    fn from(v: VirtPageNum) -> Self {
        Self(v.0 << crate::config::PAGE_SIZE_BITS)
    }
}
impl PhysAddr {
    pub fn floor(&self) -> PhysPageNum {
        PhysPageNum(self.0 / crate::config::PAGE_SIZE)
    }
    pub fn ceil(&self) -> PhysPageNum {
        if self.0 == 0 {
            PhysPageNum(0)
        } else {
            PhysPageNum((self.0 - 1 + crate::config::PAGE_SIZE) / crate::config::PAGE_SIZE)
        }
    }
    pub fn page_offset(&self) -> usize {
        self.0 & (crate::config::PAGE_SIZE - 1)
    }
    pub fn aligned(&self) -> bool {
        self.page_offset() == 0
    }
}
impl From<PhysAddr> for PhysPageNum {
    fn from(v: PhysAddr) -> Self {
        assert_eq!(v.page_offset(), 0);
        v.floor()
    }
}
impl From<PhysPageNum> for PhysAddr {
    fn from(v: PhysPageNum) -> Self {
        Self(v.0 << crate::config::PAGE_SIZE_BITS)
    }
}

impl VirtPageNum {
    /// 4 级页表索引（VMSAv8-64, 4KB granule, 48-bit VA）
    ///
    /// ```text
    /// bit  47     39 38     30 29     21 20     12 11        0
    ///      [ L0  9 ] [ L1  9 ] [ L2  9 ] [ L3  9 ] [ 页内 12 ]
    /// ```
    pub fn indexes(&self) -> [usize; 4] {
        let mut vpn = self.0;
        let mut idx = [0usize; 4];
        for i in (0..4).rev() {
            idx[i] = vpn & 511;
            vpn >>= 9;
        }
        idx
    }
}

impl PhysPageNum {
    /// 把物理页当成 512 个页表项的数组
    pub fn get_pte_array(&self) -> &'static mut [PageTableEntry] {
        let pa: PhysAddr = (*self).into();
        unsafe { core::slice::from_raw_parts_mut(pa.0 as *mut PageTableEntry, 512) }
    }
    /// 把物理页当成 4096 字节的数组
    pub fn get_bytes_array(&self) -> &'static mut [u8] {
        let pa: PhysAddr = (*self).into();
        unsafe { core::slice::from_raw_parts_mut(pa.0 as *mut u8, crate::config::PAGE_SIZE) }
    }
    /// 把物理页当成一个 T
    pub fn get_mut<T>(&self) -> &'static mut T {
        let pa: PhysAddr = (*self).into();
        unsafe { (pa.0 as *mut T).as_mut().unwrap() }
    }
}

/// 页号的 `+1` 迭代器（对照 upstream 的 `StepByOne`）
pub trait StepByOne {
    fn step(&mut self);
}
impl StepByOne for VirtPageNum {
    fn step(&mut self) {
        self.0 += 1;
    }
}

/// 简单的半开区间 [l, r)，用于 `VPNRange`
#[derive(Copy, Clone)]
pub struct SimpleRange<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + core::fmt::Debug,
{
    l: T,
    r: T,
}
impl<T> SimpleRange<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + core::fmt::Debug,
{
    pub fn new(start: T, end: T) -> Self {
        assert!(start <= end, "start {:?} > end {:?}!", start, end);
        Self { l: start, r: end }
    }
    pub fn get_start(&self) -> T {
        self.l
    }
    pub fn get_end(&self) -> T {
        self.r
    }
}
impl<T> IntoIterator for SimpleRange<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + core::fmt::Debug,
{
    type Item = T;
    type IntoIter = SimpleRangeIterator<T>;
    fn into_iter(self) -> Self::IntoIter {
        SimpleRangeIterator::new(self.l, self.r)
    }
}
pub struct SimpleRangeIterator<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + core::fmt::Debug,
{
    current: T,
    end: T,
}
impl<T> SimpleRangeIterator<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + core::fmt::Debug,
{
    pub fn new(l: T, r: T) -> Self {
        Self { current: l, end: r }
    }
}
impl<T> Iterator for SimpleRangeIterator<T>
where
    T: StepByOne + Copy + PartialEq + PartialOrd + core::fmt::Debug,
{
    type Item = T;
    fn next(&mut self) -> Option<Self::Item> {
        if self.current == self.end {
            None
        } else {
            let t = self.current;
            self.current.step();
            Some(t)
        }
    }
}

/// 虚拟页号区间
pub type VPNRange = SimpleRange<VirtPageNum>;
