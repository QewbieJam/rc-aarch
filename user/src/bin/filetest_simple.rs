#![no_std]
#![no_main]

//! app：filetest_simple（ch6）—— 对照 rCore ch6 的 `user/src/bin/filetest_simple.rs`
//!
//! 验收 easy-fs 的**写路径**：创建文件 → 写 → 关闭 → 重新打开 → 读回 → 打印。

#[macro_use]
extern crate user_lib;

use user_lib::{O_CREATE, O_RDONLY, O_WRONLY, close, open, read, write};

#[unsafe(no_mangle)]
fn main() -> i32 {
    // ① 创建并写入
    let fd = open("filea\0", O_CREATE | O_WRONLY);
    println!("[filetest] open(filea, CREATE|WRONLY) = {}", fd);
    if fd < 0 {
        println!("[filetest] 创建失败");
        return -1;
    }
    let msg = b"Hello, filea! This file lives on easy-fs.\n";
    let n = write(fd as usize, msg);
    println!("[filetest] 写入 {} 字节", n);
    close(fd as usize);

    // ② 重新打开读回
    let fd = open("filea\0", O_RDONLY);
    println!("[filetest] open(filea, RDONLY) = {}", fd);
    if fd < 0 {
        return -1;
    }
    let mut buf = [0u8; 128];
    let n = read(fd as usize, &mut buf);
    println!("[filetest] 读回 {} 字节：", n);
    write(1, &buf[..n as usize]);
    close(fd as usize);

    // ③ 再次打开并截断（CREATE 会对已存在的文件做 clear）
    let fd = open("filea\0", O_CREATE | O_WRONLY);
    close(fd as usize);
    let fd = open("filea\0", O_RDONLY);
    let n = read(fd as usize, &mut buf);
    close(fd as usize);
    println!("[filetest] 截断后读回 {} 字节（应该是 0）", n);

    println!("[filetest] pass.");
    0
}
