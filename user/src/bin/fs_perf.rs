#![no_std]
#![no_main]

//! app：fs_perf（ch9）—— 块缓存效果测量
//!
//! ## 这个测试在验什么
//!
//! 用“反复读写同一个文件”把文件系统的调用路径跑热，然后看两样东西：
//!   ① 每段操作的耗时（性能）；
//!   ② 内核停机时打印的块缓存统计（命中率）——这是 ch9 的效果指标。
//!
//! 两段场景：
//!   ① 200 次 open + read + close 同一个文件（读路径）；
//!   ② 100 次“写 512 字节 → 读回 512 字节”（写 + 读路径，同时校验内容长度）。
//!
//! ## 测试设计方法
//! - **重复/稳定性测试**：同一个操作重复几百次，放大缓存的影响；
//! - **等价类**：只读、只写、读写混合三类路径；
//! - **性能观测**：用 `get_time()` 测耗时；缓存命中率在停机统计里看。
//!
//! ## 怎么看结果
//! - 通过：打印 `[fs_perf] 完成`，退出码 0；
//! - 失败：`assert!` 失败（open/read/write 返回不对）→ 非 0 退出。
//! - 额外的观测点：停机时 `[kernel] 块缓存：命中 … / 未命中 …（命中率 83%）`。

#[macro_use]
extern crate user_lib;

use user_lib::{close, get_time, open, read, write, O_CREATE, O_RDONLY, O_WRONLY};

/// 读场景的重复次数（放大“打开 + 读取”路径的调用量）。
const READ_ROUNDS: usize = 200;
/// 写场景的重复次数（放大“写 + 读回”路径的调用量）。
const WRITE_ROUNDS: usize = 100;

#[unsafe(no_mangle)]
fn main() -> i32 {
    // 512 字节缓冲：正好是一个块的大小，方便测“一个块”的读写。
    let mut buf = [0u8; 512];

    // =====================================================================
    // ① 反复读 hello.txt（读路径 + 缓存命中）
    // =====================================================================
    let t0 = get_time(); // 开始计时（毫秒）
    for _ in 0..READ_ROUNDS {
        // Act：打开只读（路径必须以 \0 结尾）。
        let fd = open("hello.txt\0", O_RDONLY);
        assert!(fd >= 0);
        // Act：读 64 字节（文件本身是 32 字节，读到 >0 即可）。
        let n = read(fd as usize, &mut buf[..64]);
        assert!(n > 0, "read 失败: {}", n);
        // Act：关闭（下一次 open 会重新走一遍完整路径）。
        close(fd as usize);
    }
    let dt1 = get_time() - t0;
    // 观测：200 次“打开+读+关闭”的总耗时。
    println!("[fs_perf] {} 次 open+read+close（hello.txt）用时 {} ms", READ_ROUNDS, dt1);

    // =====================================================================
    // ② 反复写/读同一个文件（写路径 + 读回校验）
    // =====================================================================
    let t1 = get_time();
    for _ in 0..WRITE_ROUNDS {
        // Arrange：造 512 字节的测试数据（和 pipe_large 同一个模式）。
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (i * 7 + 3) as u8;
        }
        // Act：创建/打开文件，写 512 字节。
        let fd = open("perf.bin\0", O_CREATE | O_WRONLY);
        assert!(fd >= 0, "open 写失败");
        assert_eq!(write(fd as usize, &buf), 512);
        close(fd as usize);

        // Act：重新打开，读回 512 字节。
        let fd = open("perf.bin\0", O_RDONLY);
        assert!(fd >= 0, "open 读失败");
        let n = read(fd as usize, &mut buf);
        // Assert：读到的字节数必须正好 512（文件系统没有截断/丢数据）。
        assert_eq!(n, 512);
        close(fd as usize);
    }
    let dt2 = get_time() - t1;
    // 观测：100 次“写 512B + 读回”的总耗时。
    println!("[fs_perf] {} 次「写 512B + 读回」用时 {} ms", WRITE_ROUNDS, dt2);

    // 提示：真正的缓存效果看内核停机时打印的命中率。
    println!("[fs_perf] 完成（看内核停机的块缓存统计）");
    0
}
