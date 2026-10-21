//! 标准输入输出（ch6/ch7）
//!
//! ch7 把它们也做成 `File`，这样 fd 0/1/2 和文件/管道走同一套分发逻辑。
//! （ch6 是直接在 `sys_write` 里特判 fd==1 的。）

use super::File;
use crate::mm::UserBuffer;

/// 标准输入
pub struct Stdin;

/// 标准输出
pub struct Stdout;

impl File for Stdin {
    fn readable(&self) -> bool {
        true
    }
    fn writable(&self) -> bool {
        false
    }
    /// ch7+ phase 2：从 ivshmem 共享内存读输入（root 侧的守护进程把 ssh 的 stdin
    /// 转发进来）。没有数据就 `suspend_current_and_run_next()` 让出 CPU，回来再看，
    /// 和 ch7 的 pipe 读端是同一套阻塞模型。
    fn read(&self, buf: UserBuffer) -> usize {
        let want = buf.len();
        if want == 0 {
            return 0;
        }
        // ★ 只取用户缓冲区放得下的字节数！多取的会丢（曾经用 1 字节缓冲读，
        //   一次把环里 27 字节全吃进来、只拷 1 字节，剩下的直接丢了）。
        let cap = core::cmp::min(want, 128);
        let mut tmp = [0u8; 128];
        let mut got;
        loop {
            got = crate::ivshmem::recv(&mut tmp[..cap]);
            if got > 0 {
                break;
            }
            crate::task::suspend_current_and_run_next();
        }
        // 再尽量多拿一些（不阻塞），凑满这次 read
        if got < cap {
            got += crate::ivshmem::recv(&mut tmp[got..cap]);
        }
        // 拷贝到用户缓冲区（可能少于 want，调用方自己循环）
        let mut it = buf.into_iter();
        let mut written = 0;
        for &b in &tmp[..got] {
            match it.next() {
                Some(p) => unsafe {
                    *p = b;
                    written += 1;
                },
                None => break,
            }
        }
        written
    }
    fn write(&self, _buf: UserBuffer) -> usize {
        panic!("不能写 stdin！");
    }
}

impl File for Stdout {
    fn readable(&self) -> bool {
        false
    }
    fn writable(&self) -> bool {
        true
    }
    fn read(&self, _buf: UserBuffer) -> usize {
        panic!("不能读 stdout！");
    }
    /// ch7+ phase 2：stdout 同时写 UART 和 ivshmem 的 TX 段。
    ///
    ///   - UART：保持内核日志/验收输出仍能在部署脚本的串口控制台里看到；
    ///   - ivshmem：root 侧的守护进程读到后打到它自己的 stdout（= ssh 会话里能看到）。
    /// 两者是"并行"的，不是二选一 —— 移植/调试阶段两边都要看。
    fn write(&self, buf: UserBuffer) -> usize {
        let n = buf.len();
        let mut chunk = [0u8; 256];
        let mut used = 0;
        for buffer in buf.buffers.iter() {
            for &b in buffer.iter() {
                crate::console::uart_putc(b);
                chunk[used] = b;
                used += 1;
                if used == chunk.len() {
                    crate::ivshmem::send(&chunk[..used]);
                    used = 0;
                }
            }
        }
        if used > 0 {
            crate::ivshmem::send(&chunk[..used]);
        }
        n
    }
}
