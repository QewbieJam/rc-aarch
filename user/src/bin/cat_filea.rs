#![no_std]
#![no_main]

//! app：cat_filea（ch6）—— 对照 rCore ch6 的 `user/src/bin/cat_filea.rs`
//!
//! 从文件系统里打开 `hello.txt`（内核启动时建好的），读出来打印到标准输出。

#[macro_use]
extern crate user_lib;

use user_lib::{O_RDONLY, close, open, read, write};

#[unsafe(no_mangle)]
fn main() -> i32 {
    let fd = open("hello.txt\0", O_RDONLY);
    if fd < 0 {
        println!("[cat] 打开 hello.txt 失败（fd={}）", fd);
        return -1;
    }
    let mut buf = [0u8; 256];
    let n = read(fd as usize, &mut buf);
    close(fd as usize);
    println!("[cat] hello.txt 共 {} 字节：", n);
    write(1, &buf[..n as usize]);
    println!("[cat] 结束");
    0
}
