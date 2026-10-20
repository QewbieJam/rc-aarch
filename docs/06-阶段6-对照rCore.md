# 📘 阶段 6：文件系统（easy-fs）—— 对照 rCore ch6 逐项讲

> 项目：rc-aarch ｜ 工程：`os-cell3/`（MCS cell 版）+ 仓库根 `easy-fs/`（新 crate）+ `user/`
> 状态：**✅ ch6 核心已在树莓派 4B 的 openEuler MCS + Jailhouse cell3 上跑通**
> （easy-fs 格式化/挂载、open/read/write/close、`exec` 从文件系统读 ELF、文件创建/截断）
> 更新：2026-10-21 ｜ 分支：`ch6`（从 ch5 的 `bd45776` 分出）
> 相关：[[05-阶段5-对照rCore]] ｜ [[04-阶段4-对照rCore]] ｜ [[03-阶段3-对照rCore]]

---

## 一、验收标准（实测通过）

| 检查项 | 状态 | 实测证据 |
|---|---|---|
| easy-fs 独立 crate（no_std、零外部依赖）| ✅ | `easy-fs/` 用 `alloc`，没有 `spin`/`lazy_static` |
| RAM 磁盘块设备 | ✅ | `[fs] RAM 磁盘: 0xfa0a6000 起，2048 块 × 512B = 1024 KB` |
| 格式化（超级块/位图/inode 区/数据区）| ✅ | 启动时 `EasyFileSystem::create` 成功 |
| 写入文件（分配 inode + 数据块 + 目录项）| ✅ | `[fs] 写入 /03sbrk（25208 字节）` ×8 + hello.txt |
| 重新 mount（读回超级块验证落盘）| ✅ | `[fs] 挂载成功，根目录: [...]` |
| `open/close/read/write` 系统调用 | ✅ | `open(filea, CREATE|WRONLY) = 3`、`写入 42 字节`、`读回 42 字节` |
| 截断（CREATE 对已存在文件 clear）| ✅ | `截断后读回 0 字节` |
| `exec` 从**文件系统**读 ELF | ✅ | initproc 依次 exec `filetest_simple`/`cat_filea`/`forktest` 全部成功 |
| 进程/调度没被破坏 | ✅ | forktest 5 个孩子照旧全部回收 |

---

## 二、rCore ch6 做了什么

```
$ git diff --stat origin/ch5 origin/ch6
 easy-fs/src/{bitmap,block_cache,block_dev,efs,layout,vfs,lib}.rs   ← 新 crate（~1000 行）
 easy-fs-fuse/                                        ← 宿主机上"把目录打成磁盘镜像"的工具
 os/src/drivers/block/{mod,virtio_blk}.rs             ← QEMU 的 virtio-blk 驱动
 os/src/fs/{mod,inode,stdio}.rs                       ← File trait / OSInode / 标准输入输出
 os/src/syscall/fs.rs                                 ← open/close/read/write 按 fd 分发
 os/src/task/task.rs                                  ← TCB 加 fd_table
 user/src/bin/{cat_filea,filetest_simple,huge_write}  ← 验收 app
```

三件事：**磁盘上的格式（easy-fs）**、**内核里的文件对象（fd 表 + OSInode）**、
**系统调用（open/read/write/close）**。`exec` 也从"读内嵌 ELF"变成"从文件系统读 ELF"。

---

## 三、我们的适配：没有块设备，那就用 RAM 磁盘

板子上、Jailhouse cell 里没有 virtio-blk，也还没有 SD/ivshmem 通道。所以：

```
启动时：
  ① 从页帧池要一块**连续的 1MB**（256 页）当磁盘
  ② 用 easy-fs 格式化（清空 → 写超级块 → 建根目录 inode）
  ③ 把内核里内嵌的 app ELF **写成文件**（ch5 的 _app_names 正好提供名字）
     再写一个 hello.txt
  ④ 用 EasyFileSystem::open 重新挂载（这一步顺便验证超级块/位图真的写对了）
运行期：
  ⑤ sys_exec 从文件系统读 ELF；sys_open/read/write/close 操作文件
```

★ 为什么这样设计：
- easy-fs 只依赖 `BlockDevice` trait，**换底层不用动文件系统代码**（upstream 换 virtio，我们换 RAM 盘）；
- 不需要宿主机工具（`easy-fs-fuse`）和额外的镜像文件——内嵌的 app 就是"造盘的源料"；
- 重新 open 一次是免费的**自检**：如果超级块/位图写错，挂载就会失败。

磁盘布局（和 upstream 完全一致）：

```text
 block 0                 SuperBlock
 block 1                 inode 位图（4096 个 inode 名额）
 block 2 .. 1025         inode 区（每块 4 个 128B 的 DiskInode）
 block 1026              data 位图
 block 1027 .. 2047      data 区（1021 块 ≈ 510KB）
```

> 注意：1 块 inode 位图 = 4096 个 inode，而 inode 区就要 4096×128/512 = 1024 块。
> 所以盘不能太小 —— 我们取 2048 块（1MB），正好够。

---

## 四、逐项对照

### 4.1 easy-fs crate

| rCore ch6 | rc-aarch `easy-fs/` |
|---|---|
| `block_dev.rs`：`BlockDevice` trait | **同**（`read_block` / `write_block`）|
| `layout.rs`：SuperBlock / DiskInode / DirEntry | **同**（DiskInode 128B、DirEntry 32B，`const` 断言）|
| `bitmap.rs`：inode/data 位图 | **同**（`trailing_ones` 找最低空位）|
| `efs.rs`：create/open/root_inode/alloc_* | **同** |
| `vfs.rs`：Inode 的 find/create/ls/read_at/write_at/clear | **同** |
| `block_cache.rs`：块缓存（减少 virtio 往返）| **去掉了**：我们的盘在内存里，直接读写；少一层缓存也少一堆锁 |
| indirect2（三级索引，>78KB 文件）| **没实现**：单文件上限 156 块 ≈ 78KB，超了 panic（app 最大 25KB）|
| `spin::Mutex` + `lazy_static` | 换成 crate 内的 `Mutex`（RefCell + unsafe Sync，单核够用）|

### 4.2 内核侧

| rCore ch6 | rc-aarch |
|---|---|
| `drivers/block/virtio_blk.rs` | `os-cell3/src/fs.rs` 的 `RamDisk`（页帧池连续内存）|
| `fs/mod.rs` 的 `File` trait + `UserBuffer` | 简化成 `OSInode` + `translated_byte_buffer` 的切片 |
| `fs/stdio.rs`：Stdin/Stdout 实现 File | fd 1 在 `sys_write` 里直接特判打印；fd 0 暂返回 -1 |
| `fs/inode.rs`：`OSInode` / `OpenFlags` / `open_file` / `ROOT_INODE` | **同**（`ROOT_INODE` 用我们的 `UPSafeCell<Option<...>>`）|
| TCB 加 `fd_table` + `alloc_fd` | **同**（0/1/2 保留，从 3 开始）|
| fork 继承 fd_table | **同**（`Vec<Option<Arc<OSInode>>>` clone，共享同一个打开文件）|
| `sys_open(56)/close(57)/read(63)/write(64)` | **同** |
| `sys_exec` 从 FS 读 ELF | **同**（`fs::read_file_by_name` + `read_all`）|

---

## 五、实测输出（2026-10-21，hvc 虚拟控制台）

```
[fs] RAM 磁盘: 0xfa0a6000 起，2048 块 × 512B = 1024 KB
[fs] 写入 /00hello_world（18024 字节）
[fs] 写入 /01yield（18112 字节）
[fs] 写入 /02busy（18112 字节）
[fs] 写入 /03sbrk（25208 字节）
[fs] 写入 /cat_filea（18912 字节）
[fs] 写入 /filetest_simple（19104 字节）
[fs] 写入 /forktest（23360 字节）
[fs] 写入 /initproc（18920 字节）
[fs] 挂载成功，根目录: ["00hello_world", "01yield", "02busy", "03sbrk",
                      "cat_filea", "filetest_simple", "forktest", "hello.txt", "initproc"]
[rc-aarch] stage 6: 进程 + 文件系统（RAM 磁盘 easy-fs）on Jailhouse cell3
...
[initproc] 子进程 exec filetest_simple
[filetest] open(filea, CREATE|WRONLY) = 3
[filetest] 写入 42 字节
[filetest] open(filea, RDONLY) = 3
[filetest] 读回 42 字节：
[filetest] 截断后读回 0 字节（应该是 0）
[filetest] pass.
[initproc] 子进程 pid=2 结束，exit_code=0（waitpid 返回 2）
[initproc] 子进程 exec cat_filea
[cat] hello.txt 共 32 字节：
hello from easy-fs on rc-aarch!
[cat] 结束
[initproc] 子进程 exec forktest
...
[forktest] 5 个孩子全部回收，exit_code 之和 = 10
[forktest] pass.
[kernel] shutdown：initproc exit_code=0；页帧 7759/8048 空闲
```

最后一行的 `7759/8048` 很有意思：比 ch5 的 8033 少了 274 页 ≈ **1MB RAM 磁盘**（256 页）
+ initproc 自己（~18 页）。说明磁盘是常驻的，进程资源仍然正常回收。

---

## 六、本章的坑 / 设计要点

### 坑 1：块缓冲区必须对齐，否则 `*const DiskInode` 是未对齐指针

`[u8; 512]` 的对齐是 **1**。把它 `as *const DiskInode`（要求 4 字节对齐）来读结构体，
在真机上可能直接触发对齐异常（或者悄悄读错）。
→ 定义 `#[repr(C, align(8))] struct BlockBuf([u8; 512])`，所有"块当结构体看"的地方都用它。
（`const _: () = assert!(size_of::<DiskInode>() == 128);` 这类编译期断言也顺手加上。）

### 坑 2：盘不能太小 —— inode 位图 1 块 = 4096 个 inode ≠ 4096 个块

easy-fs 的公式是：`inode_area_blocks = inode_num * 128 / 512`。1 块位图 → 4096 个 inode
→ **inode 区就要 1024 块**。所以 `total_blocks` 至少得 1000 多，否则 `data_total_blocks`
下溢。我们的 2048 块（1MB）刚好：1025 块给 inode，1021 块给数据。

### 坑 3：`write_at` 之前必须先 `increase_size`，否则写到"不存在的块"

`Inode::write_at` 里先 `increase_size(offset+len)` 分配数据块并更新 `size`，
再 `DiskInode::write_at`（它 assert 了 `offset+len <= size`）。
如果直接调 `DiskInode::write_at`，会读到块号 0 的目录项区，把文件系统写坏。

### 坑 4：`exec` 读出来的 ELF 是 `Vec<u8>`，exec 期间不能被回收

`read_file_by_name` 返回 `Vec<u8>`（堆上），`exec(&data)` 在里面建 MemorySet；
Vec 在 syscall 返回后 drop —— 那时 ELF 已经被拷进用户页帧了，没问题。
（如果做"按需分页/懒加载"，就得让 ELF 活得更久。）

### 坑 5：fork 共享文件对象，不是复制

`fd_table.clone()` 克隆的是 `Arc<OSInode>`，父子**共享同一个读写偏移**（和 Linux 的 fork 一致）。
如果要"各读各的"，得在 fork 时复制 OSInode 和偏移（那是 dup/独立 open 的语义）。

### 坑 6：RAM 磁盘的页帧故意不还

`frame_alloc_contig(256)` 拿到的 1MB 没有 `FrameTracker` 管着，永远不释放。
这是有意的（磁盘要一直存在）；文档里写清楚，免得以后以为是泄漏。
真要回收，得在关机路径里 `frame_dealloc_range`。

### 坑 7（沿用 ch5）：`exec` 的路径还是 C 字符串

`open("filea\0", ...)`、`exec("cat_filea\0")` 都要带 `\0`。所以 `cat_filea` 里写成
`open("hello.txt\0", O_RDONLY)`。

---

## 七、文件清单（ch6 新增/改动）

| 文件 | 作用 |
|---|---|
| `easy-fs/Cargo.toml`、`easy-fs/src/{lib,block_dev,layout,bitmap,efs,vfs}.rs` | **新** easy-fs crate |
| `os-cell3/src/fs.rs` | **新** RamDisk / OSInode / OpenFlags / open_file / read_file_by_name / init |
| `os-cell3/Cargo.toml` | 加 `easy-fs` path 依赖 |
| `os-cell3/src/task/task.rs` | TCB 加 `fd_table` + `alloc_fd`；fork 继承 fd 表 |
| `os-cell3/src/syscall/fs.rs` | 重写：open/close/read/write 按 fd 分发 |
| `os-cell3/src/syscall/mod.rs` | 56 / 57 / 63 |
| `os-cell3/src/syscall/process.rs` | exec 改为从文件系统读 ELF |
| `os-cell3/src/loader.rs` | 加 `app_name(i)`（造盘时用）|
| `os-cell3/src/main.rs` | `fs::init()`、stage 6 流程 |
| `user/src/syscall.rs`、`user/src/lib.rs` | open/close/read + `O_*` 标志 |
| `user/src/bin/{filetest_simple,cat_filea}.rs` | **新** 验收 app |
| `user/src/bin/initproc.rs` | 依次 exec 三个 app |

---

## 八、怎么复现

```sh
cd ~/rc-aarch/os-cell3
RUSTUP_TOOLCHAIN=stable cargo build --release   # 会顺带编 ../easy-fs 和 ../user

cd ~/rc-aarch
RELOAD=1 ROOTCELL=root@192.168.31.75 \
  SSHOPTS="-F /dev/null -o StrictHostKeyChecking=accept-new" \
  ./scripts/deploy-cell3.sh
```

期望：`[fs] 挂载成功，根目录: [...]` → `[filetest] pass.` → `[cat] hello.txt 共 32 字节`
→ forktest pass → shutdown（页帧 ≈ 7759/8048）。

排错速查：

| 现象 | 大概率原因 |
|---|---|
| 挂载时 `magic 不对` | 格式化没跑完/超级块写错；或 `open` 前盘被别人覆盖 |
| `创建 xxx 失败` | inode 位图耗尽（盘太小）或目录项满 |
| 写文件后读回乱码 | 块缓冲区未对齐，或 `increase_size` 没在 `write_at` 前调用 |
| `文件太大` panic | 单文件超过 indirect1 上限（78KB）|
| exec 失败 | 文件名不对（大小写/后缀），或 exec 的 ELF 是坏的 |

---

## 九、今天做到哪、还剩什么（ch6 未完部分）

已完成（今天）：
- easy-fs crate（布局/位图/EFS/VFS，读写都通）
- RAM 磁盘 + 启动格式化 + 内嵌 app 落盘
- open/close/read/write 系统调用 + fd 表 + fork 继承
- exec 从文件系统加载 ELF
- 文件创建/写入/读回/截断、cat、以及进程能力回归

还没做（ch6 的边角 / ch7 的入口）：
1. **`sys_read` 的 stdin**：需要 UART 接收中断或轮询，才能跑 user_shell；
2. **`sys_dup` / 子目录 / 路径解析**（现在只支持根目录一级）；
3. **indirect2**（>78KB 文件）；
4. **块缓存**（真机 virtio 时才需要）；
5. **持久化**：现在 RAM 磁盘掉电即失，真持久化要接 SD/ivshmem（ch7 的 ivshmem 是个好入口）。

ch7（进程间通信）预告：管道 `pipe` + `sys_dup`、`sys_read/write` 对管道的支持、
以及（可选）共享内存/ivshmem。
