# 📘 阶段 7：进程间通信（管道）—— 对照 rCore ch7 逐项讲

> 项目：rc-aarch ｜ 工程：`os-cell3/` + `easy-fs/` + `user/`
> 状态：**✅ ch7 已完成并在树莓派 4B 的 openEuler MCS + Jailhouse cell3 上跑通**
> （File trait、`pipe(59)`/`dup(24)`、阻塞读写；`kill/sigaction/sigprocmask/sigreturn`；
> `exec` 带 argv；**SIMD/FP 现场跨 trap 保存**——最后这个是本章最隐蔽的 bug）
> 更新：2026-10-22 ｜ 分支：`ch7`（从 ch6 的 `cd2563a` 分出）
> 相关：[[06-阶段6-对照rCore]] ｜ [[05-阶段5-对照rCore]] ｜ [[04-阶段4-对照rCore]]

---

## 一、验收标准（实测通过）

| 检查项 | 状态 | 实测证据 |
|---|---|---|
| `File` trait：fd 背后可以是文件/管道/标准输入输出 | ✅ | `fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>` |
| `UserBuffer`：跨页用户缓冲区 | ✅ | `read/write` 统一传 `UserBuffer` |
| `sys_pipe(59)`：返回两个 fd | ✅ | `[pipetest] pipe: read_fd=3 write_fd=4` |
| `sys_dup(24)` | ✅ | 已实现（暂无专门 app）|
| 管道阻塞读（空 → 让出 CPU）| ✅ | `pipe_large_test` 读完 3000 字节 |
| 管道阻塞写（满 → 让出 CPU）| ✅ | 3000 字节穿过 **32 字节** 环不丢不重 |
| EOF：所有写端关闭 → 读端返回 0 | ✅ | pipetest 子进程读到 13 字节后正常退出 |
| fork 继承 fd（父子共享同一根管道）| ✅ | 父子分别持读/写端 |
| 既有能力回归 | ✅ | filetest/cat/forktest 全过；停机页帧 7737/8027 |
| **信号**：装 handler + kill 自己 + sigreturn | ✅ | `[sig] ★ 进入信号处理函数` → `① handler + sigreturn 正常` |
| 信号屏蔽（sigprocmask）| ✅ | 屏蔽时 kill 挂起，解除后补投 |
| 非法 sigaction | ✅ | 信号号 50 → -1 |
| SIGKILL 杀子进程 | ✅ | `waitpid=4 code=-9` |
| 异常 → SIGSEGV/SIGILL | ✅ | 缺页/非法指令挂信号，没 handler 就以 -11/-4 退出 |
| `exec(path, argv)` 带命令行参数 | ✅ | `[cmdline] argc = 3`、`argv[1] = hello` |
| 用户 SIMD/FP 现场跨系统调用保持 | ✅ | `simd_test`：`② 向量寄存器跨系统调用保持 ✓` |
| 自动向量化的用户循环结果正确 | ✅ | `simd_test`：`③ 自动向量化填充 3000 字节全对` |

---

## 二、upstream ch7 做了什么

```
$ git diff --stat origin/ch6 origin/ch7 -- os/
 os/src/fs/mod.rs        | File trait（readable/writable/read/write）
 os/src/fs/pipe.rs       | 173 +++  管道：环形缓冲 + 阻塞读写 + Weak 写端
 os/src/fs/{inode,stdio}.rs | OSInode/Stdout/Stdin 实现 File
 os/src/mm/page_table.rs | +UserBuffer（跨页缓冲区迭代器）
 os/src/syscall/fs.rs    | +sys_pipe/sys_dup，读写改走 File
 os/src/task/task.rs     | fd_table 变成 Arc<dyn File>
 os/src/task/{signal,action}.rs | +信号（sigaction/sigprocmask/kill/sigreturn）
 user/src/bin/{pipetest,pipe_large_test,user_shell,...}
```

两件事：**管道（本章主线）** + **信号（另一大块）**。
另外 upstream ch7 顺手把 `exec` 改成能传 argv（给 `user_shell` 用）。
本章三块都做了；顺带挖出一个 upstream 在 RISC-V 上**永远不会遇到**的 AArch64 坑：
用户程序的 SIMD/FP 现场必须由内核保存（见"坑 6"）。

---

## 三、我们做了什么

1. **`File` trait 重构**（对齐 upstream）
   ```rust
   pub trait File: Send + Sync {
       fn readable(&self) -> bool;
       fn writable(&self) -> bool;
       fn read(&self, buf: UserBuffer) -> usize;
       fn write(&self, buf: UserBuffer) -> usize;
   }
   ```
   实现者：`OSInode`（文件）、`Pipe`（管道）、`Stdin`/`Stdout`（标准输入输出）。
   `fd_table` 因此变成 `Vec<Option<Arc<dyn File + Send + Sync>>>`，0/1/2 一开始就装着 stdio。

2. **`UserBuffer`**：把"跨页的用户缓冲区"（`Vec<&'static mut [u8]>`）包起来，
   实现 `IntoIterator<Item = *mut u8>`，管道的逐字节读写直接用它。

3. **`fs/pipe.rs`**：和 upstream 同构
   - 32 字节环形缓冲（`head/tail/status`）；
   - 读端空且写端还活着 → `suspend_current_and_run_next()`；
   - 读端空且写端全关（`Weak::upgrade() == None`）→ 返回已读字节数（EOF）；
   - 写端满 → 让出 CPU。

4. **系统调用**：`pipe(59)`（往用户 `usize[2]` 写两个 fd）、`dup(24)`；
   `read/write` 不再特判 stdout，统一查 fd 表。

5. **信号**（`kill(129)/sigaction(134)/sigprocmask(135)/sigreturn(139)`）：
   - TCB 里加 `signals / signal_mask / handling_sig / signal_actions / killed / frozen /
     trap_ctx_backup`；
   - `current_add_signal`：缺页→SIGSEGV、非法指令→SIGILL；
   - 每次从内核返回用户态前（**同步异常和 IRQ 两条路径**）调 `handle_signals()`：
     有 handler 的信号 → 备份 TrapContext、`elr_el1 = handler`、`x0 = 信号号`，
     处理函数里必须调 `sigreturn()` 恢复现场；
     没有 handler 的错误信号 → 按 `check_error` 映射成退出码（SEGV=-11、ILL=-4、KILL=-9）；
   - `SIGSTOP/SIGCONT` 用 `frozen` + 调度让出实现；
   - `kill(pid)` 需要 pid→TCB 表（`BTreeMap`，`add_task` 时登记、进程退出时摘除）。

6. **`exec` 带 argv**：`sys_exec(path, argv)` 从用户态读一个 NULL 结尾的指针数组，
   把字符串和指针数组压到新地址空间的用户栈上，入口寄存器 `x0 = argc`、`x1 = argv`
   （AArch64 对应 RISC-V 的 `a0/a1`），`_start` 解析成 `&[&str]` 再交给 `main`。

7. **SIMD/FP 现场保存**：`TrapContext` 从 272 字节扩到 **800 字节**（+`q[32]`、`FPCR`、`FPSR`），
   `SAVE_REGS`/`RESTORE_REGS` 里 16 条 `stp q, q` 全存全取。这是修"坑 6"的补丁。

---

## 四、逐项对照

| rCore ch7 | rc-aarch |
|---|---|
| `fs/mod.rs` 的 `File` trait | **同** |
| `fs/pipe.rs`：`PipeRingBuffer` + `Pipe` + `make_pipe` | **同**（32 字节环、Weak 写端、阻塞语义）|
| `mm::UserBuffer` + 迭代器 | **同** |
| `sys_pipe(59)` / `sys_dup(24)` | **同** |
| `fd_table: Vec<Option<Arc<dyn File>>>` | **同** |
| `Stdin` 用 `console_getchar` 轮询 | 我们还没有 UART 输入，先返回 0（EOF）；**TODO** |
| `sys_exec` 带 argv（cmdline_args）| **同**（AArch64 用 `x0/x1` 传 argc/argv）|
| 信号（kill/sigaction/sigprocmask/sigreturn）| **同**（见下面"信号的 AArch64 适配"）|

---

## 五、实测输出（2026-10-21）

```
[fs] 挂载成功，根目录: [..., "pipe_large_test", "pipetest"]
[initproc] 子进程 exec pipetest
[pipetest] pipe: read_fd=3 write_fd=4
[pipetest] 子进程读到 "Hello, world!"，退出
[pipetest] passed.
[initproc] 子进程 exec pipe_large_test
[pipe_large] down: r=3 w=4, up: r=5 w=6
[pipe_large] 父进程发出 3000 字节，校验和 382468
[pipe_large] 子进程收到 3000 字节，校验和 382468
[pipe_large] 子进程退出
[pipe_large] 父进程收到子进程校验和 382468
[pipe_large] passed.
...
[initproc] 子进程 exec simd_test
[simd] ① movi/mul/str q 正常
[simd] ② 向量寄存器跨系统调用保持 ✓
[simd] ③ 自动向量化填充 3000 字节全对
[simd] simd_test passed.
[initproc] 子进程 exec cmdline_args
[cmdline] argc = 3
[cmdline] argv[1] = hello
...
[kernel] shutdown：initproc exit_code=0；页帧 7700/7996 空闲
```

`pipe_large_test` 是本章的硬指标：**3000 字节 vs 32 字节的环**，意味着写端要阻塞 ~94 次、
读端要唤醒 ~94 次；能跑通说明"管道阻塞 + 调度切换"是自洽的。

---

## 六、本章的坑

### 坑 1（今天踩的）：`exec` 用 `Vec` 读 ELF，把内核 bump 堆撑爆了

现象：第 3 个 exec（`pipetest`，ELF 44KB）时内核 panic 在 `alloc.rs: handle_alloc_error`。

原因：内核堆是**只增不减**的 bump 分配器；`OSInode::read_all` 用 `Vec` 追加，扩容时
`512 → 1024 → … → 64KB` 的旧缓冲全部泄漏。一个 44KB 文件就能吃掉 ~128KB 堆。

修法：`sys_exec` 改成 **页帧缓冲**（`mm::FrameBuf`）：
- 先 `Inode::size()` 拿到确切大小；
- `frame_alloc_contig(ceil(size/4K))` 拿连续页帧；
- `read_file_to(name, &mut slice)` 一次性读进来；
- exec 完 `Drop` 自动 `frame_dealloc_range`。
好处：长度精确（不翻倍）、内存走页帧分配器（可回收）。

> 教训：**bump 分配器只适合"只分配不释放"的启动期对象**；运行期反复分配的路径
> （exec、每次系统调用的 `translated_byte_buffer`）要么改成页帧缓冲，要么换真正的分配器。
> 现在 `translated_byte_buffer` 每次系统调用仍会漏几十字节，量小可接受，但已记在 TODO。

### 坑 2：阻塞前必须 `drop` 掉环形缓冲的借用

`Pipe::read/write` 里 `self.buffer.exclusive_access()` 是 RefCell 借用；
如果拿着借用去 `suspend_current_and_run_next()`，切回来时（或别的进程访问同一管道时）
会直接 `already borrowed` panic。upstream 也是先 `drop(ring_buffer)` 再 suspend。

### 坑 3：EOF 靠"写端的 Weak 全部失效"判断

`PipeRingBuffer` 只保存写端的 `Weak<Pipe>`。父子进程各持一个 `Arc<Pipe>`；
当**所有**写端 fd 都被 close（Arc 归零）后，`upgrade()` 返回 None，读端才能返回 0。
如果用 `Arc` 存就永远关不掉（循环引用），读端会永远阻塞。

### 坑 4：fd 编号不是随便的

`pipetest` 断言 `read_fd == 3`、`write_fd == 4`。这要求：
- fd 表 0/1/2 一开始就装着 stdio（否则 `alloc_fd` 会从 0 开始分配）；
- 进程在 pipe 之前没有打开别的文件。

### 坑 5：`translated_refmut` 要泛型

`sys_pipe` 要往用户内存写两个 `usize`，`waitpid` 写一个 `i32`。
ch6 的 `translated_refmut` 写死了 `i32`，ch7 泛化成 `translated_refmut<T>`。

### 坑 6（★ 本章最隐蔽）：用户 SIMD/FP 现场在系统调用后被内核改坏

**现象**：`pipe_large_test` 的校验和对不上——父进程说发出的是 382468，子进程收到
2952 个字节是错的，而且两边内容一模一样（校验和相同，所以**只比较校验和时"通过"**）。
用逐字节比对才暴露出来：父进程填完 `data[]` 后**还没开始写管道**，内容就已经全错——
`data[0..16]` 全是 3、`data[16..32]` 全是 115。

**原因**：AArch64 的 **EL0 和 EL1 共用同一套 SIMD/FP 寄存器**（q0~q31、FPCR、FPSR），
切换特权级不会自动保存。而我们的用户程序是 rustc 编的 AArch64 代码，`aarch64-unknown-none`
默认开 `+neon`，LLVM 会把 `for i in 0..3000 { data[i] = (i*7+3) as u8 }` 自动向量化成
"`movi v0.16b,#7` … `mul v5.16b, v4.16b, v0.16b`"；内核也是同一套代码，`memcpy`/格式化输出
会用 q0~q7。编译器的调度恰好把 `movi v0..#7` 放在一次 `close()` 系统调用**之前**——
系统调用一进内核，v0 就被写成别的值，回到用户态后循环拿错的常量算，于是数组内容整片错掉。
**不崩溃、不越界，只是数据悄悄错**，是最难查的一类 bug。

**为什么 upstream 不会踩**：rCore 跑 RISC-V，用户程序不用浮点，TrapContext 只存整数寄存器；
这是 AArch64 移植特有的问题（见 docs/04 的 I-cache 坑，同理："架构差异让移植多出一类 bug"）。

**修法**：`TrapContext` 增加 `q[32] + fpcr + fpsr`，`trap.S` 的 `SAVE_REGS`/`RESTORE_REGS`
各加 16 条 `stp/ldp q, q` 和 `mrs/msr fpcr/fpsr`。代价：每次 trap 多 528 字节访存。
（另一种做法是关掉内核的 NEON，但 `core`/`compiler_builtins` 是预编译的，关不干净。）

**顺带加固验收**：`pipe_large_test` 现在不只比校验和，还会**逐字节**比对 3000 字节的
内容；另加 `simd_test` 固定三件事：EL0 能执行 SIMD、v7 跨 `getpid()` 保持、
3000 字节自动向量化填充全对。

> 教训：**只比较"两边算出来的摘要"的测试，在两边被同一个 bug 写坏时会假通过**
> （父子进程共享同一份错误数据，校验和当然一样）。跨特权级的验收要直接比内容，
> 并单独构造"跨系统调用后状态还在不在"的用例。

---

## 七、文件清单（ch7 改动）

| 文件 | 作用 |
|---|---|
| `os-cell3/src/fs/mod.rs` | 重写：`File` trait + init（拆分成 block/inode/pipe/stdio）|
| `os-cell3/src/fs/block.rs` | **新**（从旧 fs.rs 拆出）RAM 磁盘 |
| `os-cell3/src/fs/inode.rs` | **新** OSInode/OpenFlags/open_file + `file_size`/`read_file_to` |
| `os-cell3/src/fs/pipe.rs` | **新** 管道 |
| `os-cell3/src/fs/stdio.rs` | **新** Stdin/Stdout |
| `os-cell3/src/mm/page_table.rs` | +`UserBuffer`、`translated_refmut<T>` |
| `os-cell3/src/mm/frame_allocator.rs` | +`FrameBuf`（exec 读 ELF）|
| `os-cell3/src/task/task.rs` | fd_table 换成 `Arc<dyn File>`，0/1/2 预装 stdio |
| `os-cell3/src/syscall/fs.rs` | 读写走 File；+sys_pipe/sys_dup |
| `os-cell3/src/syscall/process.rs` | exec 改用 FrameBuf + read_file_to |
| `os-cell3/src/syscall/mod.rs` | 24 / 59 |
| `easy-fs/src/vfs.rs` | +`Inode::size()` |
| `user/src/{syscall,lib}.rs` | +dup/pipe |
| `user/src/bin/{pipetest,pipe_large_test}.rs` | **新** 验收 app（large 版：4 字节校验和 + 逐字节比对）|
| `user/src/bin/{simd_test,cmdline_args}.rs` | **新** 验收 app：SIMD 现场 / argv |
| `os-cell3/src/trap/context.rs` | **改** TrapContext 272 → 800 字节（+q[32]/FPCR/FPSR）|
| `os-cell3/src/trap/trap.S` | **改** SAVE/RESTORE_REGS 加 SIMD/FP 现场 |
| `os-cell3/src/main.rs` | +启动时打印 `CPACR_EL1`（确认 FPEN=3，EL0 可用 SIMD）|
| `user/src/bin/initproc.rs` | 依次跑 8 个 app |

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

期望：`[pipetest] passed.`、`[pipe_large] passed.`（校验和 382468、逐字节全对）、
`[simd] simd_test passed.`、`[cmdline] argc = 3`、forktest 回归通过。

排错速查：

| 现象 | 大概率原因 |
|---|---|
| `handle_alloc_error` panic | 内核堆耗尽（exec 还在用 Vec？或 syscall 太多）|
| `already borrowed` | 阻塞前没 drop 环形缓冲借用 |
| 读管道永远不返回 | 写端没关（Arc 循环/fd 泄漏），或 EOF 判断用了 Arc |
| `read_fd` 不是 3 | fd 表 0/1/2 没预装 stdio，或提前开了文件 |

---

## 九、信号的 AArch64 适配（几个要点）

upstream 的 ch7 信号实现**不建用户栈上的信号帧、也没有 trampoline**：
`call_user_signal_handler` 只是把 TrapContext 备份到 TCB，然后改 `sepc`（我们改 `ELR_EL1`）
和 `a0`（我们改 `x0`），要求处理函数最后调 `sigreturn()`。AArch64 上照抄即可，
但有几个坑：

1. **备份/恢复的是整个 TrapContext**（800 字节，含 x0~x30、sp_el0、ELR、SPSR，以及 q0~q31/FPCR/FPSR）。
   `sys_sigreturn` 把它整份写回，并且**返回恢复后的 x0**——因为 `trap_handler`
   在 syscall 返回后还会写一次 `cx.x[0] = ret`，不返回旧 x0 就会把返回值改坏。
2. **两条返回路径都要处理**：同步异常（`sync_handler` → `trap_handler`）和
   中断（`irq_handler` → `irq_dispatch`）。只处理前者的话，被 SIGKILL 的进程
   要等到下一次系统调用才死。
3. **用户处理函数是被 `eret` 直接跳进去的**（不是 `blr`），所以 `x30/lr` 是中断现场的
   残留值，处理函数**不能 `ret`**，必须 `sigreturn()` 换回现场。
4. `sigaction` 的用户结构体必须和内核 `SignalAction` 布局一致：
   `#[repr(C, align(16))] { handler: usize, mask: u32 }`（16 字节）。
5. `kill` 要按 pid 找进程 → 新增 `PID2TCB`（`BTreeMap`）；`add_task` 时插入，
   进程退出时 `remove_from_pid2task`。

---

## 十、没做完的（ch7 剩余 / 下一步）

1. **stdin**：接 UART 输入。**当前环境不可验证**——mini-UART 的 RX 被 root cell
   的 Linux 持有（ttyS0 上跑着 agetty），而且我们只能 ssh，没有物理串口可敲键；
   要做得先解决输入通道（关 agetty + 共享内存转发，或接 USB-TTL）；
2. **共享内存**：rCore ch7 没有 shm；在我们的环境里对应 Jailhouse ivshmem，
   需要改 root cell 配置（独立课题，和持久化一起做）；
3. **内核堆**：bump 分配器只增不减。ch8（线程）会大量创建/销毁线程，
   必须在 ch8 之前换掉（或至少把高频路径改成页帧缓冲）——这项工作在 **`ch7+` 分支**上做；
4. **用户态堆**：`pipe_large_test` 为了不用用户态堆改成了传 4 字节校验和；
   以后要给 user crate 加 `#[global_allocator]`。

> 计划：`ch7` 到此收尾（管道 + 信号 + argv + SIMD 现场，不再合并新东西）；
> 技术债（堆分配器 + ivshmem + stdin 通道）在 **`ch7+`** 分支上继续，再从 `ch7+` 演进到 ch8。
