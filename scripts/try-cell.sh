#!/bin/sh
# 二分用：编一个 cell 配置 → 送到 root cell → 试着 create（可选再 load/start）
#   ROOTCELL=root@192.168.10.8 scripts/try-cell.sh cells/var-a-min.c [kernel.bin]
set -e

# 无论从哪里调用，都切到仓库根（脚本在 <root>/scripts/ 下）
cd "$(dirname "$0")/.."
ROOTCELL=${ROOTCELL:-root@192.168.31.74}
# ssh 连接复用：scp/ssh 多次调用只问一次密码
SSHOPTS="-o ControlMaster=auto -o ControlPath=/tmp/rc-aarch-ssh-%r@%h:%p -o ControlPersist=120"
CFG=${1:?用法: try-cell.sh <cell配置.c> [kernel.bin]}
BIN=$2
./scripts/build-cell.sh "$CFG"
scp $SSHOPTS "${CFG%.c}.cell" "$ROOTCELL:/tmp/try.cell"
[ -n "$BIN" ] && scp $SSHOPTS "$BIN" "$ROOTCELL:/tmp/try.bin"

ssh $SSHOPTS "$ROOTCELL" '
  # ★ 用 sysfs 的标志判断是否已使能（cell list 在未使能时也返回成功，不能用它判断）
  [ "$(cat /sys/devices/jailhouse/enabled 2>/dev/null)" = "1" ] || jailhouse enable /usr/share/jailhouse/cells/rpi4.cell
  jailhouse cell destroy --name rc-aarch 2>/dev/null || true
  if jailhouse cell create /tmp/try.cell; then echo ">>> CREATE 成功"; else echo ">>> CREATE 失败"; fi
  if [ -f /tmp/try.bin ] && jailhouse cell load --name rc-aarch /tmp/try.bin 2>/dev/null; then
      jailhouse cell start --name rc-aarch && echo ">>> START 成功"
  fi
  jailhouse cell list
  echo "===== hypervisor 控制台（出错原因会在这里）====="
  cat /sys/devices/jailhouse/console | tail -20
'
