//! 单核用的"内部可变性"包装（ch2-C）
//!
//! 对照 rCore ch2 的 `os/src/sync/up.rs`（UPSafeCell）：
//! 单核环境下不需要真锁，用 `RefCell` + 手动声明 `Sync` 即可。
//! 好处是"取可变借用"这个动作显式化（`exclusive_access()`），
//! 用法和 rCore 完全一致，方便逐行对照。

use core::cell::{RefCell, RefMut};

pub struct UPSafeCell<T> {
    inner: RefCell<T>,
}

unsafe impl<T> Sync for UPSafeCell<T> {}

impl<T> UPSafeCell<T> {
    /// # Safety
    /// 只在单核、且确认没有重入时使用
    pub const unsafe fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }

    pub fn exclusive_access(&self) -> RefMut<'_, T> {
        self.inner.borrow_mut()
    }
}
