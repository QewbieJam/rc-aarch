/* rc-aarch 用户程序链接脚本（ch3）
 * 对照 rCore ch3 的 user/src/linker.ld（那边 BASE_ADDRESS = 0x80400000）
 *
 * ★★ ch3 的关键变化：**每个 app 要链接到各自的加载地址**
 *   ch2 的批处理一次只跑一个 app，所有 app 都装在同一个地址，所以"都链接到 0xFA100000"
 *   没毛病。ch3 的多个任务要**同时**驻留在内存里：
 *        app_i 的加载地址 = 0xFA100000 + i * 0x20000
 *   如果还是全部按 0xFA100000 链接，app_1 里那些**绝对地址**（链接器符号
 *   start_bss/end_bss、.data 里的指针…）就会指向 app_0 的地盘 —— 表现为
 *   app_1 清 bss 时清掉 app_0 的数据，最后莫名其妙跑到 0x0。
 *   （upstream 的 user/build.py 就是每个 app 改一次 linker.ld 再单独 cargo build。）
 *
 *   我们的做法更省事：链接脚本里 BASE_ADDRESS 允许被命令行覆盖
 *   （build.rs 传 `--defsym=__app_base=...`），一个脚本搞定所有 app。
 *
 * 我们的 app 基址 = 0xFA100000：
 *   cell3 的 cell 配置里 [1] RAM 区 = 0xfa100000 + 0x1f00000（31MB）
 *   内核 batch::load_app() 会把 app 的裸二进制拷到这里，然后 eret 到 EL0 跑
 *
 * ⚠️ 和内核 ch1 踩过的同一个坑：入口必须排最前，且 .eh_frame_hdr/.note 要 DISCARD 掉。
 */
ENTRY(_start)

/* 默认 0xFA100000；build.rs 会用 --defsym=__app_base=0x... 覆盖成每个 app 自己的地址 */
BASE_ADDRESS = DEFINED(__app_base) ? __app_base : 0xFA100000;

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
