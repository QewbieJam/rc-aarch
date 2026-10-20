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
    fn read(&self, _buf: UserBuffer) -> usize {
        // TODO(ch8?)：接 UART 输入。现在没有输入通道，直接返回 0（EOF）
        0
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
    fn write(&self, buf: UserBuffer) -> usize {
        let n = buf.len();
        for buffer in buf.buffers.iter() {
            for &b in buffer.iter() {
                crate::console::uart_putc(b);
            }
        }
        n
    }
}
