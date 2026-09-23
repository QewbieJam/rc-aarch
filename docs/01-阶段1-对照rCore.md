# 📘 阶段 1：AArch64 裸机 Hello —— 对照 rCore ch1 逐行讲

> 项目：rc-aarch ｜ 工程：`os1-cell3/`（MCS cell 版）；`os1/` 保留为 QEMU 版
> 状态：**✅ 已在树莓派 4B 的 openEuler MCS + Jailhouse cell3 上跑通**（2026-09-23）
> 更新：2026-09-23

---

## 一、验收标准

| 检查项 | 状态 |
|---|---|
| `cargo build --release` 通过 | ✅ |
| `_start` 落在链接地址 `0xFA000000`（cell3 的 LOADABLE 区） | ✅ |
| ELF 段全部落在 `0xFA000000~0xFB000000`（.text/.rodata/.bss） | ✅ |
| `.bss` 段 = 64KB（启动栈） | ✅ |
| **Jailhouse cell3 上打印 `[rc-aarch] Hello, AArch64!`** | ✅ 2026-09-23（MCS / RPi4B）|
| QEMU 上打印（可选，未做） | ⏳ 未装 `qemu-system-aarch64` |

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
| `BASE_ADDRESS = 0x80200000` | `os1/` = 0x40080000（QEMU virt）<br>`os1-cell3/` = **0xFA000000**（cell3 的 LOADABLE 区）| RPi/RustSBI 用 0x80200000；QEMU virt 的 RAM 从 0x40000000 起 |
| `*(.text.entry)` 排第一 | `KEEP(*(.text.entry))` | ★ 同一个"入口契约" |
| `.bss : { *(.bss.stack) sbss=.; ... }` | `.bss : { *(.bss.stack) __bss_start=.; ... }` | 栈**不**被清零（同 rCore） |
| `sbss` / `ebss` | `__bss_start` / `__bss_end` | 名字不同，角色相同 |
| `/DISCARD/ { *(.eh_frame) }` | 还要加 `*(.eh_frame_hdr)` | 见第四节坑 1 |

### ③ 输出：`console.rs`+`sbi.rs` ↔ `uart_putc`

```
rCore:     println! → console::print → Stdout.write_str → sbi::console_putchar
                                                            → sbi_rt → [ecall] → RustSBI 代写 UART

os1:       print()  → uart_putc      → volatile 写 0x0900_0000（QEMU virt 的 PL011 数据寄存器）

os1-cell3: print()  → uart_putc      → ① hvc #0x4a48 让 hypervisor 往控制台写一个字
                                     → ② volatile 写 0xFE215040（RPi4 mini-UART 数据寄存器）
                                         写之前必须轮询 0xFE215054 的 THRE 位（见 §5.3 坑 1）
```

**本质区别**：rCore 通过 **ecall 求 M-mode 固件**代劳；rc-aarch 要么**直接写 MMIO**（os1），
要么**求 EL2 的 hypervisor**（os1-cell3 的 hvc）；而 cell3 里"直接写 MMIO"也是允许的，因为 UART 已经划给了它。
这也是 ch1 反汇编里 `li a7, 1; ecall` 与这里 `mov w8, #0x9000000; strb` 的区别。

### ④ panic：`lang_items.rs` ↔ `#[panic_handler]`

| rCore | rc-aarch |
|---|---|
| `#[panic_handler] fn panic(info) -> !` | 同 |
| `error!(...)` 打印位置 | `print("[rc-aarch] PANIC!\n")` |
| `shutdown(true)` 用失败码关机 | `loop { spin_loop() }`（裸机没处关机） |

---

## 三、实测：构建产物分析

> 本节的前两张表是 **`os1/`（QEMU 版，基址 0x40080000）** 的产物；
> `os1-cell3/`（基址 0xFA000000）的产物见 "字符串在哪" 一节里的第二段。

```bash
rust-objdump -h   target/aarch64-unknown-none/release/rc-aarch-os1
rust-readobj --program-headers target/aarch64-unknown-none/release/rc-aarch-os1
```
（在 `os1/` 或 `os1-cell3/` 目录下执行，产物就在各自的 `target/` 里，已被 `.gitignore` 排除）

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

### 字符串在哪：`.rodata` 还是立即数？（两次构建不一样，别背结论）

早期一次构建里，LLVM 把短字符串**展开成一串 `mov` 立即数**（见下），于是没有 `.rodata`：

```asm
4008004c: mov w8,  #0x9000000    ← UART 地址
40080050: mov w14, #0x5b         ← '[' （字符串第一个字符！）
40080054: mov w9,  #0x72         ← 'r'
```

但**换成 cell 版、并加上 hvc + 轮询之后，构建产物里确实有 `.rodata` 了**（2026-09-23 实测）：

```
1 .text    000000bc  VMA 0xfa000000   ← 代码
2 .rodata  0000004d  VMA 0xfa001000   ← 两行字符串就放这里
3 .bss     00010000  VMA 0xfa002000   ← 64KB 启动栈
```
**教训**：优化结果依赖具体构建，**别把"某次反汇编"当规律**；要确认常量落在哪个段、地址是否在加载区内，就 `rust-objdump -h` + `strings -t x` 实测 ✔

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

## 四、阶段 1 踩到的两个坑（构建期；运行期的 4 个坑见 §5.3）

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

## 五、在 MCS cell3 上跑通（2026-09-23 实测）

### 5.1 运行方式（Jailhouse cell，不是 QEMU）

```
宿主机(Debian)                              RPi4 root cell(openEuler MCS)
────────────                                ────────────────────────────
cells/rpi4-rust-cell3.c                     /tmp/rc-aarch.cell
  └ gcc -c + objcopy → .cell  ─── scp ───→
内核 ELF / .bin               ─── scp ───→ /tmp/rc-aarch.elf (或 .bin)
                                            jailhouse cell create   ← 按资源表建 cell
                                            jailhouse cell load     ← 镜像写进 LOADABLE 区
                                            jailhouse cell start    ← CPU3 从 cpu_reset_address 起跳
串口 / cat /sys/devices/jailhouse/console  ←── 输出
```

```bash
# 在 ~/rc-aarch 下（脚本会自动 enable、destroy 旧的、create/load/start）
ROOTCELL=root@<root cell IP> ./scripts/deploy-cell3.sh \
  os1-cell3/target/aarch64-unknown-none/release/rc-aarch-os1     # 传 ELF（推荐）
```
> 传 `.bin` 也可以，脚本会自动补 `-a 0xFA000000` ✔

### 5.2 实测输出（串口）

```
Created cell "rc-aarch"
Cell "rc-aarch" can be loaded
Started cell "rc-aarch"
[[rrcc--aaaarrcchh]]  HHeelllloo,,  AAAArrcchh6644!!

[[rrcc--aaaarrcchh]]  ssttaaggee  11::  bboooott  ++  lliinnkkeerr  ++  bbssss  ++  UUAARRTT  OOKK
```
> 字符**翻倍**是设计如此：`uart_putc` 做了"双发"（`hvc` + mini-UART 直写），
> 而 hypervisor 的控制台本来就打在同一条 mini-UART 上 → 两路都落在串口 ✔。想要一份就留一路 ✔

### 5.3 ★ 阶段 1 在 MCS 上踩的 4 个坑（ch2..ch9 还会遇到）

| # | 现象 | 真因 | 处理 |
|---|---|---|---|
| 1 | 连打一串字符，**只出现第一个**，后面全丢 | **mini-UART 不等 THRE（LSR bit5）就连着写会丢字符** | 每次写前 `while read_volatile(LSR) & 0x20 == 0 {}`（LSR = UART 基址 + 0x14，间距 4 字节）|
| 2 | cell `running` 但**一个字都不打印**，也没有 fault | cell 配置**没写 `.cpu_reset_address`** → 默认 0 → `start` 时 CPU 跳到 `0x0` | 必须显式 `.cpu_reset_address = <内核链接基址>`（对照 `configs/arm64/rpi4-zephyr.c` 的 `0x7a000000`）|
| 3 | `cell load` 报 `Invalid argument`，接着 `instruction abort at 0x0` | 镜像地址不在 cell 的 **LOADABLE 区**：raw `.bin` 工具默认用 **0**，必须显式给地址；ELF 用自己段里的 `p_paddr` | 用 ELF，或 `.bin` 加 `-a 0xFA000000` |
| 4 | `cell create/load/start` 全报 `Invalid argument` | **jailhouse 没使能**（`enable` **不跨重启保留**），而 `cell list` 在未使能时**不报错、只打印空** → 不能用它判断 | 用 `cat /sys/devices/jailhouse/enabled` 判断（=1 才算使能）；每次开机都要 `jailhouse enable`（脚本已自动处理）|

### 5.4 三个地址千万别搞混（本节最值钱的一张表）

| 地址 | 由谁决定 | 作用 |
|---|---|---|
| linker.ld 基址 `0xFA000000` | 你的链接脚本 | 代码/常量按它做绝对引用 |
| cell 的 `LOADABLE` 区 `virt_start` | cell 配置 | 镜像**被拷到哪里**（`.bin` 还要配合 `-a`）|
| `.cpu_reset_address` | cell 配置 | **CPU 从哪里开始执行** |

> 三者必须对齐 ✔：**LOADABLE 区要"包含"链接基址**（本例干脆取同一个值，最不容易错），
> 而 `cpu_reset_address` 必须**正好等于**链接基址（= `_start` 的地址）。
> 对不上的表现分别是：跳 0x0 / `cell load` EINVAL / 跑飞 ✔

---

## 六、本地 QEMU 运行（可选，未做）

```bash
sudo apt install qemu-system-arm        # 提供 qemu-system-aarch64
cd ~/rc-aarch/os1                       # 注意用 os1/，不是 os1-cell3/
make run
```

> ⚠️ QEMU 跑的是 `os1/`，**不是** `os1-cell3/`：`os1-cell3/` 里的 `hvc #0x4a48` 必须由
> EL2 的 hypervisor 处理，裸 QEMU 没有 EL2 → 会直接异常。两个目录就是"同一份代码、两套落地环境"。

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

## 七、阶段 2 预告（ch2：trap + 中断 + 定时器）

| 下一步 | rCore 里对应什么 | AArch64 / Jailhouse 这边要多做什么 |
|---|---|---|
| 异常向量表 `VBAR_EL1` | ch2 的 `__alltraps` / `stvec` | 写 `vbar_el1`，向量表 16 项 × 0x80 字节对齐 |
| 时钟中断 | ch2 的 `sie`/`sip` + `mtimecmp` | **ARM generic timer**：`cntfrq_el0` / `cntv_tval_el0` / `cntv_ctl_el0`（不是 SBI 了）|
| 中断控制器 | PLIC（`0x0c00_0000`）| **GIC-400**：分发器 GICD `0xff841000`、CPU 接口 GICC `0xff842000` |
| cell 配置要加什么 | — | 定时器走 **PPI，不用改 cell 配置**；只有用到 **SPI**（如 uart0 = 125）才必须补 `.irqchips`（下面细说）|
| syscall | `ecall` | `hvc #0x4a48`（x0=调用号，x1/x2=参数），已在阶段 1 打通 |

> 阶段 1 只碰了"CPU + 内存 + UART"三样资源；阶段 2 开始要碰**中断**，
> 而中断在 Jailhouse 里是**静态划分**的 —— SPI 必须提前在 cell 配置里划给它，否则中断到不了 cell3。

**★ 这里有个反直觉的点**（读 `hypervisor/arch/arm-common/irqchip.c` 得到，省你一次试错）：

```c
/* Permit direct access to all SGIs and PPIs except for those used by
 * the hypervisor. */
cell->arch.irq_bitmap[0] = ~((1 << SGI_INJECT) | (1 << SGI_EVENT) |
                             (1 << mnt_irq));
```

- **SGI / PPI（中断号 0..31）对所有 cell 自动放行**（只扣掉 hypervisor 自己用的 3 个）
  → 时钟中断走的是 **PPI**（虚拟定时器 27 / 物理定时器 30），**不需要任何 `pin_bitmap` 配置** ✔
- **SPI（≥32）必须在 cell 配置里显式声明**，否则 `GICD_ISENABLER` 那类寄存器的写会被
  `restrict_bitmask_access()` **静默屏蔽**（不报错，中断就是不来 —— 这才是最难查的那种坑）
- 所以 ch2 想省事：**只要定时器 → `.irqchips` 可以先不加**；一旦要用 uart0 的接收中断（SPI 125）就必须加

`.irqchips` 长这样（抄自已在跑的 guest cell `rpi4-linux.c`，位图按自己需要的改）：

```c
.irqchips = {
    /* 只写 GICD（分发器），GICC 由 hypervisor 代管，不写 */
    {
        .address = 0xff841000,
        .pin_base = 32,                 /* 只描述 SPI：从 32 号开始 */
        .pin_bitmap = {
            0xffffffff, 0xffffffff,
            0xffffffff, 0xffffffff,
        },
    },
},
```
> 位图是**按 `pin_base` 连续编号**的：第 k 位（k 从 0 数，跨 u32 连续）= 中断号 `pin_base + k`。
> 例：uart0 是 125 → `k = 125 - 32 = 93` → 落在第 3 个 u32（管 k=64..95）里的第 `93-64 = 29` 位：
> `0xffffffff & (~(1u << 29))`。
> **"先写全 1，再 `& (~(1<<x))` 清掉不要的"这种写法比硬算十六进制可读得多**（guest cell 就是这么写的）。
