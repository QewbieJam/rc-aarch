# 📘 阶段 7：进程间通信（管道）—— 对照 rCore ch7 逐项讲

> 项目：rc-aarch ｜ 工程：`os-cell3/` + `easy-fs/` + `user/`
> 状态：**✅ ch7 的管道部分已在树莓派 4B 的 openEuler MCS + Jailhouse cell3 上跑通**
> （File trait 重构、`pipe(59)`/`dup(24)`、32 字节环形缓冲、阻塞读写、父子进程传 3000 字节）
> 更新：2026-10-21 ｜ 分支：`ch7`（从 ch6 的 `cd2563a` 分出）
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
| 信号（kill/sigaction/sigprocmask/sigreturn）| **没做**（下一节）|

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

## 九、没做完的（ch7 剩余 / 下一步）

1. **信号**：`kill(129)/sigaction(134)/sigprocmask(135)/sigreturn(139)`，
   包括 trap 返回前检查 pending 信号、改用户现场跳到处理函数、`sigsuspend`。
   这是 ch7 的另一半，工作量约等于今天的管道；
2. **`exec` 带 argv**（cmdline_args）；
3. **stdin**：接 UART 输入（需要轮询或中断），之后 user_shell 才能跑；
4. **共享内存**：rCore ch7 没有 shm；在我们的环境里对应 Jailhouse ivshmem
   （之前讨论过，作为独立课题，和持久化一起做）；
5. **内核堆**：换掉 bump 分配器（或把 `translated_byte_buffer` 改成栈上小数组），
   彻底消除长跑泄漏。
