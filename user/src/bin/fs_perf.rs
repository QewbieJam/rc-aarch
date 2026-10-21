#![no_std]
#![no_main]

//! app：fs_perf（ch9）—— 块缓存效果测量
//!
//! ① 200 次 open+read+close 同一个文件；
//! ② 100 次"打开同一文件写 512B、再读回"。
//! 打印每段耗时；配合内核停机时打印的块缓存统计（命中/未命中）看 ch9 的效果。

#[macro_use]
extern crate user_lib;

use user_lib::{close, get_time, open, read, write, O_CREATE, O_RDONLY, O_WRONLY};

const READ_ROUNDS: usize = 200;
const WRITE_ROUNDS: usize = 100;

#[unsafe(no_mangle)]
fn main() -> i32 {
    let mut buf = [0u8; 512];

    // ① 反复读 hello.txt
    let t0 = get_time();
    for _ in 0..READ_ROUNDS {
        let fd = open("hello.txt\0", O_RDONLY);
        assert!(fd >= 0);
        let n = read(fd as usize, &mut buf[..64]);
        assert!(n > 0, "read 失败: {}", n);
        close(fd as usize);
    }
    let dt1 = get_time() - t0;
    println!("[fs_perf] {} 次 open+read+close（hello.txt）用时 {} ms", READ_ROUNDS, dt1);

    // ② 反复写/读同一个文件
    let t1 = get_time();
    for _ in 0..WRITE_ROUNDS {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (i * 7 + 3) as u8;
        }
        let fd = open("perf.bin\0", O_CREATE | O_WRONLY);
        assert!(fd >= 0, "open 写失败");
        assert_eq!(write(fd as usize, &buf), 512);
        close(fd as usize);

        let fd = open("perf.bin\0", O_RDONLY);
        assert!(fd >= 0, "open 读失败");
        let n = read(fd as usize, &mut buf);
        assert_eq!(n, 512);
        close(fd as usize);
    }
    let dt2 = get_time() - t1;
    println!("[fs_perf] {} 次「写 512B + 读回」用时 {} ms", WRITE_ROUNDS, dt2);

    println!("[fs_perf] 完成（看内核停机的块缓存统计）");
    0
}
