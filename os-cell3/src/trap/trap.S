/* rc-aarch 阶段 2：异常向量表 + 现场保存/恢复  （ch2-A）
 *
 * 对照 rCore-Tutorial-v3 ch2 的 os/src/trap/trap.S（__alltraps / __restore）：
 *
 *   rCore                                →  rc-aarch
 *   ------------------------------------    ------------------------------------
 *   stvec 指向 __alltraps                  VBAR_EL1 指向 __vector_table
 *   （只有一个入口，靠 scause 分类）        （16 个槽位，靠"落在哪一项"分类，
 *                                            再用 ESR_EL1.EC 细分）
 *   csrrw sp, sscratch, sp 手工换栈        不用写：硬件自动从 SP_EL0 切到 SP_EL1
 *   手工 sd 存 34 个字                     stp/ldp 一次 16 字节（省一半指令）
 *   mv a0, sp ; call trap_handler          mov x0, sp ; bl trap_handler
 *   __restore + sret                       RESTORE_REGS + eret
 *
 * ★ 向量表的硬约束：
 *   - 共 16 项，每项 0x80(128) 字节，所以整表 0x800 字节、必须 2KB 对齐
 *     （VBAR_EL1 的低 11 位是 RES0）
 *   - 一个槽位只有 128 字节，而 SAVE+call+RESTORE+eret 约 170+ 字节 → 塞不下！
 *     所以每个槽位**只放一条 b**，跳到 .text 里真正的处理函数
 *     （jailhouse 自己的 EL2 向量表就是这么写的：
 *      hypervisor/arch/arm64/entry.S 的 `ventry` 宏 = .align 7 + b）
 */

    .altmacro

/* =========================================================================
 * 1. 异常向量表（独立段 .vectors，linker.ld 里 ALIGN(2048)）
 * ========================================================================= */
    .section .vectors, "ax"
    .balign 2048
    .global __vector_table
__vector_table:

    /* 每个槽位：先对齐到 128 字节，再放一条跳转 */
    .macro ventry label
    .balign 128
    b       \label
    .endm

    /* ---- 0x000  Current EL with SP_EL0 ---- */
    ventry  unhandled               /* 0x000 同步  */
    ventry  unhandled               /* 0x080 IRQ   */
    ventry  unhandled               /* 0x100 FIQ   */
    ventry  unhandled               /* 0x180 SError*/

    /* ---- 0x200  Current EL with SP_ELx  ★ 我们在 EL1 执行 svc / 出异常落这里 ---- */
    ventry  sync_handler            /* 0x200 同步 (#4) */
    ventry  irq_handler             /* 0x280 IRQ  (#5)  ch3 填 */
    ventry  unhandled               /* 0x300 FIQ   */
    ventry  unhandled               /* 0x380 SError*/

    /* ---- 0x400  Lower EL using AArch64  ★ 将来 app(EL0) 的 svc 落这里 ---- */
    ventry  sync_handler            /* 0x400 同步 (#8) */
    ventry  irq_handler             /* 0x480 IRQ  (#9)  ch3 填 */
    ventry  unhandled               /* 0x500 FIQ   */
    ventry  unhandled               /* 0x580 SError*/

    /* ---- 0x600  Lower EL using AArch32（我们不用）---- */
    ventry  unhandled
    ventry  unhandled
    ventry  unhandled
    ventry  unhandled

/* =========================================================================
 * 2. 现场保存 / 恢复
 *    偏移必须与 context.rs 的 TrapContext 字段严格对应（272 字节）
 * ========================================================================= */
    .section .text

    .macro SAVE_REGS
    sub     sp, sp, #272
    stp     x0,  x1,  [sp, #16 * 0]
    stp     x2,  x3,  [sp, #16 * 1]
    stp     x4,  x5,  [sp, #16 * 2]
    stp     x6,  x7,  [sp, #16 * 3]
    stp     x8,  x9,  [sp, #16 * 4]
    stp     x10, x11, [sp, #16 * 5]
    stp     x12, x13, [sp, #16 * 6]
    stp     x14, x15, [sp, #16 * 7]
    stp     x16, x17, [sp, #16 * 8]
    stp     x18, x19, [sp, #16 * 9]
    stp     x20, x21, [sp, #16 * 10]
    stp     x22, x23, [sp, #16 * 11]
    stp     x24, x25, [sp, #16 * 12]
    stp     x26, x27, [sp, #16 * 13]
    stp     x28, x29, [sp, #16 * 14]
    str     x30,      [sp, #240]
    /* ★ x0~x30 全部存好之后，才可以拿 x9/x10/x11 当临时寄存器去读系统寄存器
     *   （顺序反了就会把"被打断那一刻"的 x9/x10/x11 改掉）*/
    mrs     x9,  sp_el0
    mrs     x10, elr_el1
    mrs     x11, spsr_el1
    stp     x9,  x10, [sp, #248]
    str     x11,      [sp, #264]
    .endm

    .macro RESTORE_REGS
    /* 先恢复返回现场（eret 要用 elr_el1/spsr_el1）*/
    ldp     x9,  x10, [sp, #248]
    ldr     x11,      [sp, #264]
    msr     sp_el0,  x9
    msr     elr_el1, x10
    msr     spsr_el1, x11
    /* 再恢复通用寄存器（最后才动 sp）*/
    ldp     x0,  x1,  [sp, #16 * 0]
    ldp     x2,  x3,  [sp, #16 * 1]
    ldp     x4,  x5,  [sp, #16 * 2]
    ldp     x6,  x7,  [sp, #16 * 3]
    ldp     x8,  x9,  [sp, #16 * 4]
    ldp     x10, x11, [sp, #16 * 5]
    ldp     x12, x13, [sp, #16 * 6]
    ldp     x14, x15, [sp, #16 * 7]
    ldp     x16, x17, [sp, #16 * 8]
    ldp     x18, x19, [sp, #16 * 9]
    ldp     x20, x21, [sp, #16 * 10]
    ldp     x22, x23, [sp, #16 * 11]
    ldp     x24, x25, [sp, #16 * 12]
    ldp     x26, x27, [sp, #16 * 13]
    ldp     x28, x29, [sp, #16 * 14]
    ldr     x30,      [sp, #240]
    add     sp, sp, #272
    .endm

/* =========================================================================
 * 3. 槽位跳进来的地方（真正的处理路径）
 * ========================================================================= */
    .global sync_handler
sync_handler:
    SAVE_REGS
    mov     x0, sp                  /* 第一参数 = &mut TrapContext */
    bl      trap_handler            /* ← 进 Rust 世界（trap/mod.rs）*/
    RESTORE_REGS
    eret                            /* 回到被打断的那一行继续跑 */

/* __restore：从 TrapContext 恢复现场并 eret（对照 rCore 的 __restore）
 *   x0 = &TrapContext（batch 把它压在内核栈顶）
 * 用途：第一次启动 app 时"凭空"进入 EL0 —— sync_handler 返回路径是内联的，不走这里
 */
    .global __restore
__restore:
    mov     sp, x0
    RESTORE_REGS
    eret

irq_handler:
    /* ch3 在这里实现：读 GICC_IAR → 处理定时器 → 写 GICC_EOIR
     * ch2 没有中断，走到这里说明哪里不对 → 交给 panic 路径打印 */
    b       unhandled

unhandled:
    /* 不保存现场（不打算返回）：交给 Rust 打印 ESR/ELR/FAR 后死循环 */
    b       trap_unhandled

/* =========================================================================
 * 4. ch2-A 验收自测：svc 前后"寄存器完整性"检查
 *
 *    思路（对应我们说的"自测 2"）：
 *      ① 给一批寄存器填已知值 ② svc ③ 逐个比对，错一个就把 x0 加 1
 *    它能抓出 SAVE_REGS/RESTORE_REGS 里任何一个偏移写错 —— 那种错误
 *    不会崩溃，只会"悄悄改坏寄存器"，是最难查的一类。
 *
 *    被测：x1~x15、x19~x28、x30(lr)  共 26 个
 *    用作临时：x0（错误计数）、x16、x17；x29/x30 会被本函数保存
 * ========================================================================= */
    /* 给 x<n> 填已知值 0x1000+n */
    .macro FILL n
    mov     x\n, #(0x1000 + \n)
    .endm

    /* 比对 x<n> 是否还是 0x1000+n，错一个就把 x0 加 1 */
    .macro CHECK n
    mov     x16, #(0x1000 + \n)
    cmp     x\n, x16
    b.eq    9f
    add     x0, x0, #1
9:
    .endm

    .global self_test_svc
self_test_svc:
    /* 保存 callee-saved：本函数要故意乱改它们 */
    stp     x19, x20, [sp, #-16]!
    stp     x21, x22, [sp, #-16]!
    stp     x23, x24, [sp, #-16]!
    stp     x25, x26, [sp, #-16]!
    stp     x27, x28, [sp, #-16]!
    stp     x29, x30, [sp, #-16]!

    /* ① 填已知值
     * 注意：.rept 里没有宏参数替换，所以要用 FILL/CHECK 包一层，
     *       配合 .altmacro 的 %n 把符号 n 的**值**传进去（rCore 的 SAVE_GP 同款写法）*/
    .set    n, 1
    .rept   15
    FILL    %n
    .set    n, n + 1
    .endr
    .set    n, 19
    .rept   10
    FILL    %n
    .set    n, n + 1
    .endr
    mov     x30, #0x101e            /* lr 也当普通寄存器测一把 */

    /* ② 触发一次同步异常（EC=0x15 SVC64，落在向量表 #4） */
    svc     #0

    /* ③ 逐个比对 */
    mov     x0, #0                  /* 错误计数 */
    .set    n, 1
    .rept   15
    CHECK   %n
    .set    n, n + 1
    .endr
    .set    n, 19
    .rept   10
    CHECK   %n
    .set    n, n + 1
    .endr
    mov     x16, #0x101e
    cmp     x30, x16
    b.eq    3f
    add     x0, x0, #1
3:

    /* ④ 恢复 callee-saved（含真正的 lr），把错误数留在 x0 返回 */
    ldp     x29, x30, [sp], #16
    ldp     x27, x28, [sp], #16
    ldp     x25, x26, [sp], #16
    ldp     x23, x24, [sp], #16
    ldp     x21, x22, [sp], #16
    ldp     x19, x20, [sp], #16
    ret

/* =========================================================================
 * 5. EL0 探针（ch2-C 前置：决定 app 跑 EL0 还是 EL1）
 *
 *    背景：AArch64 在 **stage-1 MMU 关闭**时，内存属性被当作 Device-nGnRnE，
 *          而架构**不允许 EL0 访问/取指 Device 内存** → 所以 rCore 那种
 *          "U-mode + 不分页" 在 AArch64 上可能根本跑不起来（RISC-V 可以）。
 *
 *    做法：@eret 时把 SPSR_EL1.M 设成 0b0000（EL0t）→ 跳进一段只做 svc 的小代码：
 *      - 若 EL0 能取指：以向量表 #8（Lower EL AArch64 Sync）进来 → EC=0x15
 *      - 若不能：立即 Instruction Abort（EC=0x20，FAR=入口地址）
 * ========================================================================= */
    .section .text
    .global enter_el0
enter_el0:                          /* x0 = 入口地址, x1 = 栈顶 */
    msr     sp_el0, x1
    msr     elr_el1, x0
    mov     x2, #0x3c0              /* DAIF 全屏蔽(0b1111<<6) + M=0 → EL0t */
    msr     spsr_el1, x2
    eret                            /* ★ 第一次真正"下到 EL0" */

    .global el0_probe_stub
el0_probe_stub:
    svc     #0x42                   /* 未定义调用号 → 内核应回 -1 */
    svc     #0x43                   /* 第二次：证明 "EL0 → EL1 → 回到 EL0" 往返正常 */
    mov     x8, #93                 /* SYSCALL_EXIT */
    mov     x0, #7                  /* 退出码 7 */
    svc     #0                      /* → sys_exit(7)：ch2-B 里打印后停住 */
    b       .                       /* 不该走到这（sys_exit 不返回）*/

/* 探针用的 4KB 栈（放在 .bss.stack 里，linker 不会清零这一块）*/
    .section .bss.stack, "aw", %nobits
    .balign 16
el0_probe_stack_lower:
    .space 4096                     /* 先留出空间，标签才是"栈顶" */
    .global el0_probe_stack_top
el0_probe_stack_top:
