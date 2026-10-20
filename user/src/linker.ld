/* rc-aarch 用户程序链接脚本（ch4）
 * 对照 rCore ch4 的 user/src/linker.ld（那边 BASE_ADDRESS = 0x10000）
 *
 * ★★ ch4 的关键变化：**所有 app 都链接到同一个用户虚拟地址 0x10000**
 *
 *   ch3：多个任务同时驻留、共用一套物理地址，只好让 app_i 链到
 *        0xFA100000 + i*0x20000（否则绝对地址会互相踩）。
 *   ch4：每个任务有**自己的页表**，同一个 VA 在不同任务里映射到不同物理页，
 *        所以所有 app 都可以链接到 0x10000 —— 这也和 upstream ch4 一致。
 *
 *   为什么从 0x10000 开始而不是 0x0：
 *     - 低地址（0x0 附近）留给以后做空指针检测/内核共享页；
 *     - upstream ch4 也从 0x10000 起，保持一致方便对照。
 *
 *   ⚠️ 入口必须排最前，.eh_frame_hdr/.note 要 DISCARD 掉（ch1 的教训）。
 */
ENTRY(_start)

BASE_ADDRESS = 0x10000;

SECTIONS
{
    . = BASE_ADDRESS;

    .text : {
        KEEP(*(.text.entry))       /* _start 必须在这，= 0x10000 */
        *(.text .text.*)
    }
    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }
    .data : ALIGN(4096) {
        *(.data .data.*)
    }
    .bss : ALIGN(4096) {
        start_bss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        end_bss = .;
    }
    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note .note.*)
        *(.ARM.attributes)
        *(.comment)
    }
}
