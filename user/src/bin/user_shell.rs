#![no_std]
#![no_main]

//! app：user_shell（ch7+ phase 2）—— 把 stdin 接上之后的第一件正经事
//!
//! 行为：
//!   1. 打印提示符 `>> `；
//!   2. 从 fd 0（内核里的 `Stdin`，背后是 ivshmem 共享内存环）读一行；
//!   3. 按空格切词，fork + exec 目标程序（easy-fs 根目录里的 app），父进程 waitpid；
//!   4. 输入 `exit` 退出；`help` 列出可用命令。
//!
//! 注意：user crate 没有堆，所有缓冲/参数数组都是固定大小（MAX_LINE / MAX_ARGS）。

#[macro_use]
extern crate user_lib;

use user_lib::{exec, exit, fork, read, waitpid};

const MAX_LINE: usize = 256;
const MAX_ARGS: usize = 8;

/// 读一行（按字节读，遇到 '\n' 结束）。返回不含 '\n' 的字节数。
fn read_line(buf: &mut [u8]) -> usize {
    let mut len = 0usize;
    while len < buf.len() {
        let n = read(0, &mut buf[len..len + 1]);
        if n <= 0 {
            // 理论上内核的 Stdin 会阻塞，不该返回 0；真返回了就继续等
            continue;
        }
        if buf[len] == b'\n' {
            break;
        }
        if buf[len] == b'\r' {
            continue;
        }
        len += 1;
    }
    len
}

fn print_cmds() {
    println!("可用命令（easy-fs 根目录里的 app）：");
    println!("  cat_filea        读 hello.txt");
    println!("  filetest_simple  文件读写测试");
    println!("  pipetest         管道测试");
    println!("  cmdline_args ... 打印 argv（试试 cmdline_args a b c）");
    println!("  forktest         fork/waitpid 测试");
    println!("  exit             退出 shell");
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    println!("[shell] rc-aarch user_shell（help 看命令，exit 退出）");
    let mut line = [0u8; MAX_LINE + 1];
    loop {
        print!(">> ");
        let len = read_line(&mut line);
        if len == 0 {
            continue;
        }
        line[len] = 0;

        // 第一遍：原地切词（把每个词尾的空白换成 '\0'），只记下区间
        let mut starts = [0usize; MAX_ARGS];
        let mut ends = [0usize; MAX_ARGS];
        let mut argc = 0usize;
        let mut i = 0usize;
        while i < len && argc < MAX_ARGS {
            while i < len && line[i] == b' ' {
                i += 1;
            }
            if i >= len {
                break;
            }
            let start = i;
            while i < len && line[i] != b' ' {
                i += 1;
            }
            line[i] = 0; // i 可能 == len（line[len] 本来就是 0，重复写无害）
            starts[argc] = start;
            ends[argc] = i;
            argc += 1;
            if i < len {
                i += 1;
            }
        }
        // 第二遍：所有修改完成之后才构造 &str（否则借用冲突）
        let mut args: [&str; MAX_ARGS] = [""; MAX_ARGS];
        for k in 0..argc {
            args[k] = core::str::from_utf8(&line[starts[k]..ends[k]]).unwrap_or("");
        }
        if argc == 0 {
            continue;
        }

        if args[0] == "exit" {
            break;
        }
        if args[0] == "help" {
            print_cmds();
            continue;
        }

        let pid = fork();
        if pid == 0 {
            exec(args[0], &args[..argc]);
            println!("[shell] exec {} 失败（easy-fs 里没有这个程序？）", args[0]);
            exit(-1);
        } else {
            let mut code: i32 = 0;
            waitpid(pid as usize, &mut code);
        }
    }
    println!("[shell] bye");
    0
}
