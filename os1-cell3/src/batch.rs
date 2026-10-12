//! 批处理系统（ch2-C）—— 对照 rCore ch2 的 `os/src/batch.rs`
//!
//! 和 rCore 的差别（都在注释里标了）：
//!   1) app 基址 = **0xFA100000**（cell3 的 [1] RAM 区），而不是 0x80400000
//!   2) app 跑在 **EL0**（已实测可用），由 `__restore` 恢复 TrapContext 后 `eret` 下去
//!   3) app 崩溃（非法指令等）→ `trap_handler` 直接调 `run_next_app()` 换下一个
//!   4) 没有 `lazy_static` 依赖：`AppManager` 用 `const fn new()` + `init()` 从
//!      链接器符号 `_num_app` 读 app 表（省一个 crate）

use crate::sync::UPSafeCell;
use crate::trap::TrapContext;

const USER_STACK_SIZE: usize = 4096 * 2;
const KERNEL_STACK_SIZE: usize = 4096 * 2;
const MAX_APP_NUM: usize = 16;
/// app 被拷到这里执行（必须等于 user/src/linker.ld 的 BASE_ADDRESS）
const APP_BASE_ADDRESS: usize = 0xFA100000;
/// 单个 app 的大小上限：每次加载前会把这块清零（对照 rCore 的 APP_SIZE_LIMIT）
const APP_SIZE_LIMIT: usize = 0x20000;

#[repr(align(4096))]
struct KernelStack {
    data: [u8; KERNEL_STACK_SIZE],
}

#[repr(align(4096))]
struct UserStack {
    data: [u8; USER_STACK_SIZE],
}

// ⚠️ 必须是 `static mut`：
//   不可变 static 会被 LLVM 放进 **.rodata（只读段）**（本项目实测：栈地址落在 .rodata），
//   而栈是要被写的！现在没炸只是因为 cell 的 LOADABLE 区恰好是 RW；
//   一旦把 .rodata 设成只读（正常加固），写栈就会变成 stage-2 权限异常
//   → jailhouse "Unhandled data write" → hypervisor panic（整板挂死）。
//   `static mut` + 全零初始化才会落到 .bss（由 clear_bss 清零）。
static mut KERNEL_STACK: KernelStack = KernelStack {
    data: [0; KERNEL_STACK_SIZE],
};
static mut USER_STACK: UserStack = UserStack {
    data: [0; USER_STACK_SIZE],
};

/// 取得内核栈（只读引用；栈内容通过原始指针写，不产生别名问题）
fn kernel_stack() -> &'static KernelStack {
    unsafe { &*core::ptr::addr_of!(KERNEL_STACK) }
}

/// 取得用户栈
fn user_stack() -> &'static UserStack {
    unsafe { &*core::ptr::addr_of!(USER_STACK) }
}

impl KernelStack {
    fn get_sp(&self) -> usize {
        self.data.as_ptr() as usize + KERNEL_STACK_SIZE
    }

    /// 把 app 的初始现场压在**内核栈顶**，返回它的地址（给 `__restore` 用）
    /// —— 对照 rCore 的 `KernelStack::push_context`
    pub fn push_context(&self, cx: TrapContext) -> &'static mut TrapContext {
        let cx_ptr = (self.get_sp() - core::mem::size_of::<TrapContext>()) as *mut TrapContext;
        unsafe {
            *cx_ptr = cx;
        }
        unsafe { cx_ptr.as_mut().unwrap() }
    }
}

impl UserStack {
    fn get_sp(&self) -> usize {
        self.data.as_ptr() as usize + USER_STACK_SIZE
    }
}

struct AppManager {
    num_app: usize,
    current_app: usize,
    /// app_i 的起止地址（app_start[num_app] = 最后一个 app 的结束地址）
    app_start: [usize; MAX_APP_NUM + 1],
}

impl AppManager {
    const fn new() -> Self {
        Self {
            num_app: 0,
            current_app: 0,
            app_start: [0; MAX_APP_NUM + 1],
        }
    }

    /// 从 `link_app.S` 里的 `_num_app` 读出 app 表（对照 rCore 的 lazy_static 初始化）
    fn init_from_linker(&mut self) {
        unsafe extern "C" {
            safe fn _num_app();
        }
        let num_app_ptr = _num_app as *const () as *const usize;
        let num_app = unsafe { num_app_ptr.read_volatile() };
        assert!(num_app <= MAX_APP_NUM, "app 太多（改 MAX_APP_NUM）");
        self.num_app = num_app;
        let start = unsafe { core::slice::from_raw_parts(num_app_ptr.add(1), num_app + 1) };
        self.app_start[..=num_app].copy_from_slice(start);
    }

    fn print_app_info(&self) {
        crate::println!("[kernel] num_app = {}", self.num_app);
        for i in 0..self.num_app {
            crate::println!(
                "[kernel] app_{} [{:#x}, {:#x})",
                i,
                self.app_start[i],
                self.app_start[i + 1]
            );
        }
    }

    /// 把第 app_id 个 app 拷到 APP_BASE_ADDRESS（对照 rCore 的 `load_app`）
    fn load_app(&self, app_id: usize) {
        assert!(app_id < self.num_app, "app_id 越界");
        // 关中断（对照 rCore 的 sie::clear_stimer()；我们本来就没开中断，保持语义）
        unsafe { core::arch::asm!("msr daifset, #2") };

        let dst = APP_BASE_ADDRESS as *mut u8;
        // 先把目标区清零（防止上一个 app 的残留数据影响这一个）
        for i in 0..APP_SIZE_LIMIT {
            unsafe { dst.add(i).write_volatile(0) };
        }
        // 再拷 app 的裸二进制
        let src = self.app_start[app_id] as *const u8;
        let len = (self.app_start[app_id + 1] - self.app_start[app_id]).min(APP_SIZE_LIMIT);
        for i in 0..len {
            unsafe { dst.add(i).write_volatile(src.add(i).read_volatile()) };
        }
        crate::println!("[kernel] load app_{} ({} 字节) → {:#x}", app_id, len, APP_BASE_ADDRESS);

        unsafe { core::arch::asm!("msr daifclr, #2") };
    }
}

static APP_MANAGER: UPSafeCell<AppManager> = unsafe { UPSafeCell::new(AppManager::new()) };

/// 初始化（对照 rCore 的 `batch::init()`）
pub fn init() {
    APP_MANAGER.exclusive_access().init_from_linker();
    APP_MANAGER.exclusive_access().print_app_info();
}

/// 跑下一个 app（对照 rCore 的 `batch::run_next_app()`）
pub fn run_next_app() -> ! {
    unsafe extern "C" {
        safe fn __restore(cx_addr: usize) -> !;
    }

    let mut app_manager = APP_MANAGER.exclusive_access();
    let current_app = app_manager.current_app;

    if current_app == app_manager.num_app {
        // 全部跑完（rCore ch2 在这里会越界，我们明确收尾）
        crate::println!("[kernel] All apps completed!");
        drop(app_manager);
        crate::trap::shutdown();
    }

    app_manager.load_app(current_app);
    app_manager.current_app += 1;
    // 手工放掉借用（对照 rCore 的 drop(app_manager)）
    drop(app_manager);

    let cx = TrapContext::app_init_context(APP_BASE_ADDRESS, user_stack().get_sp());
    let cx_ptr = kernel_stack().push_context(cx) as *const TrapContext as usize;
    __restore(cx_ptr)
}
