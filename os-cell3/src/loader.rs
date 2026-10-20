//! 把用户程序装进内存 + 准备每个任务的栈（ch3）
//!
//! 对照 rCore ch3 的 `os/src/loader.rs`，结构几乎一一对应：
//!
//!   rCore                              →  rc-aarch
//!   -----------------------------------   ---------------------------------------
//!   `KERNEL_STACK: [KernelStack; N]`     同名同结构（每任务一个内核栈）
//!   `USER_STACK: [UserStack; N]`         同名同结构（每任务一个用户栈）
//!   `load_apps()` 把 app 拷到 APP_BASE   同（我们再加一次 I-cache 同步）
//!   `init_app_cx(i)` 压 TrapContext      同，返回的地址给 `TaskContext::goto_restore`
//!
//! ★★ 和 upstream 的两点关键差异（都是 AArch64/我们这个 cell 特有的）：
//!
//! 1) **栈数组必须 `static mut`**
//!    不可变 static 会被 LLVM 放进 `.rodata`（只读段）。栈是要写的，现在没炸
//!    只是因为 cell 的 LOADABLE 区恰好可写；一旦以后把 .rodata 设成只读，
//!    写栈就会变成 stage-2 权限异常 → hypervisor "Unhandled data write" → 整板挂。
//!    `static mut` + 全零初值才会落到 `.bss`（由 clear_bss 清零）。
//!
//! 2) **这里不能用 atomic**（见 trap/mod.rs 的说明）
//!    stage-1 MMU 关闭时内存被当成 Device-nGnRnE，LDXR/STXR 永远失败。
//!    栈的分配是单核 + 顺序的，用普通指针写就够了。

use crate::config::*;
use crate::trap::TrapContext;
use core::arch::asm;

#[repr(align(4096))]
#[derive(Copy, Clone)]
struct KernelStack {
    data: [u8; KERNEL_STACK_SIZE],
}

#[repr(align(4096))]
#[derive(Copy, Clone)]
struct UserStack {
    data: [u8; USER_STACK_SIZE],
}

const KSTACK_ZERO: KernelStack = KernelStack {
    data: [0; KERNEL_STACK_SIZE],
};
const USTACK_ZERO: UserStack = UserStack {
    data: [0; USER_STACK_SIZE],
};

static mut KERNEL_STACK: [KernelStack; MAX_APP_NUM] = [KSTACK_ZERO; MAX_APP_NUM];
static mut USER_STACK: [UserStack; MAX_APP_NUM] = [USTACK_ZERO; MAX_APP_NUM];

impl KernelStack {
    /// 栈顶（内核栈从高地址往低地址长）
    fn get_sp(&self) -> usize {
        self.data.as_ptr() as usize + KERNEL_STACK_SIZE
    }

    /// 把 app 的初始现场压在**内核栈顶**，返回它在内核栈上的地址
    /// —— 对照 rCore 的 `KernelStack::push_context`
    ///
    /// 这个地址就是 `TaskContext::goto_restore(kstack_ptr)` 的 `sp`：
    /// `__switch` 换过去之后 `ret` 到 `__restore_sp`，后者直接拿它当 TrapContext。
    pub fn push_context(&self, cx: TrapContext) -> usize {
        let cx_ptr = (self.get_sp() - core::mem::size_of::<TrapContext>()) as *mut TrapContext;
        unsafe {
            cx_ptr.write_volatile(cx);
        }
        cx_ptr as usize
    }
}

impl UserStack {
    fn get_sp(&self) -> usize {
        self.data.as_ptr() as usize + USER_STACK_SIZE
    }
}

/// 取第 i 个内核栈（用裸指针 + addr_of!，不制造 `&mut static`）
fn kernel_stack(i: usize) -> &'static KernelStack {
    let base = core::ptr::addr_of!(KERNEL_STACK) as *const KernelStack;
    unsafe { &*base.add(i) }
}

fn user_stack(i: usize) -> &'static UserStack {
    let base = core::ptr::addr_of!(USER_STACK) as *const UserStack;
    unsafe { &*base.add(i) }
}

/// 链接器符号 `_num_app`：`[num_app, app0_start, app1_start, ..., appN_end]`
/// （由 build.rs 生成的 link_app.S 定义，对照 rCore 的 link_app.S）
fn app_table() -> (&'static [usize], usize) {
    unsafe extern "C" {
        safe fn _num_app();
    }
    let num_app_ptr = _num_app as *const () as *const usize;
    let num_app = unsafe { num_app_ptr.read_volatile() };
    let starts = unsafe { core::slice::from_raw_parts(num_app_ptr.add(1), num_app + 1) };
    (starts, num_app)
}

/// app 个数（对照 rCore 的 `get_num_app()`）
pub fn get_num_app() -> usize {
    app_table().1
}

/// 第 i 个 app 在内存里的加载地址（对照 rCore 的 `get_base_i`）
fn get_base_i(app_id: usize) -> usize {
    APP_BASE_ADDRESS + app_id * APP_SIZE_LIMIT
}

/// 把所有 app 从内核镜像的 `.data` 拷到各自的运行地址
/// —— 对照 rCore 的 `load_apps()`（upstream 是懒加载 `load_app`，ch3 改成一次全装）
pub fn load_apps() {
    let (starts, num_app) = app_table();
    println!("[kernel] num_app = {}", num_app);
    for i in 0..num_app {
        let base = get_base_i(i);
        let len = starts[i + 1] - starts[i];
        // 先清零整块（防止上一个 app 的残留数据影响这一个）
        let dst = base as *mut u8;
        for off in 0..APP_SIZE_LIMIT {
            unsafe { dst.add(off).write_volatile(0) };
        }
        // 再逐字节拷裸二进制
        let src = starts[i] as *const u8;
        for off in 0..len {
            unsafe { dst.add(off).write_volatile(src.add(off).read_volatile()) };
        }
        println!(
            "[kernel] load app_{} ({} 字节) → {:#x}",
            i,
            len,
            base
        );
    }
    // ★ AArch64 对应 RISC-V `fence.i` 的做法：数据写完之后同步指令流
    //   （我们没开 MMU、也没开 I-cache/D-cache，本来是空操作，写上是为了将来不出错）
    unsafe {
        asm!("dsb ish", "ic iallu", "dsb ish", "isb", options(nostack, preserves_flags));
    }
}

/// 构造第 i 个任务的内核栈顶现场，返回 `TaskContext.sp` 要用的地址
/// —— 对照 rCore 的 `init_app_cx(app_id)`
pub fn init_app_cx(app_id: usize) -> usize {
    kernel_stack(app_id).push_context(TrapContext::app_init_context(
        get_base_i(app_id),
        user_stack(app_id).get_sp(),
    ))
}
