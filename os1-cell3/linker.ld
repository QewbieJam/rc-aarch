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
