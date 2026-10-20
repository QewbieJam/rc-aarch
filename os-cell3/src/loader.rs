//! 用户程序加载（ch4）—— 对照 rCore ch4 的 `os/src/loader.rs`
//!
//! ch4 的 loader 比 ch3 **简单得多**（upstream 也是这样）：
//!   ch3 要自己把裸二进制拷到固定地址、自己管用户栈、自己压初始 TrapContext；
//!   ch4 这些全部交给 `MemorySet::from_elf` + 页帧分配器，loader 只剩两件事：
//!     ① 从 `link_app.S` 的 `_num_app` 表里读"第 i 个 app 的 ELF 在哪"；
//!     ② 给每个任务提供一块**内核栈**（`.bss` 里的静态数组，恒等映射，天然可访问）。
//!
//! 内核栈为什么还留在静态数组里？
//!   rCore ch4 把内核栈也放进"内核地址空间"（`kernel_stack_position` + 高地址映射），
//!   因为 RISC-V 的陷阱入口要靠 trampoline 切页表，内核栈必须是**固定 VA**。
//!   AArch64 这边内核恒等映射在 TTBR0 里一直有效，内核栈放 .bss 就行，
//!   不需要额外的 `insert_framed_area`。
//!
//! ★ ELF 解析：对照 upstream 用的 `xmas_elf`，我们手写一个只认 ELF64/PT_LOAD 的
//!   极简版本（见文件末尾）。ch4 只需要 `e_entry`、`p_offset/p_vaddr/p_filesz/p_memsz/p_flags`。

use crate::config::{KERNEL_STACK_SIZE, MAX_APP_NUM};
use crate::trap::TrapContext;

#[repr(align(4096))]
#[derive(Copy, Clone)]
pub struct KernelStack {
    data: [u8; KERNEL_STACK_SIZE],
}

const KSTACK_ZERO: KernelStack = KernelStack {
    data: [0; KERNEL_STACK_SIZE],
};

// ⚠️ 必须是 `static mut`：不可变 static 会被放进 .rodata（只读），而栈是要写的。
//    （ch2 已经因为这个踩过坑，见 docs/02 的坑 2。）
static mut KERNEL_STACK: [KernelStack; MAX_APP_NUM] = [KSTACK_ZERO; MAX_APP_NUM];

impl KernelStack {
    fn get_sp(&self) -> usize {
        self.data.as_ptr() as usize + KERNEL_STACK_SIZE
    }

    /// 把任务的初始 TrapContext 压在**内核栈顶**，返回它在内核栈上的地址
    ///
    /// 这个地址就是 `TaskContext::goto_restore(kstack_ptr)` 的 `sp`：
    /// `__switch` 换过去之后 `ret` 到 `__restore_sp`，后者直接拿它当 TrapContext。
    /// —— ch4 依然不让内核栈参与页表（恒等映射），所以这块和 ch3 一模一样。
    pub fn push_context(&self, cx: TrapContext) -> usize {
        let cx_ptr = (self.get_sp() - core::mem::size_of::<TrapContext>()) as *mut TrapContext;
        unsafe {
            cx_ptr.write_volatile(cx);
        }
        cx_ptr as usize
    }
}

/// 取第 i 个任务的内核栈
pub fn kernel_stack(app_id: usize) -> &'static KernelStack {
    assert!(app_id < MAX_APP_NUM, "app_id 越界（改 config::MAX_APP_NUM）");
    let base = core::ptr::addr_of!(KERNEL_STACK) as *const KernelStack;
    unsafe { &*base.add(app_id) }
}

/// 链接器符号 `_num_app`：`[num_app, app0_start, ..., appN_end]`
fn app_table() -> (&'static [usize], usize) {
    unsafe extern "C" {
        safe fn _num_app();
    }
    let num_app_ptr = _num_app as *const () as *const usize;
    let num_app = unsafe { num_app_ptr.read_volatile() };
    let starts = unsafe { core::slice::from_raw_parts(num_app_ptr.add(1), num_app + 1) };
    (starts, num_app)
}

/// app 个数（对照 upstream 的 `get_num_app`）
pub fn get_num_app() -> usize {
    app_table().1
}

/// 第 i 个 app 的 ELF 字节（对照 upstream 的 `get_app_data`）
pub fn get_app_data(app_id: usize) -> &'static [u8] {
    let (starts, num_app) = app_table();
    assert!(app_id < num_app);
    unsafe {
        core::slice::from_raw_parts(
            starts[app_id] as *const u8,
            starts[app_id + 1] - starts[app_id],
        )
    }
}

// ===========================================================================
// 极简 ELF64 解析（对照 upstream 用的 `xmas_elf`）
//
// 只认 ELF64 / little-endian / PT_LOAD，够 ch4 用。
// 需要的字段：e_entry、e_phoff、e_phentsize、e_phnum；
// 每个 PT_LOAD 的 p_flags / p_offset / p_vaddr / p_filesz / p_memsz。
// ===========================================================================

/// 一个 PT_LOAD 段（字段名和 ELF 规范一致）
#[derive(Copy, Clone, Debug)]
pub struct ProgramHeader {
    pub flags: u32,
    pub offset: usize,
    pub vaddr: usize,
    pub filesz: usize,
    pub memsz: usize,
}

/// 极简 ELF64 解析器
pub struct Elf<'a> {
    data: &'a [u8],
    pub entry: usize,
    phoff: usize,
    phentsize: usize,
    phnum: usize,
}

impl<'a> Elf<'a> {
    pub fn parse(data: &'a [u8]) -> Self {
        assert_eq!(&data[0..4], b"\x7fELF", "不是 ELF 文件！");
        assert_eq!(data[4], 2, "只支持 ELF64");
        let entry = read_u64(data, 0x18) as usize;
        let phoff = read_u64(data, 0x20) as usize;
        let phentsize = read_u16(data, 0x36) as usize;
        let phnum = read_u16(data, 0x38) as usize;
        Self {
            data,
            entry,
            phoff,
            phentsize,
            phnum,
        }
    }

    /// 所有 PT_LOAD 段
    pub fn load_segments(&self) -> alloc::vec::Vec<ProgramHeader> {
        let mut v = alloc::vec::Vec::new();
        for i in 0..self.phnum {
            let base = self.phoff + i * self.phentsize;
            if read_u32(self.data, base) != 1 {
                continue; // 只要 PT_LOAD
            }
            v.push(ProgramHeader {
                flags: read_u32(self.data, base + 0x4),
                offset: read_u64(self.data, base + 0x8) as usize,
                vaddr: read_u64(self.data, base + 0x10) as usize,
                filesz: read_u64(self.data, base + 0x20) as usize,
                memsz: read_u64(self.data, base + 0x28) as usize,
            });
        }
        v
    }
}

fn read_u16(d: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([d[off], d[off + 1]])
}
fn read_u32(d: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]])
}
fn read_u64(d: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        d[off],
        d[off + 1],
        d[off + 2],
        d[off + 3],
        d[off + 4],
        d[off + 5],
        d[off + 6],
        d[off + 7],
    ])
}
