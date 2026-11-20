#![no_std]
#![no_main]

//! app：pipe_large_test（ch7）—— 管道：32 字节环传 3000 字节
//!
//! ## 这个测试在验什么
//!
//! 管道环形缓冲区只有 32 字节，这里要传 3000 字节：
//! 写端写满就阻塞、读端取走再继续 —— 是“阻塞 + 调度切换”的真正考验。
//!
//! 验收点：
//!   1. 3000 字节穿过 32 字节的环，**不丢不重**（字节数、内容都对）；
//!   2. 写满时写端阻塞、空时读端阻塞，但不会死锁；
//!   3. 反向通道（子进程把校验和发回父进程）也能工作；
//!   4. 子进程正常退出（wait 拿到 code = 0）。
//!
//! ★ 比 upstream 多做一步：子进程**逐字节**校验收到的内容。
//!   upstream 只比较校验和——如果父子缓冲区被同一个 bug 同时写坏，校验和一样
//!   也会“通过”（我们在移植时就因此漏掉了一个 SIMD 现场的 bug，见 docs/07）。
//!
//! ## 测试设计方法
//! - **边界值**：32 字节环 vs 3000 字节数据（远超容量，强制阻塞）；
//! - **状态迁移**：空 → 有数据 → 满 → 阻塞 → 唤醒；
//! - **场景法**：fork 出父子两个进程 + 两根管道（数据 + 校验和）；
//! - **错误推测**：只比校验和不够，加逐字节比对。
//!
//! ## 怎么看结果
//! - 通过：`[pipe_large] passed.`，退出码 0；
//! - 失败：字节数不对 / 内容错位 / 死锁 → assert 或超时。

#[macro_use]
extern crate user_lib;

use user_lib::{close, fork, pipe, read, wait, write};

/// 要传的总字节数：远大于管道环的 32 字节，保证“写满 → 阻塞”一定会发生。
const LENGTH: usize = 3000;

/// 测试数据的生成规则：第 i 个字节 = (i * 7 + 3) mod 256。
///
/// 用公式而不是随机数，是为了让父子两边都能独立算出“期望值”并逐字节比对。
#[inline]
fn pattern(i: usize) -> u8 {
    (i * 7 + 3) as u8
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    // =====================================================================
    // ① 准备：两根管道（父 → 子的数据通道，子 → 父的校验和通道）
    // =====================================================================
    let mut down = [0usize; 2]; // down[0] = 读端 fd，down[1] = 写端 fd
    let mut up = [0usize; 2];   // 同上，反向通道
    assert_eq!(pipe(&mut down), 0, "第一根管道创建失败");
    assert_eq!(pipe(&mut up), 0, "第二根管道创建失败");
    println!(
        "[pipe_large] down: r={} w={}, up: r={} w={}",
        down[0], down[1], up[0], up[1]
    );

    // 3000 字节的数据缓冲（父子各一份）。
    let mut data = [0u8; LENGTH];

    // =====================================================================
    // ② 分叉：子进程读，父进程写（并发场景）
    // =====================================================================
    if fork() == 0 {
        // ------------------------- 子进程（读端）-------------------------
        // 关掉自己不需要的端：down 的写端、up 的读端。
        close(down[1]);
        close(up[0]);

        // Act：循环读，直到凑够 3000 字节。
        // 管道空时会阻塞（内核把线程挂起），有数据再被唤醒继续读。
        let mut got = 0usize;
        while got < LENGTH {
            let n = read(down[0], &mut data[got..]) as usize;
            if n == 0 {
                break; // 0 = 写端全关了（EOF）
            }
            got += n;
        }
        // Assert 1：字节数一个不能少。
        assert_eq!(got, LENGTH, "子进程只读到 {} 字节", got);
        close(down[0]);

        // Assert 2：逐字节比对内容（防“校验和相同但数据错”的假通过）。
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

        // Act：把校验和发回父进程（4 字节小端）。
        let sum: u32 = data.iter().map(|&b| b as u32).sum();
        println!("[pipe_large] 子进程收到 {} 字节，校验和 {}", got, sum);
        write(up[1], &sum.to_le_bytes());
        close(up[1]); // 关闭写端 → 父进程读 up 时会得到 EOF
        println!("[pipe_large] 子进程退出");
        0
    } else {
        // ------------------------- 父进程（写端）-------------------------
        // 关掉自己不需要的端：down 的读端、up 的写端。
        close(down[0]);
        close(up[1]);

        // Arrange：按公式造 3000 字节。
        for (i, b) in data.iter_mut().enumerate() {
            *b = pattern(i);
        }

        // Act：循环写，直到 3000 字节全部写出。
        // 管道满时会阻塞（内核把线程挂起），等读端取走后继续写。
        let mut sent = 0usize;
        while sent < LENGTH {
            let n = write(down[1], &data[sent..]) as usize;
            sent += n;
        }
        assert_eq!(sent, LENGTH);
        close(down[1]); // 关写端 → 子进程读完后得到 EOF

        // 父进程自己也算一份校验和（用于和子进程比对）。
        let sum: u32 = data.iter().map(|&b| b as u32).sum();
        println!("[pipe_large] 父进程发出 {} 字节，校验和 {}", sent, sum);

        // Act：从反向通道读回子进程的 4 字节校验和。
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
        // Assert 3：4 字节校验和收全，且两边数值一致。
        assert_eq!(got, 4);
        let child_sum = u32::from_le_bytes(buf);
        println!("[pipe_large] 父进程收到子进程校验和 {}", child_sum);
        assert_eq!(sum, child_sum);

        // Assert 4：子进程正常退出。
        let mut code: i32 = 0;
        wait(&mut code);
        assert_eq!(code, 0);
        println!("[pipe_large] passed.");
        0
    }
}
