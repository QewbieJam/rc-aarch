//! ch7+：跨 cell 共享内存（Jailhouse ivshmem）—— **phase 1：把通道打通**
//!
//! 目标（phase 1 只做这个）：证明 guest cell3 和 root cell 的用户态之间
//! 真的能收发字节；控制台语义（Stdin/Stdout/user_shell）留到 phase 2。
//!
//! ## 内存从哪来
//!
//! `cells/rpi4-rust-cell3.c` 里用 `JAILHOUSE_SHMEM_NET_REGIONS(0x6fffe000, 1)`
//! 声明了和 root cell **同一批物理页**（带 `JAILHOUSE_MEM_ROOTSHARED`）。
//! hypervisor 在 `cell_create` 时不会把这些页从 root cell 里 unmap
//! （`hypervisor/control.c`），所以两个 cell 都能访问同一块内存。
//!
//! ```text
//!   0x6fffe000  状态表（ivshmem-net 用；本阶段不碰）
//!   0x6ffff000  peer0（root）输出段   → root 写、我们读  = 我们的 RX
//!   0x7007e000  peer1（我们）输出段   → 我们写、root 读  = 我们的 TX
//! ```
//!
//! ## 段内布局（我们自己定的简单字节环，不用 virtio vring）
//!
//! ```text
//!   off 0   magic: u32   = 'RCSH'，表示这一段已经初始化
//!   off 4   write: u32   生产者写的下标（自由回绕）
//!   off 8   read:  u32   消费者写的下标
//!   off 12  cap:   u32   数据区容量
//!   off 16  data[cap]
//! ```
//!
//! 单生产者/单消费者：生产者只改 `write`，消费者只改 `read`；
//! 发布用 `stlr`（release）、观察用 `ldar`（acquire），保证"数据先于下标"可见。
//!
//! ★★ 下标归属（这一条搞反过一次，值得记下来）：
//!   jailhouse 给每个 cell 的权限是**只能写自己的输出段**（对方的段是只读的）。
//!   所以"我读到哪儿了"这个 read 下标不能写进对方的段，只能写进**自己的段**：
//!     - 段 A（root 的）= root 的 TX，里面是 root 的 write + root 对 B 的 read
//!     - 段 B（我们的）= 我们的 TX，里面是我们的 write + 我们对 A 的 read
//!   ⇒ 我们发数据：free 看 A.read（root 消费到哪儿）；收到数据：更新 B.read。
//!   VETH 协议里 rx.vr.used 和 tx.vr.used 的 swap 就是为了这个（used 环必须
//!   落在消费者自己的段里）。
//!
//! ## root 侧怎么访问
//!
//! root 的 `uio_ivshmem` 驱动把这块内存 mmap 给用户态（`/dev/uio0`）：
//!   map2（input_sections）= 0x6ffff000 起两段，只读 → 读我们的 TX
//!   map3（output_section）= 0x6ffff000 起一段，可写 → root 自己的 TX
//! 注意 dev_id=0（root）时，宏给的是 [2] 可写；root 的 map3 就是这段。

use core::arch::asm;

const MAGIC: u32 = 0x5243_5348; // 'R' 'C' 'S' 'H'

pub const SECTION_SIZE: usize = 0x7f000;
pub const RX_BASE: usize = 0x6fff_f000; // root 写 → 我们读
pub const TX_BASE: usize = 0x7007_e000; // 我们写 → root 读

const HDR: usize = 20;
const CAP: usize = SECTION_SIZE - HDR;

const OFF_MAGIC: usize = 0;
const OFF_GEN: usize = 4; // 段初始化代数：每次 init 换一个新值
const OFF_WRITE: usize = 8;
const OFF_READ: usize = 12;
const OFF_CAP: usize = 16;

/// 上一次看到的 peer（root）段的代数；变了说明 root 重新初始化了它那一段，
/// 我们记在**自己段里**的 read 下标要清零（否则会算出巨大的 used/avail）。
static mut LAST_RX_GEN: u32 = 0;

#[inline]
unsafe fn rd32(addr: usize) -> u32 {
    (addr as *const u32).read_volatile()
}

#[inline]
unsafe fn wr32(addr: usize, v: u32) {
    (addr as *mut u32).write_volatile(v);
}

/// 读一个下标（acquire：保证之后读到的数据是发布者写下的）
#[inline]
unsafe fn ld_acquire(addr: usize) -> u32 {
    let v: u32;
    asm!("ldar {v:w}, [{p}]", v = out(reg) v, p = in(reg) addr,
         options(nostack, preserves_flags));
    v
}

/// 写一个下标（release：保证之前写的数据先对读者可见）
#[inline]
unsafe fn st_release(addr: usize, v: u32) {
    asm!("stlr {v:w}, [{p}]", v = in(reg) v, p = in(reg) addr,
         options(nostack, preserves_flags));
}

/// 初始化我们自己的 TX 段（root 那段的 magic 由 root 的守护进程写）
pub fn init() {
    unsafe {
        // 代数用 CNTVCT 低 32 位：每次启动都不同，peer 借此发现"这一段被重新初始化了"
        let epoch: u32;
        asm!("mrs {g}, cntvct_el0", g = out(reg) epoch, options(nostack));
        wr32(TX_BASE + OFF_CAP, CAP as u32);
        wr32(TX_BASE + OFF_WRITE, 0);
        wr32(TX_BASE + OFF_READ, 0);
        wr32(TX_BASE + OFF_GEN, epoch);
        // 最后写 magic，并确保 root 侧能看到 cap/gen/write/read
        asm!("dsb ish", options(nostack));
        wr32(TX_BASE + OFF_MAGIC, MAGIC);
        asm!("dsb ish", "sev", options(nostack));
    }
    crate::println!(
        "[shm] TX @ {:#x}（cap {}），RX @ {:#x}",
        TX_BASE,
        CAP,
        RX_BASE
    );
}

/// 往 TX 段写字节（不阻塞：放不下的部分直接丢弃并返回实际写入数）
pub fn send(buf: &[u8]) -> usize {
    unsafe {
        let w = rd32(TX_BASE + OFF_WRITE) as usize;
        // 对方（root）消费到哪儿了 —— 记在**对方的段**里（root 写那里）
        let r = ld_acquire(RX_BASE + OFF_READ) as usize;
        let mut used = w.wrapping_sub(r);
        if used > CAP {
            // root 的 read 还是上一代的值（还没看到我们的新 gen）→ 当作"没有消费过"
            used = 0;
        }
        if used >= CAP {
            return 0; // 环满
        }
        let n = core::cmp::min(CAP - used, buf.len());
        for (i, &b) in buf[..n].iter().enumerate() {
            let idx = (w + i) % CAP;
            ((TX_BASE + HDR + idx) as *mut u8).write_volatile(b);
        }
        st_release(TX_BASE + OFF_WRITE, ((w + n) & 0xffff_ffff) as u32);
        n
    }
}

/// 从 RX 段读字节（不阻塞；返回 0 表示当前没有数据或 root 还没初始化）
pub fn recv(buf: &mut [u8]) -> usize {
    unsafe {
        if rd32(RX_BASE + OFF_MAGIC) != MAGIC {
            return 0; // root 的守护进程还没启动/初始化这一段
        }
        // root 的段是否被重新初始化过？是的话我们自己的 read 下标要清零
        let epoch = rd32(RX_BASE + OFF_GEN);
        if epoch != LAST_RX_GEN {
            LAST_RX_GEN = epoch;
            wr32(TX_BASE + OFF_READ, 0);
        }
        // 我们读到哪儿了 —— 记在**我们自己的段**里（我们写那里）
        let r = rd32(TX_BASE + OFF_READ) as usize;
        let w = ld_acquire(RX_BASE + OFF_WRITE) as usize;
        let avail = w.wrapping_sub(r);
        if avail == 0 || avail > CAP {
            return 0; // 没有数据（或者对端还没按协议来）
        }
        let n = core::cmp::min(avail, buf.len());
        for (i, b) in buf[..n].iter_mut().enumerate() {
            let idx = (r + i) % CAP;
            *b = ((RX_BASE + HDR + idx) as *const u8).read_volatile();
        }
        st_release(TX_BASE + OFF_READ, ((r + n) & 0xffff_ffff) as u32);
        n
    }
}
