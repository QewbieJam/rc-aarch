# 📘 阶段 7：进程间通信（管道）—— 对照 rCore ch7 逐项讲

> 项目：rc-aarch ｜ 工程：`os-cell3/` + `easy-fs/` + `user/`
> 状态：**✅ ch7 已完成并在树莓派 4B 的 openEuler MCS + Jailhouse cell3 上跑通**
> （File trait、`pipe(59)`/`dup(24)`、阻塞读写；`kill/sigaction/sigprocmask/sigreturn`；
> `exec` 带 argv；**SIMD/FP 现场跨 trap 保存**）
> 更新：2026-10-22 ｜ 分支：`ch7` → `ch7+`（`ch7+` 在此之上换堆分配器，继续往 ch8 演进）
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
| 用户 SIMD/FP 现场跨系统调用保持 | ✅ | `simd_test`：`② 向量寄存器跨系统调用保持 ✓` |
| 自动向量化的用户循环结果正确 | ✅ | `simd_test`：`③ 自动向量化填充 3000 字节全对` |
| **`exec` 带 argv** | ✅ | `[cmdline] argc = 3`、`argv[0]=cmdline_args argv[1]=hello argv[2]=argv` |

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
今天我们只做了管道；信号留到下次（它要改 trap 现场、处理函数跳转、屏蔽字，工作量不小）。

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
| `sys_exec` 带 argv（cmdline_args）| 没做，仍是 `exec(path)` |
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
[kernel] shutdown：initproc exit_code=0；页帧 7737/8027 空闲
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

### 坑 6（★ 最隐蔽）：用户 SIMD/FP 现场在系统调用后被内核改坏

**现象**：`pipe_large_test` 的校验和对不上——父进程说发出 382468，子进程收到 2952 个
字节是错的，而且**两边内容一模一样**（校验和相同，所以只比较校验和的旧测试会"通过"）。
用逐字节比对才露出真相：父进程填完 `data[]`、**还没开始写管道**，内容就已经全错——
`data[0..16]` 全是 3、`data[16..32]` 全是 115。

**原因**：AArch64 的 **EL0 和 EL1 共用同一套 SIMD/FP 寄存器**（q0~q31、FPCR、FPSR），
切特权级不会自动保存。用户程序是 rustc 编的 AArch64 代码，`aarch64-unknown-none`
默认开 `+neon`，LLVM 把 `for i in 0..3000 { data[i] = (i*7+3) as u8 }` 自动向量化成
"`movi v0.16b,#7` … `mul v5.16b, v4.16b, v0.16b`"；内核也是同一套代码，`memcpy`/
格式化输出会用 q0~q7。编译器恰好把 `movi v0..#7` 放在一次 `close()` 系统调用**之前**——
一进内核 v0 就被写坏，回用户态后循环拿错的常量算，整片数据悄悄错掉，**不崩溃、不越界**。

**为什么 upstream 不会踩**：RISC-V 的 rCore 用户程序不用浮点，upstream 的 TrapContext
只存整数寄存器。这是 AArch64 移植特有的坑（和 ch4 的 I-cache 坑同类：架构差异会多出
一类"看起来不可能"的 bug）。

**修法**：`TrapContext` 增加 `q[32] + fpcr + fpsr`（272 → **800 字节**），
`trap.S` 的 `SAVE_REGS`/`RESTORE_REGS` 各加 16 条 `stp/ldp q, q` 与 `mrs/msr fpcr/fpsr`。
代价：每次 trap 多 528 字节访存。（关掉内核 NEON 不现实：`core`/`compiler_builtins`
是预编译的。）

**验收加固**：`pipe_large_test` 现在**逐字节**比对 3000 字节内容；新增 `simd_test`：
① EL0 能执行 SIMD；② v7 跨 `getpid()` 保持；③ 3000 字节自动向量化填充全对。

> 教训：**只比较"两边各自算出的摘要"的测试，在两边被同一个 bug 写坏时会假通过**
> （父子进程共享同一份错误数据，校验和当然一样）。跨特权级的验收要直接比内容。

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
| `user/src/bin/{pipetest,pipe_large_test}.rs` | **新** 验收 app（large 版改成传 4 字节校验和，避免用户态堆）|
| `user/src/bin/initproc.rs` | 依次跑 5 个 app |

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

期望：`[pipetest] passed.`、`[pipe_large] passed.`（校验和一致）、forktest 回归通过。

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

1. ~~**`exec` 带 argv**~~ → **已完成**（见第十二节）；
2. **stdin**：**不做 UART 版**。设计已定：UART 归 root cell（符合预期），
   cell3 的输入通道走 **root 侧转发**——ssh 到 root cell，由 root 的守护进程把
   stdin 写进共享内存里的控制台队列，cell 的 `Stdin` 从队列读。
   这样 `user_shell` 能跑，而且能从当前 ssh 会话直接喂输入测试。
   这件事和 ivshmem 一起做（ch7+）；
3. **共享内存 / 持久化**：rCore ch7 没有 shm；在我们的环境里对应 Jailhouse ivshmem，
   `ch7+` 的下一个任务（同时解决 stdout 虚拟控制台与磁盘镜像）；
4. ~~**内核堆**：bump 分配器只增不减~~ → **已在 `ch7+` 分支解决**（见第十一节）；
5. **用户态堆**：`pipe_large_test` 为了不用用户态堆改成了传 4 字节校验和；
   以后要给 user crate 加 `#[global_allocator]`。

> 计划：`ch7` 到此收尾（不再合并新东西）；技术债（ivshmem + stdin 通道）在 **`ch7+`**
> 分支上继续，再从 `ch7+` 演进到 ch8（线程与同步）。

---

## 十一、ch7+：换上真正的堆分配器（并挖出一个隐蔽的 I-cache 崩溃）

分支：`ch7+`（从 ch7 拉出，专门还技术债：堆分配器 + 后续的 ivshmem）。

### 11.1 为什么必须换

bump 分配器只增不减：ch8 的线程会反复创建/销毁，ch7 的 `exec` 已经因此崩过一次
（用 `Vec` 读 ELF，扩容的旧缓冲全泄漏）。换成**按地址排序的空闲链表 + 首次适配 + 相邻合并**：

```text
堆区 [start, start+size)，每块 = | size(8B) | next 偏移(8B) | 负载 |
alloc：首次适配，够大就切一刀，剩下的留在链表
dealloc：按地址插回，和前/后相邻块合并
```

- 所有块 16 字节对齐；`align > 16` 的请求返回 null（内核里没有这种类型）；
- 单核 + 内核态关中断，用 `UnsafeCell`，不需要原子操作；
- 加了 `check()`：遍历空闲链表，检查有序、不重叠、不越界、大小合法。

实测：

```
[mm] kernel heap: [0xfa06f030, 0xfa0af030) 256 KB（空闲链表，可回收）初始: 空闲 256KB / 最大块 256KB / 1 块
[mm] heap_test: 总 256KB，测试前空闲 255KB，现在空闲 255KB（最大块 254KB，2 个空闲块）
[mm] heap_test passed: 分配/释放/合并都正常
...
[kernel]           内核堆 250/256 KB 空闲（最大块 245KB，7 个空闲块）
```

`heap_test` 会做 20 轮"50×1000B 反复分配释放" + 32×4KB 大块 + "A/B/C 释放 B 再申请"
的碎片场景，最后断言空闲内存回到了初始值。

### 11.2 换完之后出现的偶发崩溃（本章最值钱的一个 bug）

现象：大约 1/10 的运行会 panic：

```
[mm] 用户指针 0x13007 没映射/非用户页：pte=0x0 token=0xfa1fd000 task=2 elr=0x10620
[mm] 当前 TTBR0=0x20000fa1fd000（低 48 位 0xfa1fd000），TCB token=0xfa1fd000
[mm] 当前任务 areas: [0x10000,0x12000) ... [0x12000,0x13000) ...   ← 这是 cat/filetest 的布局
```

逐项排除：

- `TTBR0 == TCB token` → 页表切换是对的，不是"current 和 TTBR0 不同步"；
- `pte=0` → 0x13000 在**当前 app** 的页表里确实没映射；
- 但 `0x13007` 是 **initproc 的 rodata 偏移**，而 `elr=0x10620` 正是 **initproc**
  `Stdout::write_str` 里 `svc` 的下一条指令；
- 当前 areas 却是 cat/filetest 的（rodata 在 0x12000）——也就是说：
  **CPU 在用"新 app 的页表"执行"旧 app 的代码"。**

根因：**所有 app 都链接在同一段 VA（0x10000 起）上**，`exec` 把这段 VA 重新指向
**新的物理页**。ARMv8 明确要求：改完 VA→PA 映射后，必须让 **I-cache 失效**，
否则缓存里可能还留着旧物理页的指令。我们只做了 TLB 失效（`tlbi vmalle1`），
没做 I-cache 失效 → 偶尔就执行到了上一个 app 的代码（用新页表去取旧指令，
自然就会去访问旧 app 的 rodata → 未映射 → 内核拒绝）。

为什么以前没暴露：`fork` 出来的子进程跑的是**同一份指令**（旧缓存行内容一样，无害）；
只有 `exec` 到**另一个 app**、且那段缓存行恰好还在时才会踩到。

修复：在 `MemorySet::activate()`（每次换地址空间、包括 exec）里，TLB 之后加
`ic iallu` + `dsb ish` + `isb`。改完连续 16 次上板运行无复现。

> 这个 bug 很难查：崩溃点是"内核断言用户指针非法"，但真正错的是**取指**，
> 而崩溃现场又只能看到"新页表 + 旧 PC"。定位靠的是把
> TTBR0 / TCB token / 页表项 / 当前 areas / ELR 处的指令字**一次性全打出来**。

### 11.3 ch7+ 还没做的

- **ivshmem**（共享内存 / 持久化）：需要改 root cell 配置，和上一轮讨论的方案一致；
- 分配器目前是"首次适配 + 合并"，长时间运行仍会有外部碎片（教学够用；
  要更好可以换伙伴系统，upstream 用的就是 `buddy_system_allocator`）。

> ch7 收尾时修的 **SIMD/FP 现场**（见"坑 6"）已经同步到 ch7+；
> 800 字节的 TrapContext 压在 2 页（8KB）内核栈顶，剩余空间仍然充足。

---

## 十二、ch7 收尾：exec 的 argv

`sys_exec(path)` 只传程序名，很多程序（`user_shell`、`cmdline_args`）需要参数。
补齐成 `sys_exec(path, argv)`，和 upstream ch7 同构：

### 12.1 内核侧

1. `sys_exec` 把用户传进来的 **argv 指针数组**（NULL 结尾）逐项 `translated_str`
   拷回内核，得到一个 `Vec<String>`；
2. `TaskControlBlock::exec(elf, args)` 在**新用户栈**上压参数：
   ```text
   [高]  argv[argc] = NULL
         argv[argc-1] -> 最后一个字符串
         ...
         argv[0]     -> 第一个字符串
   [低]  "arg0\0" "arg1\0" ...        ← 字符串从高往低依次压
   ```
3. 入口处设置寄存器：**x0 = argc，x1 = argv**（RISC-V 是 a0/a1 = x[10]/x[11]）；
4. `sys_exec` 返回 argc —— 因为 `trap_handler` 之后还会写一次 `cx.x[0] = 返回值`，
   返回 argc 正好让入口的 x0 正确（upstream 的注释也是这么写的）；
5. SP 按 AAPCS64 对齐到 **16 字节**（upstream 只按 8 对齐，那是 RISC-V 的要求）。

### 12.2 用户侧

`_start(argc, argv)` 从 x0/x1 拿到参数，扫描每个字符串的 `\0` 求长度，
拼成 `&[&str]` 再调用 `main(argc, argv)`。

> 和 upstream 的差别：upstream 用 `Vec<&str>`（所以 ch7 给用户态加了 32KB 堆 +
> buddy 分配器）；我们用固定数组 `[&str; 16]`，**不需要用户态堆**。

### 12.3 两个容易踩的点

1. **argv 里每个字符串都必须以 `\0` 结尾**。kernel 用 `translated_str` 按 C 字符串读，
   Rust 的字面量 `"hello"` **不带** `\0`，直接传指针会一路读进相邻的 .rodata，
   表现为 `argv[0] = cmdline_argshelloSomeargvNone` 这种"粘在一起"的怪字符串。
   → 全部写成 `"hello\0"`（upstream 的测试 app 也是这么写的）。
2. `main` 的签名：lib.rs 里声明成 `fn main(argc, argv)`，但没用到参数的 app
   仍可以写 `fn main() -> i32` —— 多出来的 x0/x1/x2 它不读就没事（upstream 同样如此）。

### 12.4 实测

```
[initproc] 子进程 exec cmdline_args
[cmdline] argc = 3
[cmdline] argv[0] = cmdline_args
[cmdline] argv[1] = hello
[cmdline] argv[2] = argv
[cmdline] argv[0] 是程序名 ✓
```

---

## 十三、ch7+ phase 1：ivshmem 跨 cell 字节通道（打通了）

目标只有一个：**证明 guest cell3 和 root 用户态之间能收发字节**。控制台语义
（`Stdin`/`Stdout`/`user_shell`）留到 phase 2。

### 13.1 验收结果（2026-10-22）

root 侧 `ivshm-console -n 20` 做 round-trip：每发一行 `ping-NNN` 等 guest 回显。

```
[shm] 已向 root 发送 25 字节问候
[shm] root→guest: ping-000
...（guest 控制台共 20 行）
[ivshm] 丢弃: [guest] hello from cell3     ← guest 的启动问候，round-trip 模式丢弃
[ivshm] round-trip 20/20 成功
```

既有能力回归：pipetest / pipe_large(382468) / simd_test / sig_test / cmdline_args /
forktest 全过，停机 `页帧 7690/7983`。

### 13.2 内存从哪来、权限怎么分

`cells/rpi4-rust-cell3.c` 用 `JAILHOUSE_SHMEM_NET_REGIONS(0x6fffe000, 1)` 声明了和
root cell **同一批物理页**，带 `JAILHOUSE_MEM_ROOTSHARED`。hypervisor 在 `cell_create`
时不会把它们从 root cell 里 unmap（`hypervisor/control.c`）→ 两个 cell 共享。

| 地址 | 谁写 | 谁读 | 用途 |
|---|---|---|---|
| `0x6fffe000` | hypervisor | 两边 | 状态表（VETH 用；我们没碰）|
| `0x6ffff000` | **root** | guest | root 的 TX = guest 的 RX |
| `0x7007e000` | **guest** | root | guest 的 TX = root 的 RX |

关键约束：**每个 cell 只能写自己的输出段**（对方的段在 stage-2 里是只读的）。

### 13.3 段内协议（我们自己定的简单字节环，不是 virtio vring）

```text
off 0   magic   u32  'RCSH'
off 4   gen     u32  这一段被初始化的"代数"（每次启动换一个）
off 8   write   u32  生产者下标（自由回绕）
off 12  read    u32  消费者下标
off 16  cap     u32
off 20  data[cap]
```

★ **read 下标写在自己的段里**：因为消费者只能写自己的段。"我读到哪儿"必须让生产者
能看到，所以它放在消费者自己的段里；生产者算 free 时去读**对方段里的 read**。
（VETH 里 `rx.vr.used` 和 `tx.vr.used` 的 swap 就是为了这个。）

### 13.4 root 侧怎么访问：uio_ivshmem

- `uio_ivshmem` 只匹配 PCI class `(0x00, 0x4001 RPMSG)`；我们的设备是 VETH（class
  `0x0001`），自动绑定会 `ENODEV`。用 **`new_id` 动态加一条 ID** 就能绑：
  `echo -n "110a 4106" > /sys/bus/pci/drivers/uio_ivshmem/new_id`
- 一次只能绑一个驱动：先 `echo -n 0001:00:00.0 > .../ivshmem-net/unbind`；
- ivshmem PCI 设备**只有在 `jailhouse enable` 之后才应答 config cycle**，所以顺序必须是
  enable → 切 uio → 起守护进程 → `cell create`（首次）/`RELOAD`。
- 封装成了 `scripts/setup-ivshm-uio.sh`（幂等）。

uio 给的 map（root 视角，dev_id=0）：

| map | 内容 | 权限 |
|---|---|---|
| map0 | `0xffa00000` 4KB 寄存器 | RW |
| map1 | `0x6fffe000` 状态表 | RO |
| map2 | `0x6ffff000` 起两段（peer0/peer1 输出）| RO |
| map3 | `0x6ffff000` 一段（root 自己的输出）| RW |

### 13.5 这一阶段踩的坑

1. **用户页表只共享内核的 L1[3]**（0xC0000000~0xFFFFFFFF），而共享内存在
   `0x40000000~0x7fffffff` → L1[1]。内核在 boot 阶段写共享内存没事（TTBR0=内核页表），
   但任务跑起来后（TTBR0=用户页表）在定时器中断里读共享内存就 **translation fault
   level 1**。修法：`from_elf`/`from_existed_user` 里除了 `KERNEL_L1_INDEX` 再
   `share_l1_entry(SHMEM_L1_INDEX)`。
2. **read 下标放错段**：写进对方段会得到 stage-1 permission fault（我们是只读）。
   改成"read 写自己段、free 读对方段的 read"后正常。
3. **共享内存不清零**：`magic` 可能是上次运行留下的，守护进程会读到上一轮的残留数据
   （表现为同一句问候打印两遍）。加 `gen`（guest 用 `CNTVCT_EL0`、root 用
   `clock_gettime`）+ 启动时"把 read 推到 write"丢弃旧数据。
4. **RELOAD 不会重新 create cell**：改了 cell 配置（加共享内存段）之后必须让 cell
   **重新 create**（重启板子后第一次用完整流程），否则 stage-2 里根本没有这些页，
   首次访问就是 `Unhandled data write`。

### 13.6 文件清单（phase 1）

| 文件 | 作用 |
|---|---|
| `cells/rpi4-rust-cell3.c` | +`JAILHOUSE_SHMEM_NET_REGIONS(0x6fffe000, 1)`（4 段）|
| `os-cell3/src/ivshmem.rs` | **新**：共享内存字节环（send/recv/init/IRQ 轮询）|
| `os-cell3/src/mm/memory_set.rs` | 映射两段共享内存；用户页表挂 L1[1] |
| `os-cell3/src/mm/mod.rs` | +`SHMEM_L1_INDEX` |
| `os-cell3/src/main.rs` | 启动时 init + 发问候 |
| `os-cell3/src/trap/mod.rs` | 定时器 IRQ 里 `poll_rx_to_console()`（phase 1 的临时消费者）|
| `tools/ivshm-console.c` | root 侧 uio 守护进程：收发 + round-trip 自检 |
| `scripts/setup-ivshm-uio.sh` | **新**：把 ivshmem 设备从 net 驱动切到 uio |

### 13.7 怎么复现

```sh
# 0) 板子重启后第一次：完整流程（会 create cell）
cd ~/rc-aarch && ROOTCELL=root@192.168.31.75 scripts/deploy-cell3.sh

# 1) 切 uio（jailhouse 已使能之后）
ROOTCELL=root@192.168.31.75 ./scripts/setup-ivshm-uio.sh

# 2) 编译守护进程并上传
aarch64-linux-gnu-gcc -O2 -static -o /tmp/ivshm-console tools/ivshm-console.c
scp /tmp/ivshm-console root@192.168.31.75:/root/ivshm-console

# 3) 起守护进程（等 guest 新的 gen 后发 20 轮 ping），再 RELOAD 部署
setsid nohup /root/ivshm-console -s x -n 20 -d 60 > /root/ivshm.out 2> /root/ivshm.err &
RELOAD=1 ROOTCELL=root@192.168.31.75 scripts/deploy-cell3.sh
```

### 13.8 下一步（phase 2）

- guest：把 `poll_rx_to_console` 换成真正的 `Stdin`（读共享内存环、空了就
  `suspend_current_and_run_next`），`Stdout` 写自己的 TX 段；
- root：守护进程接 ssh 的 stdin/stdout（而不是 `-n` 自检）；
- 用户态：`user_shell`（读一行、`exec` 命令），正好用上 ch7 的 argv 和 pipe。

> 提示：目前 root 侧的 `ivshm0` 网络（ivshmem-net）被我们解绑了；如果之后还要用它，
> 记得 `echo 0001:00:00.0 > .../ivshmem-net/bind` 绑回去（两个驱动只能二选一）。

---

## 十四、ch7+ phase 2：把 stdin/stdout 接到共享内存上，跑起 `user_shell`

phase 1 证明"通道能收发字节"；phase 2 把它接到 `File` 层，于是 ch7 的
`Stdin`/`Stdout` 和 ch5 的 `fork/exec/waitpid` 第一次真正串起来跑了一个交互 shell。

### 14.1 验收结果（2026-10-22）

root 侧守护进程把一段脚本行喂进 guest：

```
>> 可用命令（easy-fs 根目录里的 app）：
  cat_filea        读 hello.txt
  ...
>> [cmdline] argc = 3
[cmdline] argv[1] = a
[cmdline] argv[2] = b
>> [cat] hello.txt 共 32 字节：
hello from easy-fs on rc-aarch!
[cat] 结束
>> [shell] bye
[initproc] 所有子进程回收完毕，退出
[kernel] shutdown：initproc exit_code=0；页帧 7682/7975 空闲
```

既有 app 全过（pipe_large 382468、simd_test、sig_test、argv、forktest），
最后 `user_shell` 读 stdin 阻塞等命令，`exit` 后整条链（shell → initproc → 停机）正常收尾。

### 14.2 内核侧改了什么

| 文件 | 改动 |
|---|---|
| `fs/stdio.rs` | `Stdin::read` 从 `ivshmem::recv` 读；没数据就 `suspend_current_and_run_next()`，和 ch7 的 pipe 读端同一套阻塞模型。`Stdout::write` **同时**写 UART 和共享内存 TX 段（tee：串口验收和控制台会话两不误）|
| `ivshmem.rs` | phase 1 的 IRQ 临时消费者删掉，RX 由 `Stdin` 真正消费 |
| `trap/mod.rs` | tick 日志只打第 1 次（长跑 cell 里每 100 次刷屏会让部署脚本永远等不到 idle）|

### 14.3 用户侧：`user/src/bin/user_shell.rs`

```
>> 提示符 → read(0) 按字节读一行 → 按空格切词（原地写 '\0'）→
   fork → 子进程 exec(命令, argv) → 父进程 waitpid → 下一轮
```
- `help` 列出 easy-fs 里的 app；`exit` 退出；
- 没有堆：行缓冲 `[u8; 257]`、参数 `[&str; 8]` 都是固定大小；
- 切词分两遍（先原地改 `line` 记区间、再借 `&str`），否则借用冲突（E0506）。

### 14.4 root 侧：`ivshm-console -i`

交互模式：`poll(stdin)` → 有输入就 `tx_write` 进 guest 的 RX；同时把 guest TX 读出来写
stdout。这样 `ssh -tt root@板子 /root/ivshm-console -i -d 60` 就是一个真终端；
非交互测试直接 `printf "...\nexit\n" | ivshm-console -i -d 60`。

### 14.5 这一阶段踩的坑

**`Stdin` 读多了会丢数据**：最初实现是"先往 128 字节临时缓冲收，再拷给用户缓冲"。
shell 的 `read_line` 一次只读 1 字节，于是内核一次把环里 27 字节全吃进临时缓冲、
只拷 1 字节给用户，剩下 26 字节**直接丢了**（表现：shell 收到半行后永远阻塞）。
修法：临时缓冲只收到 `min(用户缓冲长度, 128)` 为止，一个字节都不多拿。

> 教训：`read(fd, buf, len)` 的语义是"最多写 len 字节"，内核内部做缓冲时
> **不能先多读再截断**——多读的数据对用户不可见，等于丢失。

### 14.6 文件清单（phase 2）

| 文件 | 作用 |
|---|---|
| `os-cell3/src/fs/stdio.rs` | Stdin 接 ivshmem（阻塞读）；Stdout tee 到 UART + ivshmem |
| `os-cell3/src/ivshmem.rs` | 去掉 phase 1 的 IRQ 消费者 |
| `os-cell3/src/trap/mod.rs` | tick 日志静音 |
| `user/src/bin/user_shell.rs` | **新**：交互 shell（提示符/切词/fork/exec/waitpid）|
| `user/src/bin/initproc.rs` | app 列表最后加 `user_shell` |
| `tools/ivshm-console.c` | `-i` 交互模式（stdin → guest，guest 输出 → stdout）|

### 14.7 怎么复现

```sh
cd ~/rc-aarch
# 1) 起 root 侧守护进程，喂一串命令
ROOTCELL=root@192.168.31.75 ./scripts/setup-ivshm-uio.sh
aarch64-linux-gnu-gcc -O2 -static -o /tmp/ivshm-console tools/ivshm-console.c
scp /tmp/ivshm-console root@192.168.31.75:/root/ivshm-console
ssh root@192.168.31.75 'chmod +x /root/ivshm-console; \
  setsid nohup sh -c "printf \"help\ncmdline_args a b\ncat_filea\nexit\n\" | \
  /root/ivshm-console -i -d 60" > /root/shell.out 2>/root/shell.err < /dev/null &'
# 2) 部署（RELOAD），3 秒后看 /root/shell.out
RELOAD=1 ROOTCELL=root@192.168.31.75 ./scripts/deploy-cell3.sh
```

### 14.8 下一步（ch8 前的地基已经齐了）

- `ch7+` 到此：堆分配器、共享内存通道、stdin/stdout、user_shell 都有了；
- ch8 要的是**线程 + 同步原语**（mutex/condvar/semaphore）：分配器已经支持反复
  创建/销毁，`suspend_current_and_run_next` 的阻塞模型也已经验证过，可以直接往上加
  `wait/wake` 队列；
- 顺带可做：panic/退出时把控制权交回 shell（现在 `exit` 是 shell 的最后一个命令）。
