/* rc-aarch 用户程序链接脚本（ch2-C）
 * 对照 rCore ch2 的 user/src/linker.ld（那边 BASE_ADDRESS = 0x80400000）
 *
 * 我们的 app 基址 = 0xFA100000：
 *   cell3 的 cell 配置里 [1] RAM 区 = 0xfa100000 + 0x1f00000（31MB）
 *   内核 batch::load_app() 会把 app 的裸二进制拷到这里，然后 eret 到 EL0 跑
 *
 * ⚠️ 和内核 ch1 踩过的同一个坑：入口必须排最前，且 .eh_frame_hdr/.note 要 DISCARD 掉。
 */
ENTRY(_start)

BASE_ADDRESS = 0xFA100000;

SECTIONS
{
    . = BASE_ADDRESS;

    .text : {
        KEEP(*(.text.entry))       /* _start 必须在这，= 0xFA100000 */
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
