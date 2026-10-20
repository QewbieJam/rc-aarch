# 📘 阶段 5：进程（fork / exec / waitpid）—— 对照 rCore ch5 逐项讲

> 项目：rc-aarch ｜ 工程：`os-cell3/`（MCS cell 版）+ 仓库根 `user/`
> 状态：**✅ ch5 已在树莓派 4B 的 openEuler MCS + Jailhouse cell3 上跑通（initproc → fork/exec → forktest 5 个孩子 → 全部 waitpid 回收）**
> 更新：2026-10-21 ｜ 分支：`ch5`（从 ch4 的 `273f315` 分出）
> 相关：[[04-阶段4-对照rCore]] ｜ [[03-阶段3-对照rCore]] ｜ [[02-阶段2-对照rCore]]

---

## 一、验收标准（全部实测通过）

| 检查项 | 状态 | 实测证据 |
|---|---|---|
| `getpid` | ✅ | `id=172 返回 1`（initproc 的 pid） |
| `fork`：父进程拿到子 pid、子进程返回 0 | ✅ | `id=220 返回 2`；forktest 里 5 个孩子各自打印自己的 pid |
| `exec`：换地址空间、从新 ELF 入口跑 | ✅ | `exec forktest` / `exec 03sbrk` 都成功（`id=221 返回 0`） |
| `waitpid`：僵尸回收 + 退出码写回用户指针 | ✅ | `回收 pid=4，exit_code=1` … `5 个孩子全部回收，exit_code 之和 = 10` |
| 没有子进程 / 子进程还活着 | ✅ | `-1` / `-2` 两种返回都实测到 |
| pid 复用 | ✅ | 第二轮 exec 的子进程又拿到 `pid=2`（PidHandle Drop 回收） |
| 进程退出 → Zombie → 父进程回收 | ✅ | 回收后 TCB/页表/页帧/内核栈/pid 全部释放 |
| **资源回收**（ch4 的 TODO） | ✅ | 停机时 `页帧 8033/8066 空闲`（开机 8066 页，跑到结束只多占 33 页 = initproc 自己） |
| 板子不挂、可反复 RELOAD | ✅ | `cell list` 全程 `running`、`Failed CPUs` 为空 |

---

## 二、rCore ch5 做了什么

```
$ git diff --stat origin/ch4 origin/ch5 -- os/
 os/src/task/pid.rs        | 92 +++     pid 分配器 + 内核栈（固定 VA 槽位）
 os/src/task/manager.rs    | 40 +++     就绪队列（VecDeque）
 os/src/task/processor.rs  | 90 +++     Processor + run_tasks + schedule
 os/src/task/task.rs       | 180 ++++   Arc<TCB> + inner、fork/exec、父子关系、Zombie
 os/src/task/mod.rs        | 100 ++++   suspend/exit(exit_code)、initproc、reparent
 os/src/syscall/process.rs | 90 +++     getpid/fork/exec/waitpid
 os/src/trap/mod.rs        | 46 +++     异常退出码 -2/-3、current_trap_cx 重新取
 user/                     | 一堆新测试 app（forktest/forktree/exit/user_shell…）
```

一句话：**ch4 的"任务"是"一套页表 + 一个 ELF"；ch5 把它变成有身份（pid）、有亲缘（父子）、
有生死（Zombie/退出码）的"进程"，调度器也从"扫数组"改成"idle 循环 + 就绪队列"。**

---

## 三、调度模型的变化（本章最核心）

```
ch3/ch4：run_first_task() → 任务里一直跑
         时间片到了：__switch(当前任务, 下一个 Ready 任务)      ← 任务之间直接换
         数组里线性扫下一个 Ready

ch5：    run_tasks() 循环：
             fetch_task() 从就绪队列取一个
             __switch(idle, 任务)          ← 从"调度器"进任务
         任务里 yield / 被抢占 / 退出：
             __switch(任务, idle)          ← 回到调度器，继续 fetch
```

带来的三个新东西：

1. **`Processor`**：记录"当前谁在跑" + 调度器自己的上下文 `idle_task_cx`；
2. **`TaskManager`（就绪队列）**：`VecDeque<Arc<TaskControlBlock>>`，FIFO；
3. **`Zombie`**：进程退出不立刻消失，TCB 挂在父进程 `children` 里，等 `waitpid` 收尸。

---

## 四、逐项对照

### 4.1 pid + 内核栈（`task/pid.rs`）

| rCore ch5 | rc-aarch |
|---|---|
| `PidAllocator`（current + recycled） | **完全一致** |
| `PidHandle` Drop 时回收 pid | **完全一致**（实测 pid 2 被复用） |
| `KernelStack::new(&pid)`：按 pid 在 `TRAMPOLINE - pid*8KB` 处插一段内核空间映射 | **改成从页帧池连续分配 2 页**（8KB），`get_top() = 起始物理地址 + 8KB` |
| `KernelStack::Drop`：`remove_area_with_start_vpn` 删映射 | `frame_dealloc_range(2)` 还页帧 |

为什么能这样：AArch64 的内核恒等映射一直有效，进程的内核栈只需要"一块可写内存"，
不需要固定 VA（没有 trampoline）。好处是内核栈天然可回收（ch4 结尾列过这个 TODO）。

连续页怎么保证：`StackFrameAllocator::alloc_contig(n)` **只从还没发出去的区间往上切**，
不拼 recycled；释放后进 recycled，以后给单页分配复用。

### 4.2 TCB（`task/task.rs`）

| rCore ch5 | rc-aarch |
|---|---|
| `Arc<TaskControlBlock>` + `UPSafeCell<Inner>` | 同 |
| `pid: PidHandle` / `kernel_stack: KernelStack` | 同 |
| `trap_cx_ppn: PhysPageNum`（TrapContext 在用户空间固定页） | **`trap_cx_ptr: usize`**（TrapContext 在内核栈顶） |
| `parent: Option<Weak<TCB>>` / `children: Vec<Arc<TCB>>` | 同 |
| `exit_code` / `task_status ∈ {Ready, Running, Zombie}` | 同 |

因为 TrapContext 就在内核栈顶：

- `fork` 直接 `*child_cx = *parent_cx`（`TrapContext: Copy`），再 `child.x[0] = 0`；
- `exec` 直接覆盖成新的 `app_init_context`；
- 不需要 `TRAP_CONTEXT` 常量、不需要页表换算。

### 4.3 `fork` / `exec` / `waitpid`

| 步骤 | rCore ch5 | rc-aarch |
|---|---|---|
| fork 地址空间 | `MemorySet::from_existed_user`（逐页深拷贝） | **同**（另外逐页清 D-cache，因为子进程要执行这些页） |
| fork 内核栈 | 新 pid → 新内核栈 | 同（页帧连续 2 页） |
| fork 返回值 | 子进程 `x[10]=0`，父进程由 trap_handler 写 `x[10]=pid` | **同**（`x[0]`） |
| exec | 换 memory_set + 重置 trap_cx（`kernel_sp` 也要改） | 换 memory_set + 重置 trap_cx + **立刻 `activate(TTBR0)`** |
| waitpid | 找僵尸孩子 → 摘掉 → 读 exit_code → 写用户指针 | **同**（`translated_refmut`） |

★ `exec` 必须**当场**切 TTBR0：exec 之后不会经过调度器，而是直接 `eret` 回用户态，
不切页表就会跑回旧地址空间。

### 4.4 退出与孤儿

| 场景 | rCore ch5 | rc-aarch |
|---|---|---|
| 进程退出 | 置 Zombie、记 exit_code、**子进程托付给 initproc**、回收用户页 | **同** |
| 异常退出码 | 缺页 -2、非法指令 -3 | **同** |
| initproc 退出 | 系统停机 | 同（`IDLE_PID = 1`） |
| Zombie 的资源 | 用户页立刻回收，页表随 TCB 一起释放 | 同（`recycle_data_pages` + Drop） |

---

## 五、实测输出（2026-10-21，hvc 虚拟控制台）

```
[mm] stage-1 MMU 已打开（恒等映射，TTBR0）
[mm] remap_test passed: .text RX / .data RW&XN
[rc-aarch] stage 5: 进程（fork/exec/waitpid）on Jailhouse cell3
[kernel] apps: 00hello_world 01yield 02busy 03sbrk forktest initproc
[timer] 定时器已武装；IRQ 在内核态保持屏蔽，eret 到 EL0 后自动放行
[irq] 第 1 次 tick（来自 EL0）
[initproc] pid=1 启动
[initproc] 子进程 exec forktest
[forktest] 父进程 pid=2 开始 fork
[forktest] fork 出孩子 pid=3（第 0 个）
[forktest] 我是第 0 个孩子，pid=3
...
[forktest] 回收 pid=4，exit_code=1
[forktest] 回收 pid=5，exit_code=2
[forktest] 回收 pid=6，exit_code=3
[forktest] 回收 pid=7，exit_code=4
[forktest] 5 个孩子全部回收，exit_code 之和 = 10
[forktest] pass.
[initproc] forktest pid=2 结束，exit_code=0（waitpid 返回 2）
[initproc] 子进程 exec 03sbrk
[app3] sbrk_test: 初始 brk = 0x17000
[app3] sbrk(4096) -> 0x17000
[app3] 新页可读可写 ✓
[app3] sbrk(8192) -> 0x18000
[app3] 跨页读写 OK ✓
[app3] sbrk(-0x100000) = -1（-1 表示失败，符合预期）
[initproc] 03sbrk pid=2 结束，exit_code=0（waitpid 返回 2）
[initproc] 已经没有子进程了，waitpid 返回 -1
[initproc] 所有子进程回收完毕，退出
[kernel] initproc 退出，exit_code=0，系统停机
[kernel] shutdown：initproc exit_code=0；页帧 8033/8066 空闲（回收正常的话应该接近开机值）
```

三个看点：

1. **输出是交错的**（`[forktest] fork 出孩子 pid=[forktest] 我是第 0 个孩子…`）——
   多个进程真的在并发跑，而且 `println!` 一条要拆成好几次 `write`，中间会被抢占；
2. **pid 2 被复用了**：forktest 退出后，第二个 exec 出来的子进程又拿到 pid 2；
3. **页帧几乎全回来了**：开机 8066 页，跑完 fork/exec/5 个孩子/2 次回收之后还剩 8033 页，
   多出来的 33 页是 initproc 自己（它退出时是直接停机，没走回收）。

---

## 六、本章的坑 / 设计要点

### 坑 1：fork 时 TrapContext 必须在父进程写回返回值**之前**复制

`sys_fork` 的执行顺序是：

```
current.fork()            ← 这里复制父进程的 TrapContext 给子进程
   ...
返回 new_pid
trap_handler: cx.x[0] = new_pid    ← 父进程的返回值
```

如果在 trap_handler 写完 `x[0]` 之后再复制，子进程就会错误地拿到 `new_pid` 而不是 0。
所以 fork 里的 `*child_cx = *parent_cx` 必须发生在 syscall 返回之前（我们放在 TCB::fork 里）。

### 坑 2：exec 之后必须**立刻**切 TTBR0

exec 只是换了 TCB 里的 `memory_set`，而 CPU 的 `TTBR0` 还指着旧的页表。
如果等调度器切换，exec 的 `eret` 会先回用户态 → 立刻缺页。
→ `TaskControlBlock::exec()` 里最后一句就是 `inner.memory_set.activate(self.asid)`。

### 坑 3：fork 出来的代码页也要同步 I-cache

fork 是**逐页深拷贝**：子进程的 .text 在**新的物理页**里（通过恒等映射写进去的）。
I-cache 里可能还是零/旧内容 → 子进程一执行就 `EC=0x00`（和 ch4 的坑 2 一模一样）。
→ `from_existed_user` 里每个拷贝页都 `clean_dcache_range_to_pou`，fork 结尾再 `sync_icache_all()`。

### 坑 4：内核栈要**物理连续**，所以连续分配不能拼 recycled

`frame_alloc_contig(n)` 只从"还没发出去的区间"往上切。如果允许从 recycled 里拼，
拿到的两页可能物理不相邻，栈就裂了。代价可接受：连续分配的页归还后进 recycled，
以后被单页分配复用。

### 坑 5：`waitpid` 的两个返回码别搞混

- `-1`：**没有这个子进程**（连 pid 都对不上）
- `-2`：**有这个子进程，但还活着**

用户库的 `wait()/waitpid()` 就是靠 `-2` 来判断"要不要 yield 一下再试"，写成别的值会死循环。

### 坑 6：用户态 `exec` 的路径必须是 C 字符串

内核 `translated_str` 读到 `\0` 才停，所以用户侧要写成 `exec("forktest\0")`。
只写 `"forktest"` 会一直往后读 .rodata，直到撞到某个 0 字节 —— 结果是执行了错误的 app 或返回 -1。

### 坑 7（沿用）：`Zombie` 的页帧分两批还

退出时 `recycle_data_pages()` 只清用户页；页表本身（L0~L3 那些帧）要等 `waitpid`
把 TCB 的最后一个 `Arc` drop 掉才释放。这样父进程在收尸前还能通过页表读到退出码等元信息。

---

## 七、文件清单（ch5 新增/改动）

| 文件 | 作用 |
|---|---|
| `os-cell3/src/task/pid.rs` | **新** pid 分配器 + 页帧内核栈（可回收） |
| `os-cell3/src/task/manager.rs` | **新** 就绪队列 |
| `os-cell3/src/task/processor.rs` | **新** Processor + run_tasks + schedule |
| `os-cell3/src/task/task.rs` | 重写：`Arc<TCB>` + inner、fork/exec、父子、Zombie |
| `os-cell3/src/task/mod.rs` | 重写：suspend/exit(exit_code)、initproc、孤儿托付 |
| `os-cell3/src/mm/memory_set.rs` | 加 `from_existed_user` / `recycle_data_pages` |
| `os-cell3/src/mm/page_table.rs` | 加 `translated_str` / `translated_refmut` |
| `os-cell3/src/mm/frame_allocator.rs` | 加 `alloc_contig` / `frame_dealloc_range` / `frame_stats` |
| `os-cell3/src/loader.rs` | 加 `_app_names` 解析 + `get_app_data_by_name` + `list_apps` |
| `os-cell3/build.rs` | 生成 `_app_names` 表 |
| `os-cell3/src/syscall/process.rs` | getpid/fork/exec/waitpid（+ 保留 sbrk） |
| `os-cell3/src/syscall/mod.rs` | 172 / 220 / 221 / 260 |
| `os-cell3/src/trap/mod.rs` | 异常退出码 -2/-3、`shutdown_with`、页帧统计 |
| `os-cell3/src/main.rs` | add_initproc → run_tasks |
| `user/src/syscall.rs` | 新增 getpid/fork/exec/waitpid，`sys_exit -> !` |
| `user/src/lib.rs` | `fork/exec/getpid/wait/waitpid`，`exit -> !` |
| `user/src/bin/{initproc,forktest}.rs` | **新** 验收 app |

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

期望：`apps: ... forktest initproc` → `[initproc] pid=1 启动` → forktest 的 5 个孩子全部回收
（`exit_code 之和 = 10`）→ 03sbrk 跑完 → `waitpid 返回 -1` → `initproc 退出` → `shutdown` +
`页帧 8033/8066 空闲`。

排错速查：

| 现象 | 大概率原因 |
|---|---|
| fork 出来的子进程直接跑飞/缺页 | 子进程 cx 复制顺序错了，或 fork 后没同步 I-cache |
| exec 之后立刻缺页 | 没在 exec 里立刻 `activate(TTBR0)` |
| `waitpid` 死循环 | 返回码不是 -1/-2 |
| 子进程执行到一半 `EC=0x00` | fork 拷贝的代码页没清 D-cache / 没 ic iallu |
| 页帧数量一路涨 | TCB/页表/内核栈没释放（检查 Arc 计数和 `recycle_data_pages`）|

---

## 九、阶段 6 预告（ch6：文件系统）

ch5 的进程已经能 fork/exec/wait，ch6 要给它加"文件"：

1. **easy-fs** 风格的文件系统（块设备 + inode/目录 + 磁盘布局）；
2. `open/close/read/write` 系统调用，把 `fd` 表放进 TCB；
3. `sys_exec` 从文件系统里读 ELF（不再靠内嵌 `_app_names`）；
4. 进程的 cwd / 文件描述符继承（fork 复制、exec 保留除 CLOEXEC 外的）；
5. 顺带可以做：`sys_read` 接上 UART 输入（现在只有输出），给 user_shell 用。
