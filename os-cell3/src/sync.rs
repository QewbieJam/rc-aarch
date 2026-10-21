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

// ===========================================================================
// ch8：同步原语（互斥锁 / 信号量 / 条件变量）
//
// 和 rCore ch8 的 `os/src/sync/{mutex,semaphore,condvar}.rs` 同构：
//   - 单核，所以"锁"内部状态用 UPSafeCell 保护；
//   - 拿不到锁的线程挂到 `wait_queue`，`suspend_current_and_run_next()` 让出；
//   - unlock/up/signal 时从等待队列里 `wakeup_task()` 一个。
// ===========================================================================

use crate::task::{block_current_and_run_next, current_task, wakeup_task, TaskControlBlock};
use alloc::collections::VecDeque;
use alloc::sync::Arc;

/// 互斥锁
pub struct Mutex {
    inner: UPSafeCell<MutexInner>,
}
struct MutexInner {
    locked: bool,
    blocking: bool,
    wait_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl Mutex {
    pub fn new(blocking: bool) -> Self {
        Self {
            inner: unsafe {
                UPSafeCell::new(MutexInner {
                    locked: false,
                    blocking,
                    wait_queue: VecDeque::new(),
                })
            },
        }
    }
    /// 加锁：成功返回 true；非阻塞模式下拿不到返回 false
    pub fn lock(&self) -> bool {
        let mut inner = self.inner.exclusive_access();
        if inner.locked {
            if inner.blocking {
                inner.wait_queue.push_back(current_task().unwrap());
                drop(inner);
                block_current_and_run_next();
                true
            } else {
                false
            }
        } else {
            inner.locked = true;
            true
        }
    }
    pub fn unlock(&self) {
        let mut inner = self.inner.exclusive_access();
        inner.locked = false;
        if let Some(task) = inner.wait_queue.pop_front() {
            wakeup_task(task);
        }
    }
}

/// 信号量（计数 > 0 表示可用）
pub struct Semaphore {
    inner: UPSafeCell<SemInner>,
}
struct SemInner {
    count: isize,
    wait_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl Semaphore {
    pub fn new(count: isize) -> Self {
        Self {
            inner: unsafe {
                UPSafeCell::new(SemInner {
                    count,
                    wait_queue: VecDeque::new(),
                })
            },
        }
    }
    pub fn up(&self) {
        let mut inner = self.inner.exclusive_access();
        inner.count += 1;
        if let Some(task) = inner.wait_queue.pop_front() {
            wakeup_task(task);
        }
    }
    pub fn down(&self) {
        let mut inner = self.inner.exclusive_access();
        if inner.count > 0 {
            inner.count -= 1;
        } else {
            inner.wait_queue.push_back(current_task().unwrap());
            drop(inner);
            block_current_and_run_next();
        }
    }
}

/// 条件变量
pub struct Condvar {
    inner: UPSafeCell<CondvarInner>,
}
struct CondvarInner {
    wait_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl Condvar {
    pub fn new() -> Self {
        Self {
            inner: unsafe {
                UPSafeCell::new(CondvarInner {
                    wait_queue: VecDeque::new(),
                })
            },
        }
    }
    /// 唤醒一个等待者
    pub fn signal(&self) {
        let mut inner = self.inner.exclusive_access();
        if let Some(task) = inner.wait_queue.pop_front() {
            wakeup_task(task);
        }
    }
    /// 等待（调用前应持有配套的 mutex；退出前会重新拿锁）
    pub fn wait(&self) {
        let mut inner = self.inner.exclusive_access();
        inner.wait_queue.push_back(current_task().unwrap());
        drop(inner);
        block_current_and_run_next();
    }
}
