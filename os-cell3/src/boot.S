/* rc-aarch 阶段1 启动汇编
 * 逐行对照 rCore ch1 的 src/entry.asm：
 *
 *   rCore (RISC-V)                    →  rc-aarch (AArch64)
 *   ----------------------------------  ----------------------------------
 *   .section .text.entry                .section .text.entry
 *   la sp, boot_stack_top               ldr x0, =__stack_top ; mov sp, x0
 *   call rust_main                      bl rust_main
 *   (没有返回代码：rust_main -> !)       (保险：wfe 死循环)
 *   .section .bss.stack                 .section .bss.stack
 *   .space 4096 * 16                    .space 4096 * 16
 */
    .section .text.entry
    .global _start
_start:
    /* 设置栈指针：AArch64 没有 "la" 伪指令，用 ldr 从字面量池取 64 位地址 */
    ldr x0, =__stack_top
    mov sp, x0

    /* 跳进 Rust 世界（对应 rCore 的 call rust_main） */
    bl  rust_main

    /* rust_main 声明为 -> !，理论上不会返回；万一返回就停在这 */
1:  wfe
    b   1b

/* 启动栈：64KB，放在 .bss.stack
 * 注意：链接脚本把 sbss 定在栈之后，所以 clear_bss() 不会清零正在使用的栈
 *      —— 和 rCore 完全一样的设计 */
    .section .bss.stack
    .global __stack_bottom
__stack_bottom:
    .space 4096 * 16
    .global __stack_top
__stack_top:
