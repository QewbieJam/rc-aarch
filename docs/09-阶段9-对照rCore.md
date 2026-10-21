# 📘 阶段 9：文件系统优化（块缓存）—— 对照 rCore ch9 逐项讲

> 项目：rc-aarch ｜ 工程：`os-cell3/` + `easy-fs/` + `user/`
> 状态：**✅ ch9 的块缓存已在树莓派 4B / openEuler MCS / Jailhouse cell3 上跑通**
> 分支：`ch7+`（在 ch8 之后继续演进）
> 相关：[[08-阶段8-对照rCore]] ｜ [[07-阶段7-对照rCore]]

---

## 一、验收标准（实测通过）

| 检查项 | 状态 | 实测证据 |
|---|---|---|
| 块缓存生效，读命中率高 | ✅ | `命中 12435 / 未命中 2219 / 写 7031 / 写回 1216（命中率 84%）` |
| 反复 open+read 的开销稳定 | ✅ | `200 次 open+read+close 用时 4 ms` |
| 写回策略不丢数据 | ✅ | 写缓存 + 淘汰/shutdown 时 `sync_all`，重新 open 校验通过 |
| 既有能力回归 | ✅ | ch1~ch8 全部 app 通过，`页帧 7371/7938 空闲` |
| 可对照实验（缓存开/关）| ✅ | `CACHE_ENABLE` 开关；关掉后命中/未命中都是 0 |

> ⚠️ 诚实说明：我们的"磁盘"是**内存里的 RAM 磁盘**，所以缓存开/关的**墙上时间几乎
> 一样**（都 3~4ms）——缓存省下的是重复的"块解析 + memcpy"，不是真实的设备延迟。
> 因此 ch9 的效果用 **命中率 / 设备访问次数** 衡量（84% 的读由缓存承接），
> 真机上换成 SD/eMMC 时时间差异才会显现。

---

## 二、upstream ch9 做了什么

- 重写 easy-fs：块大小 512B、`SuperBlock`/`Bitmap`/`DiskInode` 更紧凑，代码量减半；
- 在 easy-fs 内部加**块缓存**（`block_cache.rs`）：全局缓存数组 + LRU 淘汰 +
  `sync_all`，读写都先过缓存（写回策略）；
- 上层（内核 `os/src/fs`）几乎不用改，因为缓存藏在 `BlockDevice` 之下。

---

## 三、我们做了什么

**取舍**：没有重写 easy-fs（ch6 的实现在我们的移植里已经验证过、和 upstream ch6
逐行对应），而是把块缓存放在 **`BlockDevice` 这一层**（`fs/block.rs` 的
`CachedDisk` 包住 `RamDisk`），内核其它代码一行不改就能吃到缓存的好处。

```text
easy-fs（不变）
     │  read_block / write_block
     ▼
CachedDisk（ch9 新增）
     │  32 个 512B 槽位 + LRU + 写回 + 统计
     ▼
RamDisk（页帧池里的连续 2MB）
```

- **读**：命中直接拷内存；未命中读盘并占一个槽位（满了按 LRU 淘汰，脏块先写回）；
- **写**：已在缓存 → 只改缓存并标脏（write-back）；不在缓存 → 直接写穿；
- **统计**：命中/未命中/写/写回次数，停机时打印；
- **`cache_sync_all()`**：`fs::init` 末尾和停机时各调一次，保证盘上数据与缓存一致。

---

## 四、实测输出（2026-10-22）

```
[fs_perf] 200 次 open+read+close（hello.txt）用时 4 ms
[fs_perf] 100 次「写 512B + 读回」用时 4 ms
...
[kernel] shutdown：initproc exit_code=0；页帧 7371/7938 空闲
[kernel]           块缓存：命中 12435 / 未命中 2219 / 写 7031 / 写回 1216（命中率 84%）
```

对照实验（`CACHE_ENABLE = false`，直接读盘）：

```
[fs_perf] 200 次 open+read+close（hello.txt）用时 3 ms
[fs_perf] 100 次「写 512B + 读回」用时 4 ms
[kernel]           块缓存：命中 0 / 未命中 0 / 写 0 / 写回 0（命中率 0%）
```

---

## 五、这一阶段踩的坑 / 设计要点

### 1. 写回策略必须有明确的 sync 点

写回（write-back）意味着"缓存和盘上可能暂时不一致"。如果只在淘汰时写回，
进程正常退出、系统停机时那些还在缓存里的脏块就丢了。所以我们：
- `fs::init` 末尾 `cache_sync_all()`；
- `shutdown_with` 里再 `cache_sync_all()` 一次（打印统计之前）。

### 2. 缓存层放在哪：trait 边界 vs 重写 easy-fs

upstream 把缓存塞进 easy-fs 内部；我们包在 `BlockDevice` 外面。两者语义等价，
后者改动面小（easy-fs 不动、上层零改动），代价是"缓存只按块号索引、不知道
inode 结构"。对教学/演示足够，也在文档里如实标注这处偏差。

### 3. RAM 磁盘上"时间"不是好指标

一开始想用 `get_time()` 的毫秒差证明缓存有用，实测开/关都是 3~4ms：
因为底层本来就是内存，缓存省的是重复解析和 memcpy，量级在微秒。
所以改成用**命中率**（84%）和"设备访问次数"（未命中 2219 次）作为证据，
并写明真机块设备上时间差异才会明显。

### 4. 缓存槽位要能淘汰"脏块"

`alloc_slot` 在缓存满时先从 LRU 队头拿槽位，如果那个块是脏的必须先
`write_block` 写回，否则数据丢失。写回次数（1216）就是这条路径的计数。

---

## 六、文件清单（ch9）

| 文件 | 作用 |
|---|---|
| `os-cell3/src/fs/block.rs` | **改**：`CachedDisk`（块缓存 + LRU + 写回 + 统计）、`cache_stats`/`cache_sync_all`、`CACHE_ENABLE` 对照开关 |
| `os-cell3/src/fs/mod.rs` | 把 `CachedDisk` 传给 easy-fs；init 末尾 sync |
| `os-cell3/src/trap/mod.rs` | 停机时打印块缓存统计并 sync |
| `user/src/bin/fs_perf.rs` | **新**：200 次 open+read、100 次写+读回，测耗时 |
| `user/src/bin/initproc.rs` | app 列表加 `fs_perf` |

---

## 七、怎么复现

```sh
cd ~/rc-aarch/os-cell3 && RUSTUP_TOOLCHAIN=stable cargo build --release
cd ~/rc-aarch
RELOAD=1 ROOTCELL=root@192.168.31.75 \
  SSHOPTS="-F /dev/null -o StrictHostKeyChecking=accept-new" \
  ./scripts/deploy-cell3.sh
# 想要对照：把 os-cell3/src/fs/block.rs 的 CACHE_ENABLE 改成 false 再跑一遍
```

期望：`[fs_perf] ... 用时 ... ms`，停机时 `块缓存：命中 ... / 未命中 ...（命中率 ~84%）`，
其余 app 全部通过。

---

## 八、没做 / 下一步

1. **inode 缓存**：现在每次 open 都要从盘上重新解析 inode（虽然走了块缓存）。
   upstream ch9 也没做独立的 inode 缓存，但可以作为 ch9+ 的延伸。
2. **真实块设备的吞吐**：Jailhouse cell3 没有块设备（SD 卡归 root cell），
   要测真实 I/O 得走 ivshmem 或 virtio，属于另一个课题。
3. 更细的缓存替换策略（LRU-K、写合并）在单核 RAM 盘上没有意义，暂不做。

> ch1~ch9 到此全部跑通；`ch7+` 分支同时承载了堆分配器、ivshmem 控制台、
> 线程/同步和块缓存这些"技术债 + 优化"的内容。
