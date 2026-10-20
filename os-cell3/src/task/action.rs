//! 信号处理动作（ch7）—— 对照 rCore ch7 的 `os/src/task/action.rs`
//!
//! `SignalAction` 要和用户态 ABI 完全一致（用户 `sigaction` 传进来的结构体
//! 会被内核按字节读），所以用 `#[repr(C, align(16))]`，字段顺序和 upstream 相同：
//!   handler: usize（0 = 默认动作，1 = 忽略）
//!   mask: u32（处理这个信号时额外屏蔽哪些信号）

use super::signal::{MAX_SIG, SignalFlags};

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy)]
pub struct SignalAction {
    pub handler: usize,
    pub mask: SignalFlags,
}

impl Default for SignalAction {
    fn default() -> Self {
        Self {
            handler: 0,
            mask: SignalFlags::empty(),
        }
    }
}

#[derive(Clone)]
pub struct SignalActions {
    pub table: [SignalAction; MAX_SIG + 1],
}

impl Default for SignalActions {
    fn default() -> Self {
        Self {
            table: [SignalAction::default(); MAX_SIG + 1],
        }
    }
}
