# rc-aarch — 用 AArch64 重写 rCore ch1~ch7

**A Rust bare-metal kernel written from scratch for AArch64, following rCore-Tutorial chapter by chapter (ch1–ch9), targeting the Raspberry Pi 4B as a Jailhouse inmate cell.**

The goal: there is no Rust inmate in the Jailhouse ecosystem — all upstream inmates
(inmate-demo, Zephyr, FreeRTOS) are C. This project fills that gap by reimplementing
rCore's kernel concepts (boot, trap/hypercall, task switching, paging, process, fs, IPC,
threads/sync) in Rust on AArch64, and running them as a real cell on top of
openEuler MCS + Jailhouse.

> **一句话：rCore 是"教材"，Jailhouse/MCS 是"战场"——把 rCore 各章用 AArch64 重写一遍，你就同时拿到了 Rust 系统编程能力和 MCS 项目的真实产出。**
>
> 创建：2026-09-18 ｜ 状态：**ch1 ~ ch9（线程/同步 + 块缓存）在树莓派 4B / openEuler MCS / Jailhouse cell3 上跑通**
>
> 进度：ch1 裸机 hello ｜ ch2 陷入+系统调用+批处理 ｜ ch3 任务切换+时间片 ｜
> ch4 地址空间+分页 ｜ ch5 进程（fork/exec/waitpid） ｜ ch6 文件系统（easy-fs + RAM 磁盘） ｜
> ch7 进程间通信（管道 pipe/dup + 信号 kill/sigaction/sigreturn）｜
> ch8 线程 + 同步（mutex/semaphore/condvar）+ 共享内存 stdin/stdout + user_shell ｜
> ch9 文件系统优化（块缓存 + 命中率统计）
>
> 分支：`ch1`…`ch7` 按章；`ch7+` 是演进主线（可回收堆 → ivshmem 控制台 →
> ch8 线程/同步 → ch9 块缓存 → 逐章验收）；`ch8`/`ch9` 是这条线上的两个快照分支

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
├── README.md                    ← 本文件（项目章程）
├── docs/
│   ├── 01-阶段1-对照rCore.md     ← ch1：裸机 hello（entry / linker / UART / bss）
│   ├── 02-阶段2-对照rCore.md     ← ch2：陷入/异常 + 系统调用 + 批处理
│   ├── 03-阶段3-对照rCore.md     ← ch3：任务切换 + 时间片轮转（GICv2 / CNTV）
│   ├── 04-阶段4-对照rCore.md     ← ch4：地址空间 + 分页（stage-1 MMU / 每进程页表）
│   ├── 05-阶段5-对照rCore.md     ← ch5：进程（fork / exec / waitpid）
│   ├── 06-阶段6-对照rCore.md     ← ch6：文件系统（easy-fs / open·read·write·close）
│   └── 07-阶段7-对照rCore.md     ← ch7：进程间通信（管道 + 信号）
├── os-qemu/                     ← QEMU 版内核工程（基址 0x40080000，PL011 UART）
│   ├── Cargo.toml  .cargo/  linker.ld  Makefile
│   └── src/{boot.S, main.rs}
├── os-cell3/                    ← ★ MCS cell3 版内核工程（基址 0xFA000000，mini-UART + hvc）
│   ├── Cargo.toml  .cargo/  linker.ld  Makefile  build.rs
│   └── src/
│       ├── boot.S               ← 启动汇编（对照 entry.asm）
│       ├── main.rs              ← rust_main + clear_bss + panic
│       ├── console.rs           ← print!/println!（hvc + mini-UART）
│       ├── trap/{trap.S,mod.rs,context.rs}   ← 向量表 / TrapContext / 分发
│       ├── syscall/{mod,fs,process}.rs       ← sys_write / sys_exit
│       ├── sync.rs  batch.rs    ← UPSafeCell / 批处理
│       └── link_app.S           ← build.rs 生成（内嵌 app，已 gitignore）
├── easy-fs/                     ← ch6：文件系统 crate（磁盘布局 / 位图 / inode / VFS，零外部依赖）
├── user/                        ← aarch64 用户程序 crate（app 跑在 EL0，基址 0x10000，每进程一套页表）
│   ├── Cargo.toml  .cargo/  src/linker.ld  src/{lib,syscall,console,lang_items}.rs
│   └── src/bin/*.rs             ← 每个文件一个 app
├── cells/                       ← Jailhouse cell 配置
│   └── rpi4-rust-cell3.c              （.o/.cell 产物已 gitignore）
└── scripts/
    ├── build-cell.sh            ← .c → .cell（gcc -c + objcopy）
    ├── deploy-cell3.sh          ← 编译产物 → scp → cell create/load/start → 打印控制台
    └── try-cell.sh              ← 换 cell 配置做二分排查
```

## 五、环境依赖

| 依赖 | 状态 | 安装命令 |
|---|---|---|
| `aarch64-unknown-none` target | ✅ 已装 | `rustup target add aarch64-unknown-none` |
| `rust-objcopy` / `rust-objdump` / `rust-nm` | ✅ 已有（`~/.cargo/bin`）| — |
| `qemu-system-aarch64` | ❌ 缺（只影响 QEMU 版，不影响 cell3 上机）| `sudo apt install qemu-system-arm` |
| 树莓派 4B + openEuler MCS 镜像 | ✅ | 局域网 `root@192.168.31.75`（DHCP，地址可能变：`./scripts/find-board.sh` 按 MAC 找）|

## 六、构建与运行

### ① QEMU 版（`os-qemu/`，需要 `qemu-system-aarch64`）
```bash
cd ~/rc-aarch/os-qemu
make build     # 编译
make run       # 编译 + QEMU 运行
make disasm    # 反汇编看入口
```

### ② MCS cell3 版（`os-cell3/`，跑在树莓派上 —— 主力开发路径）
```bash
cd ~/rc-aarch/os-cell3
cargo build --release          # 会顺带编译 ../user 里的 app（build.rs 负责）

cd ~/rc-aarch
SSHOPTS="-F /dev/null" ./scripts/deploy-cell3.sh \
    os-cell3/target/aarch64-unknown-none/release/rc-aarch-os
# 脚本会：编 .cell → scp → jailhouse enable（若未使能）→ destroy/create/load/start → 打印控制台
```
> 完整复现步骤（含验收标准与排错表）见 `docs/02-阶段2-对照rCore.md` 第九节。
>
> **逐章验收（真机）**：`./scripts/verify-chapters.sh` —— 按 ch1→ch9 跑一遍并打印
> PASS/FAIL 表（说明见 `docs/10-逐章验收.md`）。

### ③ 板子网络备忘（踩过的坑）

板子的 root cell 是 **initramfs**（`rootfs on /`），**IP 全靠 DHCP、`/etc` 重启即丢**，
所以地址会变、SSH 公钥也会丢。`ip addr` 里一次出现好几个 `192.168.31.x` 是正常的，
因为镜像里叠了三层来源（都是一张 `eth0`，不是多网卡）：

| 地址 | 来源 |
|---|---|
| `192.168.7.2/24` | 镜像里写死的 `/etc/systemd/network/10-eth-static.network`（出厂"直连 PC"用）|
| `192.168.31.78` | **systemd-networkd** 的 DHCP 客户端（同一文件里的 `DHCP=ipv4`）|
| `192.168.31.76` | **dhcpcd** 的 DHCP 客户端（镜像里同时开了它，和 networkd 重复 → 每次启动多领一个租约）|

想只留一个地址（比如 `192.168.31.75`）：

```sh
# ★ 顺序很重要：先补子网路由，再删地址。否则那条 192.168.31.0/24 的直连路由
#   （src 绑在要删的地址上）会跟着消失 → 板子"ARP 通、IP 不通"，只能串口/重启救。
ip route add 192.168.31.0/24 dev eth0 scope link
ip addr del 192.168.31.76/24 dev eth0
ip addr del 192.168.31.78/24 dev eth0
ip addr del 192.168.7.2/24 dev eth0
ip addr add 192.168.31.75/24 dev eth0
ip route replace default via 192.168.31.1 dev eth0
systemctl stop dhcpcd && systemctl mask dhcpcd     # 本次开机有效
```

以上都只在**本次开机**有效（`/etc` 是内存）。要重启后也稳：
1. **推荐**：路由器里做 DHCP 静态绑定，MAC `<PI_MAC>` → `192.168.31.75`；
2. 或在 PC 上改 SD 卡 BOOT 分区里的 initramfs（把 `.network` 的 `DHCP=ipv4` 改成 `DHCP=no`、
   地址写死，并 mask dhcpcd）。

> 地址变了用 `./scripts/find-board.sh` 找（并发 ping 网段 + 按 MAC 查 `ip neigh`）；
> 板子重启后 SSH 公钥会丢，用 `~/.local/bin/rc-ssh`（带密码回退）先把公钥装回去。


---

## 七、License

MIT License —— 见 [LICENSE](LICENSE)。

## 八、安全说明

本仓库是学习/研究用的公开项目。文档里出现的 `192.168.x.x` 均为**内网地址**（不可路由），
`root@<内网IP>` 仅用于说明板子的调试方式，不含任何密码或凭证。
如果你的网络环境有特殊要求，请把文档里的地址替换成占位符后再分发。
