# rc-aarch — 用 AArch64 重写 rCore ch1/ch2

> **一句话：rCore 是"教材"，Jailhouse/MCS 是"战场"——把 ch1/ch2 学的东西用 AArch64 重写一遍，你就同时拿到了 Rust 系统编程能力和 MCS 项目的真实产出。**
>
> 创建：2026-09-18 ｜ 状态：**阶段 1 进行中**

---

## 一、为什么这条路线成立

| 事实 | 影响 |
|---|---|
| Jailhouse 0.12 上游只有 `x86` + `arm-common`（arm/arm64），**没有 RISC-V** | rCore（RISC-V）**不能**直接当 Jailhouse 的 cell |
| 但 rCore 教的是**裸机内核的通用技能** | 这些技能 100% 可迁移到 AArch64 inmate |
| Jailhouse 的 inmate 上游全是 **C**（inmate-demo / Zephyr / FreeRTOS） | **Rust inmate 是真实缺口** ← 我们的目标产出 |
| 本机 `EMCS/` 已为 **rpi4-64 构建 Jailhouse**，含 `rpi4.cell`/`rpi4-inmate-demo.cell`/`rpi4-zephyr.cell` | 战场已就绪，只差"Rust 侧"的人 |

## 二、技能映射表（rCore → AArch64）

| rCore ch1/ch2 学的 | rc-aarch 里的对应 | 阶段 |
|---|---|---|
| `entry.asm`：`la sp, boot_stack_top` + `call rust_main` | `boot.S`：`ldr x0,=__stack_top; mov sp,x0; bl rust_main` | 1 |
| `linker-qemu.ld`：`.text.entry` 排最前 | `linker.ld`：`KEEP(*(.text.entry))` | 1 |
| `.bss.stack` + `clear_bss()` | 同名机制，符号换成 `__bss_start/__bss_end` | 1 |
| `console.rs` → **SBI `ecall`** 打印 | **PL011 UART MMIO** 直写（`0x0900_0000`） | 1 |
| `lang_items.rs`：`#[panic_handler]` | 同 | 1 |
| ch2 `trap` + `syscall(ecall)` | ARM64 **`hvc #0` hypercall** | 2 |
| ch3 多任务 / ch4 页表 | inmate 的实时任务 / MMU 配置 + ivshmem BAR 映射 | 2–3 |

## 三、路线图

```
阶段1（1-2周） Rust 裸机 ARM64 hello
              验收：QEMU aarch64 上打印 "Hello, AArch64!"，跑通 链接脚本+entry+bss+UART
              ← 现在在这里
阶段2（2-3周） 升级成 Jailhouse inmate
              验收：hvc hypercall 输出心跳；先在 x86 QEMU 验 vmcall 更容易
阶段3（1月）   接 ivshmem
              验收：inmate ↔ root cell 收发数据，配套 EMCS 的 ivshmem-net 驱动
阶段4（按需）  测量/对比 → 论文数据；或转向 RISC-V H 扩展选题（Jailhouse 无 RISC-V = 空白）
```

## 四、目录结构

```
rc-aarch/
├── README.md                 ← 本文件（项目章程）
├── docs/
│   └── 01-阶段1-对照rCore.md  ← 逐行对照讲解（entry / linker / UART / bss）
└── os1/                      ← 阶段 1 的 cargo 工程
    ├── Cargo.toml
    ├── .cargo/config.toml    ← target + 链接脚本 + 本地 target-dir
    ├── linker.ld
    ├── Makefile
    └── src/
        ├── boot.S            ← 启动汇编（对照 entry.asm）
        └── main.rs           ← rust_main + UART + clear_bss + panic
```

## 五、环境依赖

| 依赖 | 状态 | 安装命令 |
|---|---|---|
| `aarch64-unknown-none` target | 待装 | `rustup target add aarch64-unknown-none` |
| `qemu-system-aarch64` | ❌ 缺 | `sudo apt install qemu-system-arm` |
| `rust-objcopy` / `rust-objdump` | ✅ 已有 | — |

**注意**：Vault 是 CIFS 网络盘，所以 `.cargo/config.toml` 里把 `target-dir` 指到了
`/home/era/.cache/rc-aarch-target`（本地盘）——否则编译会很慢。

## 六、构建与运行

```bash
cd ~/Obsidian\ Vault/Rust/rc-aarch/os1
make build     # 编译
make run       # 编译 + QEMU 运行（需要 qemu-system-aarch64）
make disasm    # 反汇编看看入口
```
