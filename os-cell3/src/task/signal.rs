//! 信号（ch7）—— 对照 rCore ch7 的 `os/src/task/signal.rs`
//!
//! 一个 u32 位图就够 31 个信号（Linux 的编号：SIGKILL=9、SIGUSR1=10…）。
//! 注意 upstream 用的是 `bitflags` crate，我们手写同样的常量 + 方法（不引依赖）。

pub const MAX_SIG: usize = 31;

/// 信号位图（第 n 位 = 第 n 号信号）
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct SignalFlags(pub u32);

#[allow(dead_code)]
impl SignalFlags {
    pub const SIGDEF: Self = Self(1 << 0);
    pub const SIGHUP: Self = Self(1 << 1);
    pub const SIGINT: Self = Self(1 << 2);
    pub const SIGQUIT: Self = Self(1 << 3);
    pub const SIGILL: Self = Self(1 << 4);
    pub const SIGTRAP: Self = Self(1 << 5);
    pub const SIGABRT: Self = Self(1 << 6);
    pub const SIGBUS: Self = Self(1 << 7);
    pub const SIGFPE: Self = Self(1 << 8);
    pub const SIGKILL: Self = Self(1 << 9);
    pub const SIGUSR1: Self = Self(1 << 10);
    pub const SIGSEGV: Self = Self(1 << 11);
    pub const SIGUSR2: Self = Self(1 << 12);
    pub const SIGPIPE: Self = Self(1 << 13);
    pub const SIGALRM: Self = Self(1 << 14);
    pub const SIGTERM: Self = Self(1 << 15);
    pub const SIGCHLD: Self = Self(1 << 17);
    pub const SIGCONT: Self = Self(1 << 18);
    pub const SIGSTOP: Self = Self(1 << 19);

    pub const fn empty() -> Self {
        Self(0)
    }
    /// 从位图构造；超出合法范围（>31 位）返回 None
    pub fn from_bits(bits: u32) -> Option<Self> {
        // 0 = 空集（sigprocmask(0) 用来清空屏蔽字），合法
        if bits & 0x8000_0000 != 0 { None } else { Some(Self(bits)) }
    }
    pub fn bits(self) -> u32 {
        self.0
    }
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// 把"错误类信号"映射成退出码（对照 upstream 的 `check_error`）
    pub fn check_error(self) -> Option<(i32, &'static str)> {
        if self.contains(Self::SIGINT) {
            Some((-2, "Killed, SIGINT=2"))
        } else if self.contains(Self::SIGILL) {
            Some((-4, "Illegal Instruction, SIGILL=4"))
        } else if self.contains(Self::SIGABRT) {
            Some((-6, "Aborted, SIGABRT=6"))
        } else if self.contains(Self::SIGFPE) {
            Some((-8, "Erroneous Arithmetic Operation, SIGFPE=8"))
        } else if self.contains(Self::SIGKILL) {
            Some((-9, "Killed, SIGKILL=9"))
        } else if self.contains(Self::SIGSEGV) {
            Some((-11, "Segmentation Fault, SIGSEGV=11"))
        } else {
            None
        }
    }
}

impl core::ops::BitOr for SignalFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl core::ops::BitOrAssign for SignalFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}
