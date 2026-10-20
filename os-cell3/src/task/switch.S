/* 任务切换（ch3）—— 对照 rCore ch3 的 os/src/task/switch.S
 *
 *   RISC-V (upstream)                     →  AArch64 (我们)
 *   --------------------------------------   --------------------------------------
 *   sd sp, 8(a0)                              str  x2, [x0, #8]     （存 SP_EL1）
 *   sd ra, 0(a0)                              str  x30, [x0, #0]    （存 lr）
 *   sd s0..s11, (n+2)*8(a0)                   stp  x19..x29, ...   （11 个 callee-saved）
 *   ld ra, 0(a1) / ld sp, 8(a1)               ldr  x30, [x1,#0] / ldr x2,[x1,#8]
 *   ret                                       ret
 *
 * 为什么一定要用汇编写？（rCore 书里也问了同样的问题）
 *   因为"换栈 + 换返回地址"这个动作本身不能用 C/Rust 的调用约定表达：
 *   编译器会在函数返回时用自己保存的 lr/sp 覆盖掉我们换过的值。
 *
 * 布局必须和 context.rs 的 TaskContext 严格一致：
 *   0   ra (x30)
 *   8   sp
 *   16  s[0]  = x19      24  s[1]  = x20
 *   32  s[2]  = x21      40  s[3]  = x22
 *   48  s[4]  = x23      56  s[5]  = x24
 *   64  s[6]  = x25      72  s[7]  = x26
 *   80  s[8]  = x27      88  s[9]  = x28
 *   96  s[10] = x29 (fp)
 */

    .section .text
    .global __switch
__switch:
    /* __switch(current_task_cx_ptr: *mut TaskContext,   // x0
     *          next_task_cx_ptr:    *const TaskContext)  // x1
     */

    /* ---- 保存当前任务的上下文 ---- */
    stp     x19, x20, [x0, #16]
    stp     x21, x22, [x0, #32]
    stp     x23, x24, [x0, #48]
    stp     x25, x26, [x0, #64]
    stp     x27, x28, [x0, #80]
    str     x29,      [x0, #96]
    str     x30,      [x0, #0]          /* ra：__switch 返回时跳哪 */
    mov     x2, sp
    str     x2,       [x0, #8]          /* sp：内核栈指针 */

    /* ---- 装载下一个任务的上下文 ---- */
    /* 先把 sp 换掉（x1 指向的 TaskContext 在 TASK_MANAGER 静态区，换栈不影响读它）*/
    ldr     x2,  [x1, #8]
    mov     sp,  x2

    ldr     x29,      [x1, #96]
    ldp     x19, x20, [x1, #16]
    ldp     x21, x22, [x1, #32]
    ldp     x23, x24, [x1, #48]
    ldp     x25, x26, [x1, #64]
    ldp     x27, x28, [x1, #80]
    ldr     x30,      [x1, #0]
    ret                                 /* 跳到 x30：__restore_sp 或 __switch 的调用点 */
