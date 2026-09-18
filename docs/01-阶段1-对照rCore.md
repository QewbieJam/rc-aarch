# 📘 阶段 1：AArch64 裸机 Hello —— 对照 rCore ch1 逐行讲

> 项目：rc-aarch ｜ 工程：`os1/` ｜ 状态：**构建通过，已验证 ELF 布局**（待装 QEMU 运行）
> 更新：2026-09-18

---

## 一、验收标准

| 检查项 | 状态 |
|---|---|
| `cargo build --release` 通过 | ✅ |
| `_start` 落在链接地址 `0x40080000` | ✅ |
| ELF 入口 `Entry: 0x40080000` | ✅ |
| `.bss` 段 = 64KB（启动栈） | ✅ |
| QEMU 打印 `[rc-aarch] Hello, AArch64!` | ⏳ 待装 `qemu-system-aarch64` |

---

## 二、逐行对照：rCore ch1 ↔ rc-aarch os1

### ① 启动汇编 `entry.asm` ↔ `boot.S`

| rCore（RISC-V） | rc-aarch（AArch64） | 说明 |
|---|---|---|
| `.section .text.entry` | `.section .text.entry` | 同名独立段，交给链接脚本排最前 |
| `la sp, boot_stack_top` | `ldr x0, =__stack_top` + `mov sp, x0` | AArch64 没有 `la` 伪指令，用字面量池装载 64 位地址 |
| `call rust_main` | `bl rust_main` | `bl` 把返回地址存进 **x30 (lr)**；RISC-V 的 `call` 存进 **ra** |
| `.space 4096 * 16` | `.space 4096 * 16` | 都是 64KB 启动栈，放在 `.bss.stack` |

**实测反汇编**（入口）：
```asm
40080000: ldr x0, 0x40080018        ← 从字面量池取 __stack_top
40080004: mov sp, x0                ← 设置栈指针
40080008: bl  0x40080020 <rust_main> ← 进入 Rust
4008000c: wfe                        ← 保险死循环
```

### ② 链接脚本：`linker-qemu.ld` ↔ `linker.ld`

| rCore | rc-aarch | 说明 |
|---|---|---|
| `BASE_ADDRESS = 0x80200000` | `. = 0x40080000` | RPi/RustSBI 用 0x80200000；QEMU virt 的 RAM 从 0x40000000 起 |
| `*(.text.entry)` 排第一 | `KEEP(*(.text.entry))` | ★ 同一个"入口契约" |
| `.bss : { *(.bss.stack) sbss=.; ... }` | `.bss : { *(.bss.stack) __bss_start=.; ... }` | 栈**不**被清零（同 rCore） |
| `sbss` / `ebss` | `__bss_start` / `__bss_end` | 名字不同，角色相同 |
| `/DISCARD/ { *(.eh_frame) }` | 还要加 `*(.eh_frame_hdr)` | 见第四节坑 1 |

### ③ 输出：`console.rs`+`sbi.rs` ↔ `uart_putc`

```
rCore:     println! → console::print → Stdout.write_str → sbi::console_putchar
                                                            → sbi_rt → [ecall] → RustSBI 代写 UART

rc-aarch:  print()  → uart_putc      → volatile 写 0x0900_0000（PL011 UART 数据寄存器）
```

**本质区别**：rCore 通过 **ecall 求 M-mode 固件**代劳；rc-aarch **直接写 MMIO**。
这也是 ch1 反汇编里 `li a7, 1; ecall` 与这里 `mov w8, #0x9000000; strb` 的区别。

### ④ panic：`lang_items.rs` ↔ `#[panic_handler]`

| rCore | rc-aarch |
|---|---|
| `#[panic_handler] fn panic(info) -> !` | 同 |
| `error!(...)` 打印位置 | `print("[rc-aarch] PANIC!\n")` |
| `shutdown(true)` 用失败码关机 | `loop { spin_loop() }`（裸机没处关机） |

---

## 三、实测：构建产物分析

```bash
rust-objdump -h   /home/era/.cache/rc-aarch-target/aarch64-unknown-none/release/rc-aarch-os1
rust-readobj --program-headers <elf>
```

```
段布局：
  1 .text   0x000210  VMA 0x40080000   ← _start 在这
  2 .bss    0x010000  VMA 0x40081000   ← 64KB 启动栈
（没有 .rodata！见下）

程序头（Section → Segment 的实证）：
  PT_LOAD  VMA 0x40080000  FileSize 528    MemSize 528    R E   ← .text
  PT_LOAD  VMA 0x40081000  FileSize 0      MemSize 65536  RW    ← .bss
                    ↑ 文件里一个字节都不占，内存里占 64KB —— 这就是 .bss 零填充
Entry: 0x40080000
```

### 有趣的优化：字符串被编进了指令

`rust_main` 反汇编：

```asm
4008004c: mov w8,  #0x9000000    ← UART 地址
40080050: mov w14, #0x5b         ← '[' （字符串第一个字符！）
40080054: mov w9,  #0x72         ← 'r'
```

因为字符串长度已知，LLVM 把 `&str` 直接**展开成立即数**，一个字节一条 `mov` ——
所以整个程序**连 `.rodata` 都不需要**。（这是"常量传播 + 循环展开"的结果，不是没有字符串。）

### clear_bss 变成了空操作（暂时）

```asm
4008002c: adr x8, 0x40091000 <__stack_top>
40080038: cmp x9, x8
4008003c: b.hs 0x4008004c        ← start == end → 直接跳过
```
因为 `__bss_start == __bss_end == 0x40091000`（我们的程序**还没有任何静态变量**）。
等以后加 `static mut` 变量，它们会落在 `[__bss_start, __bss_end)` 之间，清零就真的干活了。

### AArch64 寄存器对照（呼应你学的 calling conventions）

```asm
rust_main:
  stp x29, x30, [sp, #-0x10]!   ← 保存 fp(x29) + lr(x30)，对应 RISC-V 的 sd ra/s0
  mov x29, sp                    ← 设置帧指针，对应 RISC-V 的 addi s0, sp, N
```
| 角色 | RISC-V | AArch64 |
|---|---|---|
| 返回地址 | `ra` (x1) | `x30` (lr) |
| 栈指针 | `sp` (x2) | `sp` |
| 帧指针 | `s0/fp` (x8) | `x29` (fp) |

---

## 四、阶段 1 踩到的两个坑（都和 rCore 那次事故同源）

### 坑 1：`.eh_frame_hdr` 孤儿段把 `_start` 挤到 +0x10

- 现象：`_start` 在 `0x40080010` 而不是 `0x40080000`
- 原因：只 `DISCARD` 了 `.eh_frame`，漏了 **`.eh_frame_hdr`** → 它成为"孤儿段"(orphan section)
  被链接器放在当前位置（0x40080000），把 `.text` 顶后 12 字节
- 影响：ELF 加载（QEMU `-kernel` 用 `e_entry`）无害；
  **但裸二进制+固定地址加载（像 rCore 的 `os.bin` 那样）就致命** —— 入口会执行到 note 数据
- 修复：`/DISCARD/ { *(.eh_frame_hdr) *(.note .note.*) *(.ARM.attributes) }`

### 坑 2：改 `linker.ld` 不会触发重新链接

- 现象：改完链接脚本 `cargo build` 只花 0.03s，产物没变
- 原因：**链接脚本不是 cargo 跟踪的依赖**
- 修复：Makefile 的 `build` 里先 `touch src/main.rs` 强制重链
- 教训：**"源码对了 ≠ 二进制是新的"** —— 和你 ch2 那次"二进制 10:38、源码 10:39"是同一类问题

---

## 五、运行（装好 QEMU 后）

```bash
sudo apt install qemu-system-arm        # 提供 qemu-system-aarch64
cd ~/Obsidian\ Vault/Rust/rc-aarch/os1
make run
```

预期输出：
```
[rc-aarch] Hello, AArch64!
[rc-aarch] stage 1: boot + linker + bss + UART OK
```

QEMU 命令（`make run` 内部就是这条）：
```bash
qemu-system-aarch64 -machine virt -cpu cortex-a72 -nographic -kernel <elf>
```

> 为什么 `-kernel` 能加载 ELF：QEMU 按 ELF 的 program header 加载，并跳到 ELF 头里的 `Entry` 地址（0x40080000）——**这正是"Segment 层"在真实加载器里的作用**。

---

## 六、阶段 1 之后（预告）

| 下一步 | 会新增什么 |
|---|---|
| 加日志（复用 rCore 的 `logging.rs` 思路） | 分级日志 + 自定义 `Log` 实现 |
| 加异常/trap 向量（VBAR_EL1） | 对应 rCore ch2 的 `__alltraps` |
| 加 hypercall（`hvc #0`） | 对应 rCore 的 `ecall`，为阶段 2 的 Jailhouse inmate 铺路 |
