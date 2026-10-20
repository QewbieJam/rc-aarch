# 📘 阶段 3：任务切换 + 时间片轮转 —— 对照 rCore ch3 逐项讲

> 项目：rc-aarch ｜ 工程：`os-cell3/`（MCS cell 版）+ 仓库根 `user/`（aarch64 用户程序）
> 状态：**✅ ch3 已在树莓派 4B 的 openEuler MCS + Jailhouse cell3 上跑通（3 个任务轮转、抢占 + yield）**
> 更新：2026-10-20 ｜ 分支：`ch3`（从 ch2 的 `767b09c` 分出）
> 相关：[[02-阶段2-对照rCore]] ｜ [[01-阶段1-对照rCore]] ｜ [[RPi4 MCS 内存布局与三个 Cell 的分工（750M 之谜）]]

---

## 一、验收标准（全部实测通过）

| 检查项 | 状态 | 实测证据（板子上的 hvc 控制台） |
|---|---|---|
| `__switch` 能换内核栈 + 换执行流 | ✅ | 3 个任务交替输出，且各自 `sp` 落在自己的内核栈 |
| `TaskContext` 保存 callee-saved | ✅ | yield 前后 app 的局部变量/栈都完好 |
| `loader` 给每个任务独立内核栈/用户栈 | ✅ | 各 app 的 `sp_el0` 分别是 `0xfa02_xxxx` 等不同页 |
| `sys_yield` 主动让出 | ✅ | `id=124 返回 0`，输出立刻切到下一个任务 |
| **定时器抢占**（不 yield 也能切）| ✅ | `[app2]` 一次 `yield` 都没调，仍然和 `[app1]` 交错输出 |
| `sys_exit` 后不再被调度 | ✅ | `Application exited with code 0` 之后该 app 再不出现 |
| 全部任务结束 → 收尾 | ✅ | `All applications completed!` |
| 时间片 10ms（100Hz）| ✅ | `CNTFRQ=54000000`，每 `540000` tick 一次 |
| 板子不挂、cell 可反复 RELOAD | ✅ | `jailhouse cell list` 一直是 `running`，`Failed CPUs` 为空 |

---

## 二、rCore ch3 到底做了什么

```
$ git diff --stat origin/ch2 origin/ch3 -- os/ user/
 os/src/task/context.rs  |  34 ++   ← TaskContext { ra, sp, s0..s11 }
 os/src/task/switch.S    |  40 ++   ← __switch（保存/恢复 callee-saved + sp + ra）
 os/src/task/switch.rs   |  18 ++   ← global_asm! 包装
 os/src/task/task.rs     |  17 ++   ← TaskControlBlock / TaskStatus
 os/src/task/mod.rs      | 190 ++   ← TaskManager（轮转调度）+ run_first_task
 os/src/loader.rs        |  40 +-   ← 每任务 KernelStack/UserStack + init_app_cx + load_apps
 os/src/timer.rs         |  12 +-   ← set_next_trigger()
 os/src/config.rs        |  12 ++   ← USER/KERNEL_STACK_SIZE、MAX_APP_NUM、APP_BASE
 os/src/trap/mod.rs      |  20 +-   ← Timer 中断 → suspend_current_and_run_next
 os/src/syscall/process.rs| 18 ++   ← sys_exit / sys_yield / sys_get_time
 os/src/main.rs          |  10 +-   ← load_apps → run_first_task（不再 batch）
 user/build.py           |  25 ++   ← ★ 每个 app 改一次 linker.ld（各自 BASE_ADDRESS）
 user/src/{lib,syscall}.rs, bin/01yield.rs, bin/02power.rs ...
```

一句话：ch2 是"一个 app 跑完换下一个"（批处理），ch3 变成**多个任务同时驻留 + 时间片轮转**。
三个新概念：**任务上下文（TaskContext）**、**切换（`__switch`）**、**调度器（TaskManager）**。

---

## 三、逐项对照：rCore (RISC-V) ↔ rc-aarch (AArch64)

### 3.1 TaskContext：到底哪些寄存器要存？

| rCore `TaskContext` | rc-aarch `TaskContext` | 说明 |
|---|---|---|
| `ra: usize` | `ra: usize`（x30/lr）| |
| `sp: usize` | `sp: usize`（**SP_EL1**）| 断点处的内核栈 |
| `s: [usize; 12]`（s0~s11）| `s: [usize; 11]`（x19~x28 + x29/fp）| ★ **数字不一样，必须重算** |

AArch64 AAPCS64 的 callee-saved 是 `x19~x28`（10 个）+ `x29(fp)`；`x30(lr)` 单独放 `ra`。
调用者保存的 `x0~x18` 不用管 —— 编译器在 `bl __switch` 前后会自己处理。
（d8~d15 浮点 callee-saved 本阶段不用，ch4 之后若内核出现浮点必须补。）

### 3.2 `__switch`：RISC-V ↔ AArch64

| rCore `switch.S` | rc-aarch `switch.S` |
|---|---|
| `sd sp, 8(a0)` | `str x2, [x0, #8]`（先 `mov x2, sp`）|
| `sd ra, 0(a0)` | `str x30, [x0, #0]` |
| `sd s0..s11, (n+2)*8(a0)` | `stp x19,x20 ... str x29`（16..96）|
| `ld ra, 0(a1)` / `ld sp, 8(a1)` / `ret` | `ldr x30,[x1,#0]` / `ldr x2,[x1,#8]` / `mov sp,x2` / `ret` |

### 3.3 第一次进用户态：`__restore` vs `__restore_sp`（★ AArch64 特有）

rCore 的 `TaskContext::goto_restore(kstack_ptr)` 把 `ra = __restore`、`sp = kstack_ptr`，
因为 RISC-V 的 `__restore` 第一句就是 `mv sp, a0`，而 `a0` 在 `__switch` 之后是**不可用**的
（调用者保存寄存器不会被恢复）—— upstream 其实靠 `sp` 恰好已经等于 `a0` 才成立。

AArch64 上我们把它写明白，**新加一个入口**：

```asm
__restore_sp:            /* 地址直接来自 sp，不从 x0 取 */
    RESTORE_REGS
    eret
```

于是 `task_cx = { ra: __restore_sp, sp: &TrapContext }`，`__switch` 换完栈 `ret` 过来，
`RESTORE_REGS` 把 EL0 的现场恢复好、`eret` 下去 —— 任务就跑起来了。

控制流一眼图（A 是刚被中断的任务，B 是下一个）：

```
EL0 app A ──IRQ──▶ EL1 向量#9 ──▶ irq_handler ──SAVE_REGS──▶ irq_dispatch
                                                              │ ack → set_next_trigger → eoi
                                                              ▼
                                                     suspend_current_and_run_next
                                                              │
                                    __switch(A.cx, B.cx) ─────┘   （A 的 sp 停在 irq_dispatch 里）
                                                              │
                              B 没跑过 → sp=trap_cx_B, ra=__restore_sp
                                                              ▼
                                        __restore_sp → RESTORE_REGS → eret → EL0 app B
```

### 3.4 其它逐项

| rCore ch3 | rc-aarch | 说明 |
|---|---|---|
| `sie::set_stimer()` + `set_timer(mtime+freq/100)` | `GICD_ISENABLER(PPI27)` + `CNTV_TVAL_EL0 = cntfrq/100` | AArch64 没有 SBI，直接写 EL1 虚拟定时器 |
| `Trap::Interrupt(SupervisorTimer)` | 向量表 **#5/#9** 的 `irq_dispatch` → `IRQ_TIMER` | 中断和同步异常是**两个入口** |
| `trap_handler` 返回 `&mut TrapContext` | `trap_handler(cx)` 就地改 + 内联 `RESTORE_REGS` | |
| `UPSafeCell` + `lazy_static!` | `UPSafeCell` + `const fn` 静态构造 + `init()` | 我们没有 `lazy_static` 依赖 |
| `KERNEL_STACK: [KernelStack; 4]` | `[KernelStack; 16]`，`static mut` | 16×8KB + 16×8KB ≈ 256KB（< 1MB LOADABLE 区）|
| 每个 app 链到 `APP_BASE + i*SIZE` | **同**（用链接器 `--defsym=__app_base` 实现）| ★ 见坑 3 |

---

## 四、实测输出（2026-10-20，hvc 虚拟控制台）

```
Started cell "rc-aarch"
[rc-aarch] stage 3: task switch + 时间片轮转 on Jailhouse cell3
[kernel] num_app = 3
[kernel] load app_0 (8720 字节) → 0xfa100000
[kernel] load app_1 (8704 字节) → 0xfa120000
[kernel] load app_2 (8702 字节) → 0xfa140000
[kernel] task manager: 3 个任务就绪
[comm] sig=JHCOMM rev=2 cell_state=0 gic_version=2
[comm] gicd=0xff841000 gicc=0xff842000 gicr=0x0 vpci_irq_base=0
[timer] CNTFRQ=54000000 Hz（每 540000 个 tick 中断一次 → 100 Hz）
[gic] gicd=0xff841000 gicc=0xff842000 —— 初始化（CTLR/PMR/ISENABLER）
[timer] 定时器已武装；IRQ 在内核态保持屏蔽，eret 到 EL0 后自动放行
[irq] 第 1 次 tick（来自 EL0）
[app1] A0
Hello, world from EL0 (user mode!)
[app2] B0
0[app2] B[app1] A1
1
[app1] A2
[app1] A3
[app2] B2
[app1] A4
[kernel] Application exited with code 0
[app2] B3
[app2] B4
[kernel] Application exited with code 0
[kernel] All applications completed!
[trap] shutdown：进入死循环
```

看点：
- `[irq] 第 1 次 tick（来自 EL0）` —— 定时器真的打断了用户态；
- `[app1]`（每行 `yield`）和 `[app2]`（**一次都没 yield**，纯忙等）**交错**出现
  → 既有协作式让出，也有**抢占式**时间片；
- `[app2] B` 被切成两段（`[app2] B` … 下一行 `1`）—— 因为 Rust 的 `println!` 对
  "字符串 + 整数 + 换行"会调 **3 次** `write_str`，中间被 tick 打断换走了。这恰好是
  "抢占真的生效"最直观的证据（也说明 syscall 不是原子的，ch4 以后可以考虑合并写）。

---

## 五、★ ch3 踩到的坑（按"值钱程度"排序）

### 坑 1（架构级，最坑）：**MMU 关闭时不能用 `core::sync::atomic`**

AArch64 在 **stage-1 MMU 关闭**（`SCTLR_EL1.M = 0`）时，把**所有**数据访问都当成
**Device-nGnRnE** 内存。而架构规定 exclusive 访问（`LDXR/STXR`）**不支持 Device 内存**
→ `STXR` **永远失败**。

Rust 的 `AtomicUsize::fetch_add` 在 `aarch64-unknown-none`（generic，没有 LSE）上正好编译成：

```asm
1:  ldxr    x8, [x9]
    add     x8, x8, #1
    stxr    w10, x8, [x9]
    cbnz    w10, 1b        ← 永远从这里跳回去
```

**现象**：控制台停在 `[timer] 中断已开，等 tick`，`jailhouse cell list` 仍是 `running`，
`/sys/devices/jailhouse/cells/1/statistics/*` 里的计数器**完全冻结**（说明不是 vmexit 卡住，
是 guest 自己在 EL1 死循环）。

**定位过程**（很值得记，因为以后还会用到）：用"往控制台打点"的办法逐段二分 ——
`进入 handler` ✅ → `ack` ✅（IAR=27）→ `eoi` ✅ → `fetch_add` ❌ 卡死。

**对策**：单 CPU（cell 只有 CPU3）+ 中断在 handler 里本来就屏蔽（`PSTATE.I=1`），
根本不需要原子性 → 用 `static mut` + `read_volatile/write_volatile`。
**ch3 及以后都别用 `core::sync::atomic`**，等 ch4 打开 MMU、内存变成 Normal 属性之后再说。

> 顺带：`static mut` 数组还解决另一个坑 —— 不可变 `static` 会被放进 `.rodata`（只读），
> 而栈是要写的。ch2 已经踩过一次（见 02 文档坑 2），ch3 的 `KERNEL_STACK/USER_STACK` 沿用 `static mut`。

### 坑 2：IRQ handler **必须 ack + EOI**，否则活锁（不是死机，是原地打转）

Jailhouse 用 **GICH_LR（list register）** 把物理中断注入成虚拟中断。guest 不读
`GICC_IAR` / 不写 `GICC_EOIR`，那条 LR 就一直 **active** → `eret` 回 EL1 的瞬间同一条
中断立刻又进来 → CPU 永远在 `handler ↔ eret` 之间打转：

- 控制台一行新东西都没有（主循环根本没机会跑）；
- `jailhouse cell list` 仍是 `running`，`Failed CPUs` 为空；
- **板子不挂**，只是这个 cell 的 CPU 空转。

我们最初为了"隔离变量"故意写了一个 `MINIMAL_IRQ_HANDLER`（只计数、不碰 GIC），
结果就是它把自己活锁死了。正确顺序：

```
ack (读 GICC_IAR) → 处理 → set_next_trigger → eoi (写 GICC_EOIR) → 再切任务
```

★ EOI 必须在**切换任务之前**：先把中断真撤销，再让别人跑。

### 坑 3（ch3 新增，隐蔽）：**多个 app 必须各自链接到自己的加载地址**

ch2 的批处理一次只跑一个 app，所有 app 都装在 `0xFA100000`，所以"全部按
`0xFA100000` 链接"没毛病。ch3 的多个任务**同时驻留**：

```
app_i 的加载地址 = 0xFA100000 + i * 0x20000
```

如果还按同一个地址链接，app_1 里那些**绝对地址**（链接器符号 `start_bss/end_bss`、
`.data` 里的指针）会指向 app_0 的地盘 —— 症状是"清 bss 清掉别人的数据"，
最后 **pc 跳到 0x0**（hypervisor 打印 `FATAL: instruction abort at 0x0`）。

upstream 的 `user/build.py` 是"每编一个 app 就把 `linker.ld` 里的 BASE_ADDRESS 改一次"。
我们改成链接器的 `--defsym`（一个脚本搞定，不改文件）：

```
# user/src/linker.ld
BASE_ADDRESS = DEFINED(__app_base) ? __app_base : 0xFA100000;

# os-cell3/build.rs（每个 app 单独 cargo rustc）
cargo rustc --release --bin <app> -- -Clink-arg=--defsym=__app_base=0xFA120000
```

> AArch64 上整块代码平移还能跑（函数调用是 PC 相对的），所以这个 bug 不会立刻炸，
> 而是"跑着跑着跳到 0x0"—— 特别难查。

### 坑 4：**控制台只有 2KB，而且 `/sys/devices/jailhouse/console` 只给前 2KB**

- `/sys/devices/jailhouse/console` 是"**最早** 2048 字节的快照"，写满就再也不更新 →
  跑几次以后看到的永远是开头那 2KB，很容易误判成"卡住了"。
- 真正的增量接口是 **`/dev/jailhouse`**：每个 fd 自己记 head，`read` 只给上次之后的
  **新**内容；一次增量超过 2048 时先返回一行 `<missed N bytes>` 并把 head 前移，
  **再读一次**才给数据。
- ★ 必须让 `dd` **继承** fd（`<&3`）；写成 `if=/dev/fd/3` 会重新 open 设备，每次 head
  都从 0 开始 → 永远收到同一行 `<missed ...>`（我们在这上面绕了半天）。

`scripts/deploy-cell3.sh` 里的做法：

```sh
exec 3</dev/jailhouse
drain_console() { ... dd ... <&3 ... }   # 先吃掉旧内容
jailhouse cell load ... && jailhouse cell start ...
while [ $idle -lt 10 ]; do               # 边跑边抓，每 200ms 一次，2s 没新数据才停
    out=$(dd bs=8192 count=1 iflag=nonblock status=none <&3 2>/dev/null)
    ...
done
```

### 坑 5：`global_asm!(include_str!("trap.S"))` 里 **`.S` 注释不能有花括号**

`global_asm!` 把汇编文件当**格式串**解析，注释里的 `{ ra: ..., sp: ... }` 会被当成
格式占位符 → 编译报 `invalid asm template string: expected '}'`。注释里写 `{` 要转义
或干脆别写。

### 坑 6（沿用 ch2 的结论）：**别用 `cell destroy` 做日常重载**

destroy 会把 CPU3 还给 root cell，Linux 热插拔 CPU3 时在 `secondary_start_kernel` /
`el1_irq` 里吃 SError → `Kernel panic` → 整板失联，只能断电。
日常改内核用 **`RELOAD=1`**：`cell shutdown` → `cell load` → `cell start`（不 destroy），
CPU3 始终留在 cell 里。`scripts/deploy-cell3.sh` 已经支持。

---

## 六、文件清单（ch3 新增/改动）

| 文件 | 作用 |
|---|---|
| `os-cell3/src/config.rs` | **新** USER/KERNEL_STACK_SIZE、MAX_APP_NUM、APP_BASE_ADDRESS |
| `os-cell3/src/loader.rs` | **新** KernelStack/UserStack 数组、`load_apps()`、`init_app_cx()` |
| `os-cell3/src/task/mod.rs` | **新** TaskManager（轮转）+ run_first_task + suspend/exit |
| `os-cell3/src/task/context.rs` | **新** TaskContext（ra/sp/x19~x29）+ `goto_restore` |
| `os-cell3/src/task/switch.S` | **新** `__switch`（AArch64 版）|
| `os-cell3/src/task/switch.rs` | **新** `global_asm!` 包装 |
| `os-cell3/src/task/task.rs` | **新** TaskControlBlock / TaskStatus |
| `os-cell3/src/trap/trap.S` | 加 `__restore_sp` |
| `os-cell3/src/trap/mod.rs` | `irq_dispatch` 去掉 atomic / 接上任务切换；`init_timer_irq()` |
| `os-cell3/src/syscall/{mod,process,fs}.rs` | 加 124 yield / 169 get_time；sys_exit 走调度器 |
| `os-cell3/src/main.rs` | 改成 load_apps → task::init → run_first_task |
| `os-cell3/src/{gic,timer,comm_region}.rs` | ch3-A 的 GIC/定时器/通信区（已在前一步验证）|
| `os-cell3/build.rs` | ★ 每个 app 用自己的链接基址（`--defsym=__app_base`）|
| `user/src/linker.ld` | `BASE_ADDRESS = DEFINED(__app_base) ? __app_base : 0xFA100000` |
| `user/src/{lib,syscall}.rs` | `yield_()` / `get_time()` |
| `user/src/bin/01yield.rs` | **新** 每行 yield（协作式）|
| `user/src/bin/02busy.rs` | **新** 忙等不 yield（验证抢占）|
| `scripts/deploy-cell3.sh` | ★ `RELOAD=1` 路径 + `/dev/jailhouse` 增量读控制台 |

---

## 七、怎么复现（日常开发循环）

```sh
# ① 编内核（build.rs 会顺带编 ../user 的 app，每个 app 链到自己的基址）
cd ~/rc-aarch/os-cell3
RUSTUP_TOOLCHAIN=stable cargo build --release
#   ⚠️ 本机默认 toolchain 是 1.83（不支持 edition2024），必须用 stable/nightly

# ② 部署：RELOAD=1 只 scp 新 ELF + shutdown/load/start（不 destroy）
cd ~/rc-aarch
RELOAD=1 ROOTCELL=root@192.168.31.75 \
  SSHOPTS="-F /dev/null -o StrictHostKeyChecking=accept-new" \
  ./scripts/deploy-cell3.sh
```

### 7.1 期望输出（逐行对账）

见第四节，关键三行：

```
[kernel] task manager: 3 个任务就绪
[irq] 第 1 次 tick（来自 EL0）
[kernel] All applications completed!
```

### 7.2 跑挂了怎么救（RELOAD 路径）

```sh
# cell 还活着（running）→ 直接再 RELOAD 一次
# 万一 cell 变成 failed / 想彻底重来（★ 有 SError 风险，优先关机重启板子）
ssh root@192.168.31.75 'jailhouse cell shutdown --name rc-aarch'
```

### 7.3 工具链小抄

| 项 | 值 |
|---|---|
| 内核/app 编译 toolchain | `RUSTUP_TOOLCHAIN=stable`（1.99）或 `nightly-2025-05-20` |
| target | `aarch64-unknown-none` |
| 内核链接基址 | `0xFA000000`（= cell virt_start = cpu_reset_address）|
| app 基址 | `0xFA100000 + i*0x20000`（cell 的 31MB RWX 区）|
| 定时器 | `CNTV_TVAL_EL0 = CNTFRQ/100`（54MHz → 540000 → 100Hz）|
| 定时器 PPI | 27（Group 1！复位是 Group 0，要写 `GICD_IGROUPR0`）|

---

## 八、阶段 4 预告（ch4：地址空间 / 分页）

ch3 是"不分页的多任务"：所有 app 共用同一套物理地址，互相能踩到（只是我们给每个
app 分了固定槽位）。ch4 要做的：

1. 打开 **stage-1 MMU**（`SCTLR_EL1.M=1`），自己建页表（AArch64 的 4 级页表 + MAIR 属性）；
2. ★ 顺带解决坑 1：内存变成 Normal 属性之后，`core::sync::atomic` 才真正可用；
3. 每个任务一套页表（`TTBR0_EL1`），`APP_BASE` 统一成 `0x0` 或 `0x10000`；
4. `sys_write` 的指针检查改成"查页表"而不是"落没落在 cell 里"；
5. 注意 Jailhouse 的 **stage-2**（hypervisor）和我们的 stage-1 是叠加关系，
   页表属性（Device/Normal）要一致，否则会出 "Unhandled data abort"。

> 在打开 MMU 之前，**本阶段的 `static mut` + 不用 atomic 的写法要一直保持住**。

---

## 九、附：顺手把公钥烧进 MCS 镜像（解决"重启丢公钥"）

root cell 是 **initramfs**，重启后 `/root/.ssh/authorized_keys`（还有 sshd host key）
都会丢，每次上电都得先用密码登录一次装公钥。所以在本地的
`meta-openeuler-bsp` 里加了一段 bbappend：

```
bsp/meta-openeuler-bsp/raspberrypi/recipes-core/images/openeuler-image-mcs.bbappend
    + files/rc-aarch-authorized_keys            ← 内容就是 ~/.ssh/id_ed25519.pub

rc_aarch_install_authorized_keys() { ... }      ← ROOTFS_POSTPROCESS_COMMAND
```

两个小坑：

1. **image recipe 的 `do_fetch/do_unpack` 是 noexec** —— 写 `SRC_URI += "file://..."` 没用，
   文件根本不会进 `${WORKDIR}`（实测日志：`公钥文件不存在`）。直接用 layer 里的
   绝对路径（容器里 `/usr1/openeuler/src` = 宿主 `~/EMCS/src`）。
2. **`${THISDIR}` 必须用 `:=` 立即展开**。`?=`/`=` 是延迟展开，等 `do_rootfs` 跑的时候
   `THISDIR` 已经被覆盖成 base recipe 的目录了 → 路径跑到 `meta-openeuler/recipes-core/images/files/`。
   写成 `RC_AARCH_SSH_AUTHORIZED_KEYS_FILE := "${THISDIR}/files/..."` 才对。

验证：

```sh
cd ~/EMCS/build/raspberrypi4-64 && oebuild bitbake openeuler-image-mcs
R=tmp/work/raspberrypi4_64-openeuler-linux/openeuler-image-mcs/1.0-r0/rootfs
ls -la $R/root/.ssh/            # drwx------ + authorized_keys -rw-------
cat  $R/root/.ssh/authorized_keys   # 就是你的公钥
```

> ⚠️ 这仍然**不能**省掉上电后的 `jailhouse enable`（那是运行期状态），但至少
> `ssh root@<board>` 可以直接进，不用再走密码/keyboard-interactive。
