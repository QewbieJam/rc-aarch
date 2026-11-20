#![no_std]
#![no_main]

//! app：simd_test（ch7+ 回归测试）—— 系统调用会不会弄坏用户的 SIMD 现场？
//!
//! ## 这个测试在验什么
//!
//! AArch64 的 EL0（用户态）和 EL1（内核态）**共用同一套 SIMD/FP 寄存器**（q0~q31、
//! FPCR、FPSR）。用户程序把常量放进 v0~v3、中间做一次系统调用，内核自己的
//! memcpy/格式化输出也会用 q0~q7 —— 如果内核不保存用户现场，回到用户态后计算
//! 就会**悄悄算错**（不崩溃、不报错，数据就是不对）。
//!
//! 这个测试把三件事固定成验收项：
//!   ① EL0 能正常执行 ldr q / movi / mul / str q 这些基本 SIMD 指令；
//!   ② 向量寄存器里放的值，跨一次系统调用（getpid）后必须一模一样；
//!   ③ 被 LLVM 自动向量化出来的循环，算出的结果逐字节正确。
//!
//! ## 测试设计方法
//! - **错误推测 + 回归测试**：它来自移植时真实踩到的 bug，加进来是为了防止复发；
//! - **等价类**：单次系统调用、多次系统调用（含 IRQ 打断）两个场景；
//! - **内容断言**：不只看“有没有报错”，而是逐字节比对结果。
//!
//! ## 怎么看结果
//! - 通过：打印 `[simd] simd_test passed.`，进程退出码 0；
//! - 失败：`assert_eq!` 触发 panic → 进程以非 0 退出 → initproc 打印 exit_code=127
//!   → 验收脚本对应章节判 FAIL。

#[macro_use]
extern crate user_lib;

use user_lib::getpid;

/// 用内联汇编把 16 个字节的常量（val 重复 16 次）放进 v7。
///
/// 这里用内联汇编而不是普通赋值，是为了**精确控制**哪个向量寄存器被写：
/// `dup v7.2d` 把 64 位值复制到 v7 的两个 64 位通道，等价于 16 个字节都是 val。
unsafe fn set_v7(val: u8) {
    let v = val as u64 * 0x0101_0101_0101_0101;
    core::arch::asm!("dup v7.2d, {v}", v = in(reg) v, out("v7") _);
}

/// 把 v7 的 16 个字节读出来，写进 `out`。
unsafe fn get_v7(out: &mut [u8; 16]) {
    core::arch::asm!("str q7, [{p}]", p = in(reg) out.as_mut_ptr(), out("v7") _);
}

#[unsafe(no_mangle)]
fn main() -> i32 {
    // =====================================================================
    // ① EL0 基本 SIMD 指令（Arrange + Act + Assert 一体）
    // =====================================================================
    let mut out = [0u8; 16];
    unsafe {
        // Act：v0 每字节 = 7，v1 每字节 = 3，v2 = v0 * v1（按字节乘），
        //      再把 v2 的 16 个字节存进 out。
        core::arch::asm!(
            "movi v0.16b, #7",
            "movi v1.16b, #3",
            "mul v2.16b, v0.16b, v1.16b",
            "str q2, [{dst}]",
            dst = in(reg) out.as_mut_ptr(),
            out("q0") _, out("v1") _, out("q2") _,
        );
        // Assert：7 * 3 = 21，16 个字节都应该是 21。
        assert_eq!(out, [21u8; 16], "movi/mul 结果不对");
    }
    println!("[simd] ① movi/mul/str q 正常");

    // =====================================================================
    // ② 向量寄存器跨系统调用不丢（本测试的核心）
    // =====================================================================
    // Arrange：把 v7 的 16 个字节都设成 0xab。
    unsafe { set_v7(0xab) };
    // Act：做一次系统调用，进内核绕一圈（内核此时可能用 q0~q7 做 memcpy/格式化）。
    let _ = getpid();
    // Assert：回来之后 v7 必须还是 0xab × 16。
    let mut out2 = [0u8; 16];
    unsafe { get_v7(&mut out2) };
    assert_eq!(out2, [0xabu8; 16], "v7 被内核改坏了：{:?}", out2);
    // 再多绕几次：getpid 本身不让出 CPU，但定时器 IRQ 会进来，同样会进内核。
    for _ in 0..8 {
        let _ = getpid();
    }
    unsafe { get_v7(&mut out2) };
    assert_eq!(out2, [0xabu8; 16], "v7 在多次系统调用后被改坏");
    println!("[simd] ② 向量寄存器跨系统调用保持 ✓");

    // =====================================================================
    // ③ 自动向量化的循环：3000 字节填充，逐字节比对
    // =====================================================================
    // Arrange + Act：这个循环会被 LLVM 自动向量化成 SIMD 指令（这正是 bug 的现场）。
    let mut big = [0u8; 3000];
    for (i, b) in big.iter_mut().enumerate() {
        *b = (i * 7 + 3) as u8;
    }
    // Assert：逐字节比对，一个都不能错（不只看校验和）。
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
