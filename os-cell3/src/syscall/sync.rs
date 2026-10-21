//! 同步类系统调用（ch8）—— 对照 rCore ch8 的 `os/src/syscall/sync.rs`
//!
//! 和 upstream 一样：内核里维护三个全局池（mutex / semaphore / condvar），
//! 用户拿到的 id 就是池里的下标；对象用 `Arc` 包着，可以安全地跨多个线程共享。
//!
//! 调用号（和 upstream 一致）：
//!   1000 thread_create  1001 gettid  1002 waittid
//!   1010 mutex_create   1015 mutex_lock  1020 mutex_unlock
//!   1030 sem_create     1035 sem_up      1040 sem_down
//!   1050 condvar_create 1055 condvar_signal 1060 condvar_wait

use crate::sync::{Condvar, Mutex, Semaphore, UPSafeCell};
use alloc::sync::Arc;
use alloc::vec::Vec;

static MUTEX_POOL: UPSafeCell<Vec<Option<Arc<Mutex>>>> = unsafe { UPSafeCell::new(Vec::new()) };
static SEM_POOL: UPSafeCell<Vec<Option<Arc<Semaphore>>>> = unsafe { UPSafeCell::new(Vec::new()) };
static CONDVAR_POOL: UPSafeCell<Vec<Option<Arc<Condvar>>>> =
    unsafe { UPSafeCell::new(Vec::new()) };

/// 在池里分配一个 id（复用已释放的槽位）
fn alloc_id<T>(pool: &mut Vec<Option<T>>) -> usize {
    if let Some(id) = (0..pool.len()).find(|i| pool[*i].is_none()) {
        return id;
    }
    pool.push(None);
    pool.len() - 1
}

// ---------------------------------------------------------------------------
// 互斥锁
// ---------------------------------------------------------------------------

pub fn sys_mutex_create(blocking: bool) -> isize {
    let mut pool = MUTEX_POOL.exclusive_access();
    let id = alloc_id(&mut pool);
    pool[id] = Some(Arc::new(Mutex::new(blocking)));
    id as isize
}

pub fn sys_mutex_lock(id: usize) -> isize {
    let mutex = {
        let pool = MUTEX_POOL.exclusive_access();
        match pool.get(id).and_then(|m| m.clone()) {
            Some(m) => m,
            None => return -1,
        }
    };
    if mutex.lock() {
        0
    } else {
        -1
    }
}

pub fn sys_mutex_unlock(id: usize) -> isize {
    let mutex = {
        let pool = MUTEX_POOL.exclusive_access();
        match pool.get(id).and_then(|m| m.clone()) {
            Some(m) => m,
            None => return -1,
        }
    };
    mutex.unlock();
    0
}

// ---------------------------------------------------------------------------
// 信号量
// ---------------------------------------------------------------------------

pub fn sys_semaphore_create(res_count: usize) -> isize {
    let mut pool = SEM_POOL.exclusive_access();
    let id = alloc_id(&mut pool);
    pool[id] = Some(Arc::new(Semaphore::new(res_count as isize)));
    id as isize
}

pub fn sys_semaphore_up(id: usize) -> isize {
    let sem = {
        let pool = SEM_POOL.exclusive_access();
        match pool.get(id).and_then(|s| s.clone()) {
            Some(s) => s,
            None => return -1,
        }
    };
    sem.up();
    0
}

pub fn sys_semaphore_down(id: usize) -> isize {
    let sem = {
        let pool = SEM_POOL.exclusive_access();
        match pool.get(id).and_then(|s| s.clone()) {
            Some(s) => s,
            None => return -1,
        }
    };
    sem.down();
    0
}

// ---------------------------------------------------------------------------
// 条件变量
// ---------------------------------------------------------------------------

pub fn sys_condvar_create(_arg: usize) -> isize {
    let mut pool = CONDVAR_POOL.exclusive_access();
    let id = alloc_id(&mut pool);
    pool[id] = Some(Arc::new(Condvar::new()));
    id as isize
}

pub fn sys_condvar_signal(id: usize) -> isize {
    let condvar = {
        let pool = CONDVAR_POOL.exclusive_access();
        match pool.get(id).and_then(|c| c.clone()) {
            Some(c) => c,
            None => return -1,
        }
    };
    condvar.signal();
    0
}

pub fn sys_condvar_wait(id: usize) -> isize {
    let condvar = {
        let pool = CONDVAR_POOL.exclusive_access();
        match pool.get(id).and_then(|c| c.clone()) {
            Some(c) => c,
            None => return -1,
        }
    };
    condvar.wait();
    0
}
