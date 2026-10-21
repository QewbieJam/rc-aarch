#!/bin/sh
# 把 cell3 的配置 + Rust 内核送到 root cell，并在那里 create / load / start
#   ROOTCELL=root@192.168.10.8 scripts/deploy-cell3.sh [kernel.bin]
# 默认内核：os-cell3/target/aarch64-unknown-none/release/rc-aarch-os
#
# ★ RELOAD=1：日常改内核用这条路径
#   RELOAD=1 ROOTCELL=root@192.168.31.75 scripts/deploy-cell3.sh
#   只 scp 新 ELF，然后 cell shutdown → load → start，**不 destroy**。
#
# ★ 板子 IP 变了（DHCP 重新分配）：./scripts/find-board.sh 按 MAC 扫网段找它；
#   注意脚本只编 .cell 配置、**不会** cargo build 内核，改完内核记得先 build。
#
#   为什么日常不用 destroy：实测（见 docs/02 的"三个坑"）destroy 会把 CPU3
#   还给 root cell，Linux 热插拔 CPU3 时在 secondary_start_kernel/el1_irq 里
#   吃 SError → Kernel panic → 整板失联，只能断电。
#   而 shutdown → load → start 复用同一个 cell（CPU3 始终留在 cell 里）不会触发。
#   注意：RELOAD 要求 cell 已经存在；全新上电/没 create 过时会自动回退到完整流程。
set -e

# 无论从哪里调用，都切到仓库根（脚本在 <root>/scripts/ 下）
cd "$(dirname "$0")/.."
ROOTCELL=${ROOTCELL:-root@192.168.31.74}
# 传 ELF（推荐，工具会读它的加载地址）或 .bin（脚本会自动补 -a 0xFA000000）
BIN=${1:-os-cell3/target/aarch64-unknown-none/release/rc-aarch-os}
RELOAD=${RELOAD:-0}
# ssh 连接复用：scp/ssh 多次调用只问一次密码
# 允许外部覆盖（例如本机 /etc/ssh 所有权坏了时：SSHOPTS="-F /dev/null"）
SSHOPTS="${SSHOPTS:--o ControlMaster=auto -o ControlPath=/tmp/rc-aarch-ssh-%r@%h:%p -o ControlPersist=120}"
[ -f "$BIN" ] || { echo "没有 $BIN —— 先编内核（make）并 objcopy -O binary"; exit 1; }
./scripts/build-cell.sh

case "$BIN" in
    *.bin) REMOTE=/tmp/rc-aarch.bin ;;
    *)     REMOTE=/tmp/rc-aarch.elf ;;
esac
scp $SSHOPTS cells/rpi4-rust-cell3.cell "$ROOTCELL:/tmp/rc-aarch.cell"
ssh $SSHOPTS "$ROOTCELL" 'rm -f /tmp/rc-aarch.bin /tmp/rc-aarch.elf'   # 清掉上次的残留
scp $SSHOPTS "$BIN" "$ROOTCELL:$REMOTE"

ssh $SSHOPTS "$ROOTCELL" "
  # hypervisor 没使能就先使能（已使能时 cell list 会成功，不重复 enable）
  # ★ 用 sysfs 的标志判断是否已使能（cell list 在未使能时也返回成功，不能用它判断）
  [ \"\$(cat /sys/devices/jailhouse/enabled 2>/dev/null)\" = \"1\" ] || jailhouse enable /usr/share/jailhouse/cells/rpi4.cell

  # ★★ 控制台怎么读（ch3 实测的关键，见 docs/03）：
  #   /sys/devices/jailhouse/console 是**前 2048 字节的快照**，写满就不再更新 →
  #   跑几次以后看到的永远是开头那 2KB（还以为是\"卡住了\"）。
  #   /dev/jailhouse 是个\"增量\"接口：每个 fd 自己记 head，read 只给上次之后的**新**内容；
  #   增量超过 2048 时先返回一行 '<missed N bytes>'(并把 head 前移)，再读一次才给数据。
  #   所以：**同一个 fd** 先 drain 掉旧内容，再 load/start，之后边跑边循环读
  #   （每 200ms 一次，2s 没新数据才停 —— 见本脚本最后那段 while）。
  # ★ 只能让 dd **继承** fd 3（<&3）；写 if=/dev/fd/3 会重新 open 设备，
  #   每次 read 的 head 都从 0 开始 → 永远收到同一行 '<missed ...>'。
  exec 3</dev/jailhouse
  drain_console() {
      i=0
      while [ \$i -lt 16 ]; do
          n=\$(dd bs=8192 count=1 iflag=nonblock status=none <&3 2>/dev/null | wc -c)
          [ \"\$n\" -gt 0 ] || break
          i=\$((i+1))
      done
  }
  drain_console

  # 注意：cell load/start/destroy 的第一个位置参数是 ID；按名字要用 --name
  HAVE=\$(jailhouse cell list 2>/dev/null | grep -c 'rc-aarch' || true)
  if [ '$RELOAD' = '1' ] && [ \"\$HAVE\" != '0' ]; then
      echo '>>> RELOAD：复用已有 cell（shutdown → load → start，不 destroy）'
      jailhouse cell shutdown --name rc-aarch || true
      sleep 1
  else
      [ '$RELOAD' = '1' ] && echo '>>> RELOAD 但 cell 不存在 → 回退到完整 create 流程'
      jailhouse cell destroy --name rc-aarch 2>/dev/null || true
      jailhouse cell create /tmp/rc-aarch.cell
  fi

  # ★ 镜像必须落在 cell 的 LOADABLE 区（0xFA000000）内，否则驱动直接 EINVAL：
  #   - ELF：工具从 ELF 头取加载地址（要求链接基址 = 0xFA000000）
  #   - raw .bin：工具默认用 0 → 必须显式 -a 0xFA000000
  if [ -f /tmp/rc-aarch.bin ]; then
      IMG=/tmp/rc-aarch.bin; LOADARG='-a 0xFA000000'   # raw .bin 工具默认用 0，必须显式给地址
  else
      IMG=/tmp/rc-aarch.elf; LOADARG=''                # ELF 自带加载地址
  fi
  if ! jailhouse cell load --name rc-aarch \$IMG \$LOADARG; then
      echo '>>> LOAD 失败：检查内核链接基址是否为 0xFA000000（rust-objdump -h 看 .text 的 VMA），且镜像 < 1MB —— 不执行 start'
      jailhouse cell list; exit 1
  fi
  drain_console   # load/create 自己的 hypervisor 日志也丢掉，只看 cell 的输出
  jailhouse cell start --name rc-aarch
  sleep 0.3
  jailhouse cell list
  echo '===== cell 控制台（hvc 输出，增量读取）====='
  # ★ 边跑边读：cell 一次可能吐 >2KB，一次 read 装不下（会只剩 newest 2KB），
  #   所以每 200ms 抓一次，连续 10 次没新数据（=2s）才停。
  idle=0
  while [ \$idle -lt 10 ]; do
      out=\$(dd bs=8192 count=1 iflag=nonblock status=none <&3 2>/dev/null)
      if [ -n \"\$out\" ]; then
          printf '%s' \"\$out\"
          idle=0
      else
          sleep 0.2
          idle=\$((idle+1))
      fi
  done
  echo
"
