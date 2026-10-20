//! 用户库（ch3）—— 对照 rCore ch3 的 `user/src/lib.rs`
//!
//! 每个 app（`src/bin/*.rs`）只写一个 `#[unsafe(no_mangle)] fn main() -> i32`，
//! 由这里的 `_start()` 负责：清 .bss → 调 main → 用 `sys_exit` 退出。
//!
//! 运行环境（实测确认）：**EL0**（用户态），通过 `svc #0` 陷入内核的 EL1 向量表 #8。
//! 注意：EL0 下**不能**执行 `hvc`/特权寄存器访问（那是未定义指令），
//!       所以 app 里唯一的输出手段就是 `sys_write` 系统调用。

#![no_std]

/// 把链接脚本里的符号变成 usize（和 rCore ch2 的写法一致）
macro_rules! linker_symbol_addr {
    ($symbol:path) => {
        ($symbol as *const ()).addr()
    };
}

#[macro_use]
pub mod console;
mod lang_items;
mod syscall;

// app 提供 main（对应 rCore 的 `unsafe extern "Rust" { fn main() -> i32; }`）
unsafe extern "Rust" {
    fn main() -> i32;
}

/// 用户程序入口（对应 rCore ch2 的 `_start`）
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    clear_bss();
    unsafe { exit(main()) }
}

/// 清 .bss：链接器只把"文件里有的字节"装进 .bin，
/// 所以 .bss 必须由 app 自己清零（内核不做这件事）
fn clear_bss() {
    unsafe extern "C" {
        safe fn start_bss();
        safe fn end_bss();
    }
    (linker_symbol_addr!(start_bss)..linker_symbol_addr!(end_bss))
        .for_each(|addr| unsafe { (addr as *mut u8).write_volatile(0) });
}

use syscall::*;

// ---- ch7：信号 ----
/// 信号处理动作（和内核的 `SignalAction` 布局必须一致：16 字节）
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct SignalAction {
    pub handler: usize,
    pub mask: u32,
}

impl Default for SignalAction {
    fn default() -> Self {
        Self {
            handler: 0,
            mask: 0,
        }
    }
}

/// 常用信号号（Linux 编号）
pub const SIGDEF: i32 = 0;
pub const SIGKILL: i32 = 9;
pub const SIGUSR1: i32 = 10;
pub const SIGSEGV: i32 = 11;
pub const SIGUSR2: i32 = 12;
pub const SIGCONT: i32 = 18;
pub const SIGSTOP: i32 = 19;

/// 给某个进程发信号（`signo=0` 是 SIGDEF，Linux 里用来探测进程存在）
pub fn kill(pid: usize, signo: i32) -> isize {
    sys_kill(pid, signo)
}

/// 设置信号处理函数；`old_action` 非空时回填旧的处理动作
pub fn sigaction(
    signum: i32,
    action: Option<&SignalAction>,
    old_action: Option<&mut SignalAction>,
) -> isize {
    sys_sigaction(
        signum,
        action.map_or(core::ptr::null(), |a| a),
        old_action.map_or(core::ptr::null_mut(), |a| a),
    )
}

/// 设置/清空信号屏蔽字（返回旧的）
pub fn sigprocmask(mask: u32) -> isize {
    sys_sigprocmask(mask)
}

/// 从信号处理函数返回（必须调用，否则处理函数返回不了）
pub fn sigreturn() -> isize {
    sys_sigreturn()
}

// ---- ch6：打开标志（和 Linux/easy-fs 一致）----
pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_CREATE: u32 = 1 << 9;
pub const O_TRUNC: u32 = 1 << 10;

/// 读文件（对应 rCore 的 `pub fn read(fd: usize, buf: &mut [u8]) -> isize`）
pub fn read(fd: usize, buf: &mut [u8]) -> isize {
    sys_read(fd, buf)
}

/// 打开文件（path 必须以 '\0' 结尾）
pub fn open(path: &str, flags: u32) -> isize {
    sys_open(path, flags)
}

/// 关闭文件
pub fn close(fd: usize) -> isize {
    sys_close(fd)
}

/// 复制 fd（ch7）
pub fn dup(fd: usize) -> isize {
    sys_dup(fd)
}

/// 创建管道：`pipe_fd[0]` 读端，`pipe_fd[1]` 写端（ch7）
pub fn pipe(pipe_fd: &mut [usize]) -> isize {
    sys_pipe(pipe_fd)
}

/// 对应 rCore 的 `pub fn write(fd: usize, buf: &[u8]) -> isize`
pub fn write(fd: usize, buf: &[u8]) -> isize {
    sys_write(fd, buf)
}

/// 退出（对应 rCore 的 `pub fn exit(exit_code: i32) -> !`）
pub fn exit(exit_code: i32) -> ! {
    sys_exit(exit_code)
}

/// ch3：主动让出 CPU（对应 rCore 的 `pub fn yield_() -> isize`）
pub fn yield_() -> isize {
    sys_yield()
}

/// ch3：读毫秒时间（对应 rCore 的 `pub fn get_time() -> isize`）
pub fn get_time() -> isize {
    sys_get_time()
}

/// ch4：改 program break（对应 rCore 的 `pub fn sbrk(size: i32) -> isize`）
pub fn sbrk(size: i32) -> isize {
    sys_sbrk(size)
}

// ---------------------------------------------------------------------------
// ch5：进程相关
// ---------------------------------------------------------------------------

/// 取当前进程 pid
pub fn getpid() -> isize {
    sys_getpid()
}

/// fork：父进程返回子进程 pid，子进程返回 0
pub fn fork() -> isize {
    sys_fork()
}

/// exec：把当前进程换成另一个 app。`path` 必须带 '\0' 结尾（如 "forktest\0"）
pub fn exec(path: &str) -> isize {
    sys_exec(path)
}

/// 等任意子进程（对应 rCore 的 `pub fn wait(exit_code: &mut i32) -> isize`）
pub fn wait(exit_code: &mut i32) -> isize {
    loop {
        match sys_waitpid(-1, exit_code as *mut _) {
            -2 => {
                yield_(); // 有子进程但还活着 → 让出 CPU 再试
            }
            pid => return pid, // -1（没子进程）或真实 pid
        }
    }
}

/// 等指定子进程
pub fn waitpid(pid: usize, exit_code: &mut i32) -> isize {
    loop {
        match sys_waitpid(pid as isize, exit_code as *mut _) {
            -2 => {
                yield_();
            }
            pid => return pid,
        }
    }
}
