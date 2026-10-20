#![no_std]
#![no_main]

//! app：pipetest（ch7）—— 对照 rCore ch7 的 `user/src/bin/pipetest.rs`
//!
//! 父子进程通过一根管道传一句话：子进程读、父进程写。
//! 同时验证：pipe 拿到的 fd 是 3/4；关闭写端后读端读到 EOF 不会卡死。

#[macro_use]
extern crate user_lib;

use user_lib::{close, fork, pipe, read, wait, write};

static STR: &str = "Hello, world!";

#[unsafe(no_mangle)]
pub fn main() -> i32 {
    let mut pipe_fd = [0usize; 2];
    assert_eq!(pipe(&mut pipe_fd), 0);
    println!("[pipetest] pipe: read_fd={} write_fd={}", pipe_fd[0], pipe_fd[1]);
    assert_eq!(pipe_fd[0], 3);
    assert_eq!(pipe_fd[1], 4);

    if fork() == 0 {
        // 子进程：读
        close(pipe_fd[1]); // 关掉写端，父进程关完后这里会 EOF
        let mut buffer = [0u8; 32];
        let len_read = read(pipe_fd[0], &mut buffer) as usize;
        close(pipe_fd[0]);
        assert_eq!(
            core::str::from_utf8(&buffer[..len_read]).unwrap(),
            STR
        );
        println!("[pipetest] 子进程读到 \"{}\"，退出", STR);
        0
    } else {
        // 父进程：写
        close(pipe_fd[0]);
        assert_eq!(write(pipe_fd[1], STR.as_bytes()), STR.len() as isize);
        close(pipe_fd[1]);
        let mut code: i32 = 0;
        wait(&mut code);
        assert_eq!(code, 0);
        println!("[pipetest] passed.");
        0
    }
}
