# 📘 阶段 4：地址空间 + 分页 —— 对照 rCore ch4 逐项讲

> 项目：rc-aarch ｜ 工程：`os-cell3/`（MCS cell 版）+ 仓库根 `user/`
> 状态：**✅ ch4 已在树莓派 4B 的 openEuler MCS + Jailhouse cell3 上跑通（4 个任务、各自一套页表、sbrk 扩页）**
> 更新：2026-10-21 ｜ 分支：`ch4`（从 ch3 的 `6862ea4` 分出）
> 相关：[[03-阶段3-对照rCore]] ｜ [[02-阶段2-对照rCore]] ｜ [[01-阶段1-对照rCore]]

---

## 一、验收标准（全部实测通过）

| 检查项 | 状态 | 实测证据 |
|---|---|---|
| 建内核地址空间 + 打开 stage-1 MMU | ✅ | `[mm] stage-1 MMU 已打开（恒等映射，TTBR0）` |
| 段权限（.text RX / .data RW XN） | ✅ | `[mm] remap_test passed: .text RX / .data RW&XN` |
| 物理页帧分配器 | ✅ | `frame allocator: [0xfa092000, 0xfc000000) 共 7985 页（31 MB）` |
| 内核堆（alloc） | ✅ | `kernel heap: 256 KB（bump，不回收）`，Vec/BTreeMap 可用 |
| 每个任务一套页表 | ✅ | `task manager: 4 个任务就绪（每任务一套页表）` |
| app 统一加载到 0x10000（ELF 解析） | ✅ | 崩溃日志里 ELR=`0x10290`、`0x1008c`…（用户态 VA） |
| `sys_write` 翻译用户指针 | ✅ | 4 个 app 都能正常打印 |
| `sys_sbrk` 扩页 / 缩页失败 | ✅ | `初始 brk = 0x17000` → `sbrk(4096) -> 0x17000` → `sbrk(8192) -> 0x18000`；`sbrk(-0x100000) = -1` |
| 时间片/抢占仍然工作 | ✅ | `[app1] A1..A4` 与 `[app2] B1..B4` 交错 |
| 板子不挂、可反复 RELOAD | ✅ | `cell list` 全程 `running`、`Failed CPUs` 为空 |

---

## 二、rCore ch4 做了什么

```
$ git diff --stat origin/ch3 origin/ch4 -- os/ user/
 os/src/mm/address.rs        | 263 +++   地址/页号类型（SV39）
 os/src/mm/frame_allocator.rs| 136 +++   物理页帧分配器
 os/src/mm/heap_allocator.rs |  46 +++   buddy 内核堆
 os/src/mm/page_table.rs     | 157 +++   页表项/页表/用户指针翻译
 os/src/mm/memory_set.rs     | 425 +++   MapArea / MemorySet / 内核空间 / from_elf
 os/src/mm/mod.rs            |  28 +++   初始化入口
 os/src/task/{task,mod,context}.rs | +180  每任务 memory_set、切 satp、sbrk
 os/src/trap/{trap.S,mod,context}.rs | +140 trampoline、切页表、TRAP_CONTEXT
 os/src/syscall/fs.rs        |  10 +-   sys_write 翻译用户指针
 os/src/{main,config}.rs     |  28 +-   mm::init()、TRAMPOLINE/TRAP_CONTEXT
 user/build.py 删除、所有 app 统一链接到 0x10000、改用 ELF
```

一句话：**ch3 的"多任务"共用一套物理地址；ch4 给每个任务一张页表，把"地址"和"物理内存"解耦。**

---

## 三、★★ 结构差异：AArch64 为什么不需要 trampoline

这是本章移植里最值得记的一点。

| | RISC-V（upstream） | AArch64（rc-aarch） |
|---|---|---|
| 页表基址寄存器 | 只有 `satp` 一个 | **`TTBR0_EL1`（低半区）+ `TTBR1_EL1`（高半区）** |
| 用户/内核地址空间 | 共用 `satp`，靠**切 satp** 区分 | 我们用**一个 TTBR0**，内核映射挂在每个任务页表里 |
| 陷阱入口 | 进来时还在用户页表上 → 必须跳到用户空间最高页的 **trampoline**，先切 satp 再跳内核 | 陷阱进来时内核映射**已经可访问**（共享 L1[3] 子树），直接跑内核代码 |
| TrapContext 放哪 | 用户地址空间的 `TRAP_CONTEXT` 页（trampoline 用固定 VA 找它） | **内核栈顶**（恒等映射，随时可访问） |
| 内核栈 | 必须在"内核空间"固定 VA | `.bss` 里的静态数组即可 |

我们的页表布局（一张任务页表）：

```
VA 0x0000_0000 ~ 0xBFFF_FFFF   ← L1[0]：用户空间（app 0x10000 起、栈、sbrk 堆）
                                  每个任务**私有**，AP=01（EL0 可读写）
VA 0xC000_0000 ~ 0xFFFF_FFFF   ← L1[3]：内核镜像 + 页帧池 + comm + UART + GIC
                                  所有任务**共享同一棵子树**（内核建一次，任务页表直接挂）
```

好处：

- 陷阱进来不用切页表（省掉 trampoline、`TRAP_CONTEXT` 页、`kernel_satp`/`kernel_sp` 字段）；
- 内核栈可以直接放 `.bss`，不用在内核空间里"插一段"；
- `sp` 在 EL0/EL1 之间由硬件自动切（SP_EL0/SP_EL1），和 ch3 一致。

代价：用户 VA 只能落在 `0x0 ~ 0xC000_0000`，并且内核页必须 `UXN=1`（见坑 1）。

---

## 四、逐项对照

### 4.1 地址类型（`mm/address.rs`）

| rCore（SV39） | rc-aarch（VMSAv8-64, 4KB granule） |
|---|---|
| VA 39 位 / PA 56 位 / 3 级页表 | **VA 48 位 / PA 40 位 / 4 级页表** |
| `indexes() -> [usize; 3]` | `indexes() -> [usize; 4]`（L0~L3） |
| `VirtAddr` 高位符号扩展 | 我们不用高半区，直接按 48 位截断 |
| `get_pte_array()/get_bytes_array()/get_mut()` | 同（物理页恒等映射，PA 直接当指针用） |

### 4.2 页表项（`mm/page_table.rs`）

| 概念 | RISC-V PTE | AArch64 stage-1 描述符 |
|---|---|---|
| 有效 | `V` bit0 | `[0]` VALID（叶子/表项都是 `0b11`） |
| 物理页号 | `[53:10]` | `[47:12]` |
| 权限 | `R W X U` bit1..4 | `AP[2:1]`（bit7/bit6）+ `UXN`(54) / `PXN`(53) |
| 已访问/脏 | `A D` bit6..7 | `AF`(bit10) 必须置 1；脏位硬件管 |
| 内存属性 | 无（靠 PMA） | `AttrIndx[2:0]` 选 **MAIR_EL1** 里的属性（0=Normal WB，1=Device） |
| 页表基址 token | `satp = 8<<60 \| ppn` | `TTBR0 = root_pa \| ASID<<48` |

### 4.3 MMU 三件套（AArch64 特有，upstream 没有）

| 寄存器 | 我们写的值 | 作用 |
|---|---|---|
| `MAIR_EL1` | `0x00FF` | attr0=Normal Write-Back，attr1=Device-nGnRnE |
| `TCR_EL1` | T0SZ=16, TG0=4KB, IRGN0/ORGN0=WBWA, SH0=inner, T1SZ=16, EPD1=1, IPS=40bit | 48 位 VA、4KB 粒度、TTBR1 不走表 |
| `SCTLR_EL1` | `0x30D01805` | RES1 位 + `M`(MMU) + `C`(D-cache) + `I`(I-cache) |

打开顺序（`mm/mod.rs::enable_mmu`）：写 MAIR/TCR/TTBR0 → `dsb ish` → `ic iallu` → `tlbi vmalle1` → `dsb ish` → `isb` → 写 SCTLR（这条 `msr` 前后 PC 连续，因为恒等映射，**不需要跳板**）→ `isb`。

### 4.4 其它

| rCore ch4 | rc-aarch | 说明 |
|---|---|---|
| `init_heap()` + buddy | 静态 256KB + **bump 分配器** | 不引 `buddy_system_allocator`，`alloc` 是 sysroot 自带 |
| `init_frame_allocator()` 用 `ekernel..MEMORY_END` | 同（`__kernel_end..0xFC000000`） | 8030/7985 页可用 |
| `MemorySet::new_kernel()` 映射各段 + MMIO | 同（多了 `DEVICE` 权限） | 恒等映射 |
| `MemorySet::from_elf()` + trampoline + TRAP_CONTEXT | 同去掉了 trampoline/TRAP_CONTEXT | 用户栈仍由内核映射 |
| `TaskControlBlock::new` 用 `trap_cx_ppn` | TrapContext 在内核栈顶 | 见第三节 |
| `trap_return()` 跳 trampoline 里的 `__restore` | `__restore_sp`（sp 就是 TrapContext） | ch3 的机制原样保留 |
| `sys_write` → `translated_byte_buffer` | 同 | 还要检查 PTE 的 `AP[1]`（用户页） |
| `sys_sbrk(214)` + `change_program_brk` | 同 | 扩/缩页 + `tlbi` |

---

## 五、实测输出（2026-10-21，hvc 虚拟控制台）

```
[mm] kernel heap: [0xfa06d000, 0xfa0ad000) 256 KB（bump，不回收）
[mm] frame allocator: [0xfa092000, 0xfc000000) 共 7985 页（31 MB）
[mm] stext   = 0xfa000000
[mm] srodata = 0xfa00a000
[mm] sdata   = 0xfa00c000
[mm] sbss    = 0xfa020000
[mm] ekernel = 0xfa092000
[mm] stage-1 MMU 已打开（恒等映射，TTBR0）
[mm] remap_test passed: .text RX / .data RW&XN
[rc-aarch] stage 4: 地址空间 + 分页（每任务一套页表）on Jailhouse cell3
[kernel] num_app = 4
[kernel] 建任务 0（ELF 17832 字节）
[kernel] 建任务 1（ELF 17928 字节）
[kernel] 建任务 2（ELF 17928 字节）
[kernel] 建任务 3（ELF 25024 字节）
[kernel] task manager: 4 个任务就绪（每任务一套页表）
...
[irq] 第 1 次 tick（来自 EL0）
[app1] A
Hello, world from EL0 (user mode!)
[app3] sbrk_test: 初始 brk = 0x17000
[app3] sbrk(4096) -> 0x17000
[app3] 新页可读可写 ✓
[app2] B0
[app3] sbrk(8192) -> 0x18000
[app3] 跨页读写 OK ✓
[app3] sbrk(-0x100000) = -1（-1 表示失败，符合预期）
[app1] A1 / [app2] B1 / [app1] A2 / [app1] A3 / [app2] B2 ...
[kernel] All applications completed!
```

看点：

- app 的 ELR 全是 `0x10xxx`（用户 VA 0x10000 起），不再有 `0xfa1xxxxx`；
- `sbrk` 扩出来的页能读能写（背后是新分配的物理页帧），缩太多返回 -1；
- 4 个任务各有页表，抢占/yield 照旧。

---

## 六、★ ch4 踩到的坑

### 坑 1：EL0 的"可执行"只看 `UXN`，不看 `AP`

AArch64 stage-1 里 `AP[1:0]` 管的是**数据访问**权限，**指令取指在 EL0 由 `UXN` 单独控制**。
所以一个"EL0 不能读写"的页（`AP=00`），只要 `UXN=0`，EL0 照样可以跳上去执行。
→ 内核页必须显式置 `UXN=1`（我们的 `MapPermission::to_pte_flags()` 对所有非 U 页都加了）。

另外 `remap_test` 一开始误用了"`UXN==0`"来判断 `.text 可执行`，而内核页 `UXN=1`、`PXN=0`，
于是自测直接报"`.text` 不可执行"。**内核看 PXN，用户看 UXN**，两个位别混。

### 坑 2（最坑）：`DC CVAU` 只清**一条 cache line**，不是一整页

写完 app 代码要 `dc cvau`（clean by VA to PoU）+ `ic iallu`（invalidate I-cache）。
我们最初对**页首地址**只调了一次 `dc cvau` —— 4KB 页里只有前 64 字节被写回内存，
剩下的还在 D-cache 里。ELF 加载用的是**恒等映射的普通写**，于是：

```
现象：app 跑到某个函数的第一条指令就 EC=0x00（Unknown），
      而我们用页表把那条指令读出来一看：完全合法（cbz/sub ...）。
原因：CPU 取指走 PoU（L2/内存），拿到的是"还没写回的旧内容/零"，解码成 udf。
```

修法：**按 64 字节步进**，把整页逐行清（`mm::clean_dcache_range_to_pou`）。

> 这个 bug 很有欺骗性：反汇编看指令是对的、崩溃地址每次还不一样（取决于哪些脏行被
> 自然淘汰写回），一开始会怀疑页表/ELF 解析/栈。最后靠"把 ELR 处的指令字读出来打印"
> 才定位到"内存里对、CPU 看到的错"。

### 坑 3：同一个 crate 里不能重复声明同名 extern 符号

`main.rs` 已经有 `static __bss_start`，`mm/memory_set.rs` 又写了 `safe fn __bss_start()`。
Rust 允许两个同名 extern 声明（类型不同），但 LLVM 会把其中一个改名成 `__bss_start.1`，
链接时就成了 "undefined symbol: `__bss_start.1`"。
→ 在链接脚本里多导出一个 `__sbss` 给 mm 用（upstream 的 `sbss_with_stack` 也是这个道理）。

### 坑 4：用户指针不能直接解引用了

ch3 内核和用户共用物理地址，`sys_write` 直接 `from_raw_parts(buf, len)`。
ch4 每个任务页表不同，用户的 `0x10000` 在内核当前页表下查不到 → 必须
`translated_byte_buffer(当前任务 token, buf, len)` 先翻译成物理页再读，并且要检查
PTE 的 `AP[1]`（否则用户传一个内核地址就能读内核）。

### 坑 5：内核页表必须挂进每个任务页表

任务页表是新建的（root 页是空的）。用户段都映射在 L1[0]，而内核/MMIO 在 L1[3]。
如果不把内核的 L1[3] 表项复制过去，`eret` 到 EL0 之后**第一次陷阱进门就缺页**
（内核向量表都取不到）。`PageTable::share_l1_entry()` 干的就是这件事。

### 坑 6：ELF 默认 64KB 段对齐，每个 app 白白胖 64KB

`aarch64-unknown-none` 的 lld 默认 `-z max-page-size=65536`，而 app 链接在 0x10000，
于是 `p_offset == p_vaddr == 0x10000`，文件里垫了 64KB 的洞，一个 app ELF 79KB。
4 个 app 就是 300 多 KB 的内核 `.data`（内嵌 ELF）。
→ `user/.cargo/config.toml` 加 `-Clink-arg=-zmax-page-size=4096`，ELF 降到 17~25KB。

### 坑 7（沿用）：改了 `-T` 链接脚本，cargo 不会自动重链

cargo 只跟踪源文件，不跟踪 `-T` 指定的脚本。`build.rs` 里比较 `linker.ld` 和 ELF 的
mtime，必要时 `touch` 一下 app 源文件强制重建（否则会拿旧的 0xFA100000 版本去跑）。

---

## 七、文件清单（ch4 新增/改动）

| 文件 | 作用 |
|---|---|
| `os-cell3/src/mm/mod.rs` | **新** init（堆→页帧→内核空间→开 MMU）、MAIR/TCR/SCTLR、cache 维护 |
| `os-cell3/src/mm/address.rs` | **新** VA/PA/页号（48 位 VA、4 级索引）|
| `os-cell3/src/mm/page_table.rs` | **新** AArch64 描述符、页表、`translated_byte_buffer`、共享内核子树 |
| `os-cell3/src/mm/frame_allocator.rs` | **新** 页帧分配器（栈式 + 回收）|
| `os-cell3/src/mm/heap_allocator.rs` | **新** 内核堆（bump + `#[global_allocator]`）|
| `os-cell3/src/mm/memory_set.rs` | **新** MapArea/MemorySet/`new_kernel`/`from_elf`/`remap_test` |
| `os-cell3/src/loader.rs` | 改为 `get_app_data` + 内核栈 + 手工 ELF64 解析 |
| `os-cell3/src/task/task.rs` | TCB 加 `memory_set`/`asid`/`heap_bottom`/`program_brk` |
| `os-cell3/src/task/mod.rs` | `Vec<TCB>`、切 TTBR0、`current_user_token`、sbrk 入口 |
| `os-cell3/src/syscall/{fs,process,mod}.rs` | 用户指针翻译、`sys_sbrk(214)` |
| `os-cell3/src/{main,config}.rs` | `mm::init()` 流程、PAGE_SIZE/MEMORY_END |
| `os-cell3/src/trap/mod.rs` | UNKNOWN 分支打印 task/ELR/指令字（调试用）|
| `os-cell3/linker.ld` | 导出 `__text_start/__rodata_start/__data_start/__sbss/__kernel_end` |
| `os-cell3/build.rs` | 内嵌 ELF（不再 objcopy 成 bin）、链接脚本变更检测 |
| `user/src/linker.ld` | `BASE_ADDRESS = 0x10000`（所有 app 同一个 VA）|
| `user/.cargo/config.toml` | `-zmax-page-size=4096` |
| `user/src/bin/03sbrk.rs` | **新** sbrk 验收 app |

---

## 八、怎么复现

```sh
cd ~/rc-aarch/os-cell3
RUSTUP_TOOLCHAIN=stable cargo build --release

cd ~/rc-aarch
RELOAD=1 ROOTCELL=root@192.168.31.75 \
  SSHOPTS="-F /dev/null -o StrictHostKeyChecking=accept-new" \
  ./scripts/deploy-cell3.sh
```

期望看到：`remap_test passed` → `task manager: 4 个任务就绪` → 四个 app 交替输出
→ `sbrk` 的 4 行 ✓ → `All applications completed!`。

排错速查：

| 现象 | 大概率原因 |
|---|---|
| 开 MMU 后立刻死/无输出 | 页表映射漏了（内核段/MMIO），或 SCTLR/TCR 写错 |
| `remap_test: .text 不可执行` | 权限位判断用错（内核看 PXN，用户看 UXN）|
| app 在某函数开头 `EC=0x00`，反汇编正常 | 代码页 D-cache 没逐行清 → I-fetch 拿到旧内容 |
| 第一次陷阱进门就缺页 | 任务页表没挂内核 L1[3] 子树 |
| `undefined symbol: __bss_start.1` | 同一 crate 重复声明了同名 extern 符号 |

---

## 九、阶段 5 预告（ch5：进程）

ch4 的"任务"还只是"一套页表 + 一个 ELF"，ch5 要变成**进程**：

1. `fork` / `exec`：复制页表（写时复制或直接深拷贝）、替换 ELF；
2. `waitpid` / 进程退出状态、`pid` 管理（`TaskControlBlock` 变成 `Arc<Mutex<...>>`）；
3. 带参数的 `sys_exec`（用户栈上放 argc/argv）；
4. 内核栈/页表帧的**真正回收**（现在 bump 堆不回收、TCB 退出后 memory_set 还占着帧）；
5. 顺带把 ch4 的"用户指针不合法就 panic 内核"改成返回 -1（安全加固）。
