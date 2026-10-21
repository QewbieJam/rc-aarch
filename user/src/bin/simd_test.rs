#![no_std]
#![no_main]

//! app：simd_test（ch7）—— SIMD 现场是不是跨系统调用保住了？
//!
//! 背景（移植时踩到的坑）：AArch64 的 EL0 和 EL1 **共用 SIMD/FP 寄存器**，
//! 用户程序把常量放进 v0~v3、中间做一次系统调用，内核的 memcpy/格式化输出
//! 会用 q0~q7 —— 如果内核不保存用户现场，回到用户态后计算就悄悄错了。
//!
//! 这个 app 把这几件事固定成验收：
//!   ① EL0 能正常执行 ldr q / movi / mul / str q；
//!   ② 向量寄存器里放的值，跨一次系统调用（getpid）后一模一样；
//!   ③ 被 LLVM 自动向量化的循环算出来的结果逐字节正确。

#[macro_use]
extern crate user_lib;

use user_lib::getpid;

/// 用内联汇编把一个常量向量放进 v7，读出到数组
unsafe fn set_v7(val: u8) {
    let v = val as u64 * 0x0101_0101_0101_0101;
    core::arch::asm!("dup v7.2d, {v}", v = in(reg) v, out("v7") _);
}

unsafe fn get_v7(out: &mut [u8; 16]) {
    core::arch::asm!("str q7, [{p}]", p = in(reg) out.as_mut_ptr(), out("v7") _);
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    // ① EL0 基本 SIMD 指令
    let mut out = [0u8; 16];
    unsafe {
        core::arch::asm!(
            "movi v0.16b, #7",
            "movi v1.16b, #3",
            "mul v2.16b, v0.16b, v1.16b",
            "str q2, [{dst}]",
            dst = in(reg) out.as_mut_ptr(),
            out("q0") _, out("v1") _, out("q2") _,
        );
        assert_eq!(out, [21u8; 16], "movi/mul 结果不对");
    }
    println!("[simd] ① movi/mul/str q 正常");

    // ② 向量寄存器跨系统调用不丢
    unsafe { set_v7(0xab) };
    let _ = getpid(); // 进内核绕一圈
    let mut out2 = [0u8; 16];
    unsafe { get_v7(&mut out2) };
    assert_eq!(out2, [0xabu8; 16], "v7 被内核改坏了：{:?}", out2);
    // 多绕几次，顺便测调度（getpid 不会让出，但 IRQ 会进来）
    for _ in 0..8 {
        let _ = getpid();
    }
    unsafe { get_v7(&mut out2) };
    assert_eq!(out2, [0xabu8; 16], "v7 在多次系统调用后被改坏");
    println!("[simd] ② 向量寄存器跨系统调用保持 ✓");

    // ③ 自动向量化的循环：3000 字节填充，逐字节比对
    let mut big = [0u8; 3000];
    for (i, b) in big.iter_mut().enumerate() {
        *b = (i * 7 + 3) as u8;
    }
    let mut bad = 0usize;
    for (i, &b) in big.iter().enumerate() {
        if b != (i * 7 + 3) as u8 {
            bad += 1;
        }
    }
    assert_eq!(bad, 0, "向量化填充有 {} 字节错位", bad);
    println!("[simd] ③ 自动向量化填充 3000 字节全对");
    println!("[simd] simd_test passed.");
    0
}
