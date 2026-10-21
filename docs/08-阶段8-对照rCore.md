# 📘 阶段 8：线程与同步 —— 对照 rCore ch8 逐项讲

> 项目：rc-aarch ｜ 工程：`os-cell3/` + `easy-fs/` + `user/`
> 状态：**✅ ch8 的线程 + 互斥锁 + 信号量 + 条件变量已在树莓派 4B 的
> openEuler MCS + Jailhouse cell3 上跑通**
> 分支：`ch7+`（从 ch7 的收尾状态继续演进；ch7 分支保持冻结）
> 相关：[[07-阶段7-对照rCore]]

---

## 一、验收标准（实测通过）

| 检查项 | 状态 | 实测证据 |
|---|---|---|
| PCB / TCB 拆分：同进程线程共享地址空间 | ✅ | 4 个线程共用一个 `memory_set`，`SHARED` 计数正确累加 |
| `thread_create(entry, arg, stack_top)` | ✅ | `[thread] 创建线程 tid=7/6/5` |
| `gettid` 返回唯一线程号 | ✅ | 主线程 `tid=2`（= pid），3 个子线程 tid 各不相同 |
| `waittid` 回收线程并拿到退出码 | ✅ | `回收 tid=7 exit_code=1/2/3` |
| 线程里访问共享静态变量 | ✅ | `COUNTER=300`（3 线程 × 100） |
| 互斥锁（blocking）| ✅ | `4 线程 x 1000 次，SHARED=4000 ✓` |
| 信号量：`down` 阻塞、`up` 放行 | ✅ | worker 先阻塞，`main 发 up` 后才 `worker 被放行 ✓` |
| 条件变量：`wait` 阻塞、`signal` 唤醒 | ✅ | worker 等条件，main `signal` 后 `READY=1` |
| 既有能力回归 | ✅ | pipe_large 382468、simd_test、sig_test、argv、forktest 全过 |
| 停机页帧 | ✅ | `页帧 7379/7946 空闲` |

---

## 二、upstream ch8 做了什么

```
$ git diff --stat origin/ch7 origin/ch8 -- os/
 os/src/task/task.rs     | 拆出 ProcessControlBlock；TCB 只保留线程私有部分
 os/src/task/manager.rs  | 就绪队列放线程；pid → PCB 表
 os/src/task/mod.rs      | exit 区分"主线程退出 = 进程退出"
 os/src/sync/*.rs        | Mutex / Semaphore / Condvar（UPSafeCell + 等待队列）
 os/src/syscall/sync.rs  | 1000..1060 一组调用
 user/src/bin/*          | threads / sync_mutex / sync_sem / sync_condvar ...
```

---

## 三、我们做了什么

### 3.1 PCB / TCB 拆分（`task/task.rs`）

```text
ProcessControlBlock            TaskControlBlock
  pid / exit_code / is_zombie    tid / kernel_stack / asid
  memory_set（线程共享）          process: Weak<PCB>
  fd_table / heap / user_sp      trap_cx_ptr（内核栈顶的用户现场）
  threads: Vec<Arc<TCB>>         task_cx / status / 信号字段
  parent / children
```

- `TCB.process` 用 **Weak**：PCB.threads 持有 TCB，TCB 再强引用 PCB 就是引用环；
- 但 PCB 的强引用必须有主：`register_process()` 在 `TaskControlBlock::new`/`fork`
  返回**之前**把 PCB 放进 pid→PCB 表（踩坑记录见第五节）；
- 线程的 tid 用 `PidHandle` 持有（主线程的 tid 就是 PCB 的 pid），否则 tid 会被复用。

### 3.2 线程系统调用

| 调用号 | 语义 |
|---|---|
| 1000 `thread_create(entry, arg, stack_top)` | 在本进程里新建线程（**我们多一个 `stack_top`**：每线程一块用户栈）|
| 1001 `gettid` | 当前线程号 |
| 1002 `waittid(tid)` | 等同进程线程退出；没退出返回 -2（用户库循环 + yield）|

### 3.3 同步原语（`sync.rs` + `syscall/sync.rs`）

| 调用号 | 语义 |
|---|---|
| 1010/1015/1020 | `mutex_create(blocking)` / `lock` / `unlock` |
| 1030/1035/1040 | `semaphore_create(n)` / `up` / `down` |
| 1050/1055/1060 | `condvar_create` / `signal` / `wait` |

实现就是"单核 + 等待队列"：拿不到资源 → 把当前线程塞进对象的等待队列 →
`block_current_and_run_next()`；释放资源 → `wakeup_task()` 唤醒队首。

### 3.4 关键新原语：`block_current_and_run_next`

ch5 的 `suspend_current_and_run_next` 是 **yield**：它会把当前线程放回就绪队列。
用它实现阻塞会出问题——等待者立刻又被调度上来，越过阻塞点继续跑。

所以 ch8 加了 **block**：从调度器摘掉、**不放回**就绪队列，只有 `wakeup_task`
才回来。pipe/Stdin 的"空 → 让出再重试"仍然用 yield（它们自己会重试），
mutex/semaphore/condvar 用 block。

---

## 四、实测输出（2026-10-22）

```
[thread] 主线程 pid=2 tid=2
[thread] 创建线程 tid=7 (arg=0) / tid=6 (arg=1) / tid=5 (arg=2)
[thread] tid=7 arg=0 干完了 ...
[thread] 回收 tid=7 exit_code=1
[thread] 回收 tid=6 exit_code=2
[thread] 回收 tid=5 exit_code=3
[thread] 3 个线程共自增 300 次 ✓
[thread] thread_test passed.

[mutex] 创建互斥锁 id=0
[mutex] 4 线程 x 1000 次，SHARED=4000 ✓
[mutex] mutex_test passed.

[sem] worker 等信号量…
[sem] main 发 up
[sem] worker 被放行 ✓
[sem] 信号量验收通过

[cond] worker 等条件变量…
[cond] main 触发 signal
[cond] worker 被唤醒 ✓（READY=1）
[cond] 条件变量验收通过
[sync] sync_test passed.

[kernel] shutdown：initproc exit_code=0；页帧 7379/7946 空闲
```

---

## 五、这一阶段踩的坑

### 坑 1：PCB 的强引用在 `new()` 返回时归零

现象：启动后第一次调度就 panic `进程已经回收了`（tid=1 的初始化进程）。

原因：TCB 里存的是 `Weak<PCB>`（为了不造成引用环），而 PCB 的强引用要等
`add_task` 才放进 pid→PCB 表——`TaskControlBlock::new` 返回时局部 `Arc` 已经
归零，PCB 先被释放了。

修法：`register_process(process)` 放在 `new()`/`fork()` **返回之前**。

### 坑 2：`suspend` ≠ `block`

现象：`semaphore_down` 明明把线程挂到等待队列了，worker 却在 main `up` 之前
就打印了"被放行"。

原因：`suspend_current_and_run_next()` 会把线程放回就绪队列（yield 语义），
于是它立刻又被调度上来，函数返回、越过阻塞点。

修法：新增 `block_current_and_run_next()`（不放回就绪队列），同步原语统一用它。
这也解释了为什么 rCore 的测试能"侥幸跑过"——它的等待队列和 yield 语义混用，
靠后面 `unlock` 再唤醒一次兜住。

### 坑 3：tid 不持有 → 所有线程同一个 tid

现象：3 个线程 `gettid()` 都返回 7。

原因：`let tid = pid_alloc().0;` 里 `PidHandle` 是临时的，语句结束就 Drop，
tid 立刻被回收；下一次分配又拿到同一个。

修法：TCB 保存 `tid_handle: Option<PidHandle>`，主线程为 None（tid 即 pid）。

### 坑 4：1MB RAM 磁盘装不下 ch8 的 app

现象：mount 时 `读越界`（`fs/block.rs`）。

原因：ch8 多了 3 个测试 app，加上原有 15 个，1MB 的 easy-fs 镜像装不下。

修法：`DISK_BLOCKS` 2048 → 4096（2MB）。页帧池有 ~8000 页，够用。

---

## 六、文件清单（ch8）

| 文件 | 作用 |
|---|---|
| `os-cell3/src/task/task.rs` | **重写**：PCB + TCB（地址空间/fd 表移到 PCB）|
| `os-cell3/src/task/manager.rs` | 就绪队列放线程；pid→PCB 表；`wakeup_task` |
| `os-cell3/src/task/processor.rs` | 切换时 activate 进程页表；token 从进程取 |
| `os-cell3/src/task/mod.rs` | 线程/进程退出、sbrk、**block_current_and_run_next** |
| `os-cell3/src/task/pid.rs` | 不变（tid 复用同一分配器）|
| `os-cell3/src/sync.rs` | **新增** Mutex / Semaphore / Condvar |
| `os-cell3/src/syscall/sync.rs` | **新**：10 个同步/线程系统调用 |
| `os-cell3/src/syscall/process.rs` | waitpid 改等子进程；kill 挂到进程的所有线程；+thread_create/gettid/waittid |
| `os-cell3/src/syscall/mod.rs` | 1000..1060 分发 |
| `os-cell3/src/syscall/fs.rs` | fd 表改从 PCB 取 |
| `os-cell3/src/fs/block.rs` | RAM 磁盘 1MB → 2MB |
| `user/src/{syscall,lib}.rs` | 线程/同步系统调用封装；`waittid` 循环重试 |
| `user/src/bin/{thread_test,mutex_test,sync_test}.rs` | **新** 验收 app |
| `user/src/bin/initproc.rs` | 追加 3 个 app |

---

## 七、怎么复现

```sh
cd ~/rc-aarch/os-cell3 && RUSTUP_TOOLCHAIN=stable cargo build --release
cd ~/rc-aarch
RELOAD=1 ROOTCELL=root@192.168.31.75 \
  SSHOPTS="-F /dev/null -o StrictHostKeyChecking=accept-new" \
  ./scripts/deploy-cell3.sh
```

期望：`thread_test passed.` / `mutex_test passed.` / `sync_test passed.`，
以及 ch1~ch7 的既有输出全部通过，最后 `页帧 ~7379/7946 空闲`。

---

## 八、没做 / 下一步

1. **多线程 exec**：目前只换当前线程的现场，同进程其它线程会用到旧地址空间。
   ch8 的测试不覆盖这个场景；真要支持得在 exec 时把其它线程杀掉。
2. **线程有独立的用户栈**：upstream 让所有线程共用一个栈顶，我们改成显式传
   `stack_top`（app 里用 `static` 数组），语义更干净但和 upstream 的 ABI 不同。
3. `condvar` 是"手动的"：`wait` 不自动释放配套的 mutex、`signal` 也不立刻交接锁
   （和 upstream 的教学版一致）。真要用，按 `lock → while(!cond) wait → unlock`
   的常规写法配合。
4. sleep/文件系统性能/网络都还没做（ch9 是文件系统优化，本环境另一条线）。
