# 📘 阶段 2：陷入/异常 + 系统调用 + 批处理 —— 对照 rCore ch2 逐项讲

> 项目：rc-aarch ｜ 工程：`os1-cell3/`（MCS cell 版）+ 仓库根 `user/`（aarch64 用户程序）
> 状态：**✅ ch2-A / ch2-B / ch2-C 全部已在树莓派 4B 的 openEuler MCS + Jailhouse cell3 上跑通**
> 更新：2026-10-12 ｜ 分支：`ch2`（存档点 `ch1-done`）
> 相关：[[01-阶段1-对照rCore]] ｜ [[RPi4 MCS 内存布局与三个 Cell 的分工（750M 之谜）]]

---

## 一、验收标准（全部实测通过）

| 检查项 | 状态 | 实测证据 |
|---|---|---|
| 16 项向量表 + 2KB 对齐（`.vectors` 段） | ✅ | `__vector_table = 0xfa002800 & 0x7ff == 0`；16 槽反汇编逐项核对 |
| `VBAR_EL1` 生效、`svc` 落向量表 #4 | ✅ | `EC=0x15 ESR=0x56000000`（EL1）；EL0 落 #8 |
| SAVE/RESTORE 现场完整性 | ✅ | 26 个寄存器跨 `svc` 一字不差（`self test PASSED`）|
| `svc` → `syscall` 分发（`x8` 调用号 / `x0` 返回值）| ✅ | `id=64 返回 43`（= `sys_write` 的 len）|
| `sys_write` 输出 | ✅ | 内核态与 EL0 两种来源都打印成功 |
| 未知调用号 → `-1` | ✅ | `id=57005 返回 -1` |
| **app 跑在 EL0** | ✅ | `SPSR=0x600003c0`（M=`0b0000` = EL0t），`from EL0` |
| `batch` 加载 app 到 0xFA100000 并 `eret` 下去 | ✅ | `load app_0 (8718 字节) → 0xfa100000` |
| app 退出 → 内核切下一个 / 全部完成 | ✅ | `Application exited with code 0` → `All apps completed!` |
| app 崩溃 → 内核兜住并换下一个 | ✅（代码就绪）| `fault_from()`：EL0 异常 → `run_next_app()`；EL1 异常 → 停机 |

---

## 二、rCore ch2 到底做了什么

```
$ git diff --stat origin/ch1 origin/ch2 -- os/
 os/build.rs            |  56 ++   ← 编译 user app、生成 link_app.S（内嵌 app 二进制）
 os/src/batch.rs        | 155 ++   ← 批处理：AppManager / 加载 app / run_next_app
 os/src/sync/{mod,up}.rs|  36 ++   ← UPSafeCell（单核内部可变性）
 os/src/syscall/*.rs    |  55 ++   ← sys_write / sys_exit 分发
 os/src/trap/{trap.S,mod.rs,context.rs} ← __alltraps/__restore、TrapContext、trap_handler
 os/src/main.rs         |  38 +-   ← trap::init() → batch::init() → run_next_app()
 user/                  |  11 个新文件（5 个测试 app）
```

⚠️ **注意：ch2 没有定时器/中断** —— `timer.rs`、`set_next_trigger`、`enable_timer_interrupt`
全在 **ch3**（本工程严格按上游分章，所以 ch2 期间 **cell 配置一个字都没改**）。

三件事一句话：**A 是"门"（异常入口+现场），B 是"门里的人"（系统调用），C 是"敲门的人"（用户程序）**。

---

## 三、逐项对照：rCore (RISC-V) ↔ rc-aarch (AArch64)

| rCore ch2 | rc-aarch | 说明 |
|---|---|---|
| `stvec` + Direct 模式，单一入口 | **`VBAR_EL1`**，16 项 × 0x80（2KB 对齐）| RISC-V 靠 `scause` 分类；AArch64 **先按落在哪个向量项分类**，再用 `ESR_EL1.EC` 细分 |
| `__alltraps`：`csrrw sp,sscratch,sp` 换栈 | **不用换栈** | 从 EL0 进 EL1 时硬件自动切到 `SP_EL1` |
| 手工 `sd` 存 34 个字 | `stp/ldp` 一次 16 字节 | 省一半指令 |
| `TrapContext { x[32], sstatus, sepc }` | `TrapContext { x[31], sp_el0, elr_el1, spsr_el1 }` | 都是 **34 × 8 = 272 字节** |
| `scause` / `stval` | `ESR_EL1`（EC + ISS）/ `FAR_EL1` | EC：`0x15`=SVC64、`0x18`=SYS64、`0x20/0x21`=IABT、`0x24/0x25`=DABT、`0x00`=UNKNOWN |
| `ecall`（U→S）| **`svc #0`** | 落向量表 **#8（Lower EL AArch64 Sync）** |
| `ecall`（S→M，SBI）| **`hvc #0x4a48`**（EL1→EL2）| 落 hypervisor 的 EL2 表；`DEBUG_CONSOLE_PUTC = 8` |
| 调用号 `x[17]`(a7)、参数 `x[10..12]`、返回 `x[10]` | **`x8`=调用号、`x0/x1/x2`=参数、`x0`=返回** | AArch64/Linux ABI 习惯 |
| `sret` | `eret` | |
| app 在 U-mode（无分页）| app 在 **EL0**（无分页）| ✅ 已实测可行（见第六节）|
| app 基址 `0x80400000` | app 基址 **`0xFA100000`** | cell3 的 `[1] RAM 区`，见 `user/src/linker.ld` |
| `01store_fault` 测越界写 | ⚠️ **跳过**（原因见坑 3）| 在 Jailhouse 里越界是 hypervisor 级事件 |

---

## 四、实测输出（完整串口，2026-10-12）

```
[rc-aarch] stage 2: trap framework on Jailhouse cell3
[trap] self test: 26 个寄存器 svc 前后完整性检查 ...
[trap] EC=0x15 ESR=0x0000000056000000 ELR=0xfa000168 FAR=0x0 SPSR=0x600003c5 (from EL1)
[trap]   -> svc from EL1: id=4104 返回 ...
[trap] self test PASSED (0 个寄存器被写坏)
[rc-aarch] ch2-A done: svc -> vector#4 -> trap_handler -> eret 全部正常
[syscall] sys_write: hello from a syscall!                      ← ch2-B
[trap]   -> svc from EL1: id=64 返回 43 (写回 x0)
[syscall] 不支持的调用号: 57005
[trap]   -> svc from EL1: id=57005 返回 -1 (写回 x0)
[kernel] num_app = 1                                            ← ch2-C
[kernel] app_0 [0xfa005018, 0xfa007226)
[kernel] load app_0 (8718 字节) → 0xfa100000
[trap] EC=0x15 ESR=0x56000000 ELR=0xfa100158 FAR=0x0 SPSR=0x600003c0 (from EL0)   ← ★ app 在 EL0 发 svc
Hello, world from EL0 (user mode)!                              ← ★ app 的输出
[trap]   -> svc from EL0: id=64 返回 35 (写回 x0)                ← 35 = 消息长度
[trap] EC=0x15 ESR=0x56000000 ELR=0xfa10004c FAR=0x0 SPSR=0x600003c0 (from EL0)   ← app 的 sys_exit
[kernel] Application exited with code 0
[kernel] All apps completed!
[trap] shutdown：进入死循环（ch3 之后可以改成报状态给 hypervisor）
```

关键值解读：
- `ESR=0x56000000` = `EC(0x15)<<26 | IL(bit25)` —— 看到 **`0x5a…` 就是 `hvc`**（EC=0x16），可以据此区分两条陷入路径。
- `SPSR` 的 `M[3:0]`：`0b0101`=EL1h（内核自己）、`0b0000`=EL0t（app）。
- `ELR` 是"返回地址"：SVC 时 **已经指向下一条指令**（见坑 1）。

---

## 五、★ ch2 踩到的三个坑（都很值钱）

### 坑 1（移植级）：AArch64 的 `ELR_EL1` 已经指向 `svc` 的下一条指令

| | RISC-V `sepc` | AArch64 `ELR_EL1` |
|---|---|---|
| 指向 | `ecall` **本身** | `svc` 的**下一条指令** |
| 处理 | 必须 `cx.sepc += 4` | **什么都不用改** |

照抄 rCore 的 `+= 4` → 返回时**跳过一条指令**。本次后果：EL0 探针里跳过了 stub 的 `b .`，一路跑进后面的代码，
最后在 EL0 执行到 `hvc`（`hvc` 在 EL0 是**未定义指令**）→ `EC=0x00` 每秒几千次刷屏。
**现象极具误导性**（看起来像"EL0 跑不起来"），真因只是多加了 4。

> 教训：移植时**逐个寄存器/字段核对语义**，别照抄"看起来等价"的代码。

### 坑 2：不可变 `static` 会被放进 `.rodata`（栈必须用 `static mut`）

ch2-C 的 `KERNEL_STACK` / `USER_STACK` 一开始写成不可变 `static`，结果 `rust-nm` 显示：

```
0xfa005000 r USER_STACK      ← 'r' = .rodata（只读段）
0xfa007000 r KERNEL_STACK
```

栈是要被写的！现在没炸**只是因为**本 cell 的 LOADABLE 区恰好是 RW；
一旦把 `.rodata` 设成只读（正常加固），写栈就会变成 stage-2 权限异常 → hypervisor panic（整板挂死）。
修法：`static mut` + 全零初始化 → 落到 `.bss`（由 `clear_bss` 清零）。修完 `.rodata` 从 `0x5400` 缩到 `0x810`。

> 顺手记：`rust-nm`/`rust-objdump -h` 是**唯一**能发现这类"段放错"的手段 —— 编译和运行都不会报错。

### 坑 3（架构级）：Jailhouse 的"隔离边界"在 hypervisor 手里，越界访问会打死整台机器

**问题**：Jailhouse 不是静态分区吗？一个 cell 越界访问别的区域，难道不是"这个 cell 自己崩"吗？

**答案（读源码 + 实测确认）**：**不是。** arm64 上"stage-2 fault"**必须由 EL2 处理**，所以越界访问
根本不会变成 cell 自己的异常，而是变成 hypervisor 的一次 trap：

```c
/* hypervisor/arch/arm64/traps.c */
static enum trap_return handle_iabt(struct trap_context *ctx)   /* 取指越界 */
{
        ...
        panic_printk("FATAL: instruction abort at 0x%lx\n", (hpfar << 8) | (hdfar & 0xfff));
        return TRAP_FORBIDDEN;          /* → arch_handle_trap: FATAL + dump_regs + panic_park() */
}

static const trap_handler trap_handlers[0x40] = {   /* ★ 表里没有 ESR_EC_SVC64！ */
        [ESR_EC_HVC64] = handle_hvc, [ESR_EC_SMC64] = handle_smc,
        [ESR_EC_SYS64] = handle_sysreg, [ESR_EC_IABT_LOW] = handle_iabt,
        [ESR_EC_DABT_LOW] = arch_handle_dabt,
};
```
```c
/* hypervisor/arch/arm64/mmio.c */
static void arch_inject_dabt(struct trap_context *ctx, unsigned long addr)
{
        int err __attribute__((unused)) = trace_error(-EINVAL);
        while (1);                      /* ★ 这个"把 DABT 注入回 cell"的函数根本没实现 */
}
...
error_unhandled:
        panic_printk("Unhandled data %s at 0x%lx(%d)\n", ...);
        return TRAP_UNHANDLED;          /* → FATAL: unhandled trap → panic_park() */
```

**结论对照表**：

| app 干了什么 | rCore（RISC-V + SBI）| 我们（Jailhouse cell3）|
|---|---|---|
| EL0 执行非法/特权指令 | U-mode 异常 → 内核兜住 | **落我们自己的 EL1 向量**（`EC=0x00/0x18`）→ 能兜住、能 kill app ✅ |
| 访问 cell 外的地址（越界读写/取指）| StoreFault → 内核兜住 | **stage-2 fault 进 EL2 → jailhouse `FATAL: unhandled trap` + `panic_park()`** → **整板挂死（要断电）** ⚠️ |

**设计取舍**：这是"静态分区 + 可验证性优先"的必然结果 —— hypervisor 假设**配置是对的**，
越界 = 配置/驱动 bug（而不是可恢复的运行时错误），所以选择 fail-fast（不写恢复路径，代码更小更可审计）。
代价：**cell 之间的"故障隔离"不等于"任意越界的故障隔离"**；越界是系统级事故。

**我们怎么和它共处**：
1. **`01store_fault` 这个上游 app 不照搬**（它写非法地址 → 会打死 hypervisor，而不是"内核 kill app"）。
   替代：用 **EL0 非法指令**（`03priv_inst`/`04priv_csr`）来演示"内核兜住并切下一个 app"。
2. **syscall 边界必须做指针检查**：`sys_write` 里加了
   `0xFA000000 ≤ buf && buf+len ≤ 0xFC000000`（cell 自己的内存窗口），
   否则 app 传一个坏指针给内核，内核一读 = 板子挂死。
3. **cell 的区域就是 app 的"沙箱"**：`[1] RAM` 那 31MB 内随便乱写都只是打到自己的空地，
   不会惊动 hypervisor（这也解释了为什么 cell 配置里给得比较宽松）。

---

## 六、EL0 到底能不能跑？（探针实验）

**背景担忧**：AArch64 在 stage-1 MMU 关闭时内存属性会变成 Device，而架构**不允许 EL0 访问/取指 Device 内存**；
rCore 那种"U-mode + 不分页"在 AArch64 上可能成立不了。

**探针做法**（`trap.S` 的 `enter_el0` + `el0_probe_stub`，`trap::probe_el0()`）：
构造 `SPSR_EL1.M = 0b0000`（EL0t）+ `ELR_EL1` 指向一小段只做 `svc` 的代码 + `SP_EL0` 指向探针栈 → `eret`。

**实测结果（✅ 可用）**：

```
[probe] 准备 eret 到 EL0：入口=0xfa00033c 栈顶=0xfa014000
[trap] EC=0x15 ESR=0x56000042 ELR=0xfa000340 FAR=0x0 SPSR=0x3c0 (from EL0)   ← 第 1 次 svc
[trap] EC=0x15 ESR=0x56000043 ELR=0xfa000344 FAR=0x0 SPSR=0x3c0 (from EL0)   ← 回到 EL0 后第 2 次 ✔
```

**为什么能跑**：jailhouse 的 **stage-2 映射**给 cell 的 RAM 是 Normal 属性（RWX），
它在 stage-1 关闭时仍然生效，所以 EL0 能正常取指/读写。

**好处**：ch2-C 的 app 可以**忠实跑 EL0**，与 rCore 的 U-mode 语义完全对齐 —— 上游那 5 个
测试 app 里的"特权指令/越权访问"类（除越界访存外）都能照搬。

> `probe_el0` 留作工具（`#[allow(dead_code)]`），CL0 探针代码见 `src/trap/trap.S` 第 5 节。

---

## 七、文件清单（ch2 新增/改动）

| 文件 | 作用 | 对照 upstream |
|---|---|---|
| `os1-cell3/linker.ld` | 新增 `.vectors` 独立段（`ALIGN(2048)`，且**不能排在 `_start` 前面**）| `linker-qemu.ld` |
| `os1-cell3/src/console.rs` | 升级为 `core::fmt::Write` + `print!/println!` | `os/src/console.rs` |
| `os1-cell3/src/trap/trap.S` | 16 项向量表 + `SAVE/RESTORE_REGS` + `__restore` + EL0 探针 | `os/src/trap/trap.S` |
| `os1-cell3/src/trap/context.rs` | `TrapContext`（34×8 字节 ABI）| `os/src/trap/context.rs` |
| `os1-cell3/src/trap/mod.rs` | `VBAR_EL1` 初始化 + 按 `ESR_EL1.EC` 分发 + `fault_from` + `shutdown` | `os/src/trap/mod.rs` |
| `os1-cell3/src/syscall/{mod,fs,process}.rs` | `syscall` 分发 / `sys_write`（带指针检查）/ `sys_exit` | `os/src/syscall/*` |
| `os1-cell3/src/sync.rs` | `UPSafeCell`（不依赖 `lazy_static`）| `os/src/sync/up.rs` |
| `os1-cell3/src/batch.rs` | `AppManager` / `KernelStack` / `UserStack` / `load_app` / `run_next_app` | `os/src/batch.rs` |
| `os1-cell3/build.rs` | 编 user crate + `rust-objcopy` + 生成 `src/link_app.S` | `os/build.rs` |
| `user/`（仓库根，**不在 os1-cell3 里**）| aarch64 用户程序 crate（`_start` 清 bss → `main` → `sys_exit`）| `user/`（同样在仓库根）|
| `user/src/bin/00hello_world.rs` | 第一个 app | 同 |

**两个结构性注意点**：
1. `user/` 必须在**仓库根**：cargo 会向上合并父目录的 `.cargo/config.toml`，
   放在 `os1-cell3/` 里会同时吃到内核的 `-Tlinker.ld` 和自己的 `-Tsrc/linker.ld` → 链接报 "cannot find linker script"。
   （upstream 的 `user/` 本来就在仓库根，是 `os/` 的兄弟目录。）
2. `src/link_app.S` 是 `build.rs` 生成的（已加进 `.gitignore`）。

---

## 八、阶段 3 预告（ch3：中断 + 定时器 + 抢占）

| 要做的 | rCore ch3 | AArch64 / Jailhouse 这边 |
|---|---|---|
| 定时器 | `mtimecmp` + SBI `set_timer` | **ARM generic timer**：`CNTFRQ_EL0` / **`CNTV_TVAL_EL0` + `CNTV_CTL_EL0`**（upstream inmate 库就是这么用的），中断号 **PPI 27** |
| 中断入口 | `sie`/`sip` + `scause` 最高位 | 向量表 **#5/#9（IRQ）** —— ch2 已留好槽位（现在指向 `unhandled`）|
| 中断控制器 | PLIC | **GIC-400**：使能 `GICD_ISENABLER`（PPI 自动放行，**不用改 cell 配置**）→ 读 **`GICC_IAR`** → 写 **`GICC_EOIR`**；`GICC_CTLR=1`、`GICC_PMR=0xF0`（照抄 `inmates/lib/arm-common/gic-v2.c`）|
| 为什么中断能进 EL1 | — | hypervisor 在 EL2 收下 IRQ，用 **GICv2 list register（`GICH_LR`）注入成虚拟中断**（`gic-v2.c:289 gicv2_inject_irq`），所以 cell 里照样走自己的 `VBAR_EL1` |
| 抢占/切换 | `suspend`/`resume`、任务切换 | 用 ch2 已有的 `TrapContext` + `__restore` 就能做"时间片轮转"；批处理升级成多道程序 |
| `sys_yield` | ch3 新增 | 同理 |

> 好消息：**ch3 大概率也不用改 cell 配置**（定时器是 PPI，`irqchip.c` 里对 PPI/SGI 自动放行，
> 只排除 hypervisor 自己用的 3 个）。
