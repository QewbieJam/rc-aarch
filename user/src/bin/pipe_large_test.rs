#![no_std]
#![no_main]

//! app：pipe_large_test（ch7）—— 对照 rCore ch7 的同名 app
//!
//! 管道环形缓冲区只有 32 字节，这里要传 3000 字节：
//! 写端写满就阻塞、读端取走再继续 —— 是"阻塞 + 调度"的真正考验。
//!
//! ★ 比 upstream 多做一步：子进程**逐字节**校验收到的内容。
//!   upstream 只比较校验和，两个进程的缓冲区如果同时被写坏，校验和一样也会"通过"
//!   （我们在移植时就因此漏掉了一个 SIMD 现场的 bug，见 docs/07）。

#[macro_use]
extern crate user_lib;

use user_lib::{close, fork, pipe, read, wait, write};

const LENGTH: usize = 3000;

#[inline]
fn pattern(i: usize) -> u8 {
    (i * 7 + 3) as u8
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    let mut down = [0usize; 2];
    let mut up = [0usize; 2];
    assert_eq!(pipe(&mut down), 0);
    assert_eq!(pipe(&mut up), 0);
    println!(
        "[pipe_large] down: r={} w={}, up: r={} w={}",
        down[0], down[1], up[0], up[1]
    );

    let mut data = [0u8; LENGTH];
    if fork() == 0 {
        // 子进程：从 down 读 3000 字节，逐字节校验，再从 up 发回校验和
        close(down[1]);
        close(up[0]);
        let mut got = 0usize;
        while got < LENGTH {
            let n = read(down[0], &mut data[got..]) as usize;
            if n == 0 {
                break;
            }
            got += n;
        }
        assert_eq!(got, LENGTH, "子进程只读到 {} 字节", got);
        close(down[0]);
        let mut bad = 0usize;
        for (i, &b) in data.iter().enumerate() {
            if b != pattern(i) {
                if bad < 4 {
                    println!("[pipe_large] 错位 i={} want={} got={}", i, pattern(i), b);
                }
                bad += 1;
            }
        }
        assert_eq!(bad, 0, "子进程收到 {} 字节错位", bad);
        let sum: u32 = data.iter().map(|&b| b as u32).sum();
        println!("[pipe_large] 子进程收到 {} 字节，校验和 {}", got, sum);
        write(up[1], &sum.to_le_bytes());
        close(up[1]);
        println!("[pipe_large] 子进程退出");
        0
    } else {
        // 父进程：造 3000 字节，写进 down；从 up 读 4 字节校验和
        close(down[0]);
        close(up[1]);
        for (i, b) in data.iter_mut().enumerate() {
            *b = pattern(i);
        }
        let mut sent = 0usize;
        while sent < LENGTH {
            let n = write(down[1], &data[sent..]) as usize;
            sent += n;
        }
        assert_eq!(sent, LENGTH);
        close(down[1]);
        let sum: u32 = data.iter().map(|&b| b as u32).sum();
        println!("[pipe_large] 父进程发出 {} 字节，校验和 {}", sent, sum);

        let mut buf = [0u8; 4];
        let mut got = 0usize;
        while got < 4 {
            let n = read(up[0], &mut buf[got..]) as usize;
            if n == 0 {
                break;
            }
            got += n;
        }
        close(up[0]);
        assert_eq!(got, 4);
        let child_sum = u32::from_le_bytes(buf);
        println!("[pipe_large] 父进程收到子进程校验和 {}", child_sum);
        assert_eq!(sum, child_sum);

        let mut code: i32 = 0;
        wait(&mut code);
        assert_eq!(code, 0);
        println!("[pipe_large] passed.");
        0
    }
}
