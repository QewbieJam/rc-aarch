#![no_std]
#![no_main]

//! app_3：验证 sbrk（ch4）—— 对照 rCore ch4 的 `user/src/bin/sbrk_test.rs`
//!
//! 分页之后，`sbrk` 是唯一能让用户程序"向内核要新页"的手段：
//! 内核在 program_brk 处 append 新的虚拟页（背后是新分配的物理页帧）。
//!
//! 期望：
//!   brk 初始 = 用户栈顶（0x16000 附近）
//!   sbrk(4096) 返回旧 brk，且新页可读可写
//!   sbrk(8192) 再扩两页，跨页写也不能炸

#[macro_use]
extern crate user_lib;

use user_lib::sbrk;

#[unsafe(no_mangle)]
fn main() -> i32 {
    let old = sbrk(0);
    println!("[app3] sbrk_test: 初始 brk = {:#x}", old as usize);

    // 扩一页，返回旧 brk（= 新页的起始地址）
    let p1 = sbrk(4096);
    println!("[app3] sbrk(4096) -> {:#x}", p1 as usize);
    assert_eq!(p1, old, "sbrk 应该返回旧的 brk");
    unsafe {
        let p = p1 as *mut u8;
        p.write_volatile(0xAB);
        p.add(4095).write_volatile(0xCD);
        assert_eq!(p.read_volatile(), 0xAB);
        assert_eq!(p.add(4095).read_volatile(), 0xCD);
    }
    println!("[app3] 新页可读可写 ✓");

    // 再扩两页，写到第二页末尾（跨页验证）
    let p2 = sbrk(8192);
    println!("[app3] sbrk(8192) -> {:#x}", p2 as usize);
    unsafe {
        let p = p2 as *mut u8;
        p.add(8191).write_volatile(0x5A);
        assert_eq!(p.add(8191).read_volatile(), 0x5A);
    }
    println!("[app3] 跨页读写 OK ✓");

    // 缩回去（缩太小应该失败）
    let bad = sbrk(-0x100000);
    println!("[app3] sbrk(-0x100000) = {}（-1 表示失败，符合预期）", bad);
    assert_eq!(bad, -1);
    0
}
