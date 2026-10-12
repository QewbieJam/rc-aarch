/* rc-aarch 阶段1 链接脚本
 * 对照 rCore ch1 的 src/linker-qemu.ld，逐段对应：
 *   rCore 用 0x80200000（RustSBI 加载内核的地址）
 *   我们用 0x40080000（QEMU virt 的 RAM 从 0x40000000 开始，留 512KB 间隙）
 */
ENTRY(_start)

SECTIONS
{
    . =0xFA000000;
    /* . = 0x40080000; */

    /* ---- 代码段 ---- */
    .text : {
        /* ★ 关键：入口必须排在所有代码最前面
         *   对应 rCore 的 *(.text.entry) —— 这就是那次 0xB4 事故的教训：
         *   没有这一行，_start 的位置就"看运气" */
        KEEP(*(.text.entry))
        *(.text .text.*)
    }

    /* ---- 异常向量表（ch2）----
     * ★ 必须同时满足三件事：
     *   1) 2KB 对齐 —— VBAR_EL1 低 11 位必须为 0（表长 16 × 0x80 = 0x800）
     *   2) 绝不能排在 _start 前面！cell 配置里 cpu_reset_address = 0xFA000000 是写死的，
     *      若向量表占了 0xFA000000，上电就会去"执行向量表"而不是 _start
     *      （复习 §5.4：linker 基址 / LOADABLE 区 / cpu_reset_address 三个地址必须对齐）
     *   3) 段名**不能**叫 .text.vectors —— 上面 .text 的通配符 `*(.text.*)` 会先把它吞掉。
     *      （本次就踩了这个坑：被吞掉后靠 asm 里的 .balign 侥幸仍然对齐，但那就成了
     *        "碰巧对"：哪天有人删掉 .balign，链接出来的向量表就会悄悄错位。）
     *      所以独立命名 .vectors，并在这里显式 ALIGN(2048)。*/
    .vectors : ALIGN(2048) {
        KEEP(*(.vectors))
    }

    /* ---- 只读数据（字符串常量等）---- */
    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    /* ---- 已初始化数据 ---- */
    .data : ALIGN(4096) {
        *(.data .data.*)
    }

    /* ---- 未初始化数据 + 启动栈 ---- */
    .bss : ALIGN(4096) {
        *(.bss.stack)          /* 启动栈（对应 rCore 的 boot_stack） */
        __bss_start = .;       /* clear_bss() 的起点（对应 rCore 的 sbss） */
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }
    . = ALIGN(4096);
    __bss_end = .;             /* clear_bss() 的终点（对应 rCore 的 ebss） */
    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)      /* ★ 漏掉它 → 变成孤儿段挤在 .text 前面（实测偏移 0x10） */
        *(.note .note.*)      /* .note.gnu.property 之类同理 */
        *(.ARM.attributes)
        *(.comment)
    }
}
