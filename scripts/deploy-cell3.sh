#!/bin/sh
# 把 cell3 的配置 + Rust 内核送到 root cell，并在那里 create / load / start
#   ROOTCELL=root@192.168.10.8 scripts/deploy-cell3.sh [kernel.bin]
# 默认内核：os1/target/aarch64-unknown-none/release/rc-aarch-os1.bin
set -e

# 无论从哪里调用，都切到仓库根（脚本在 <root>/scripts/ 下）
cd "$(dirname "$0")/.."
ROOTCELL=${ROOTCELL:-root@192.168.31.74}
# 传 ELF（推荐，工具会读它的加载地址）或 .bin（脚本会自动补 -a 0xFA000000）
BIN=${1:-os1/target/aarch64-unknown-none/release/rc-aarch-os1}
# ssh 连接复用：scp/ssh 多次调用只问一次密码
SSHOPTS="-o ControlMaster=auto -o ControlPath=/tmp/rc-aarch-ssh-%r@%h:%p -o ControlPersist=120"
[ -f "$BIN" ] || { echo "没有 $BIN —— 先编内核（make）并 objcopy -O binary"; exit 1; }
./scripts/build-cell.sh

scp $SSHOPTS cells/rpi4-rust-cell3.cell "$ROOTCELL:/tmp/rc-aarch.cell"
case "$BIN" in
    *.bin) REMOTE=/tmp/rc-aarch.bin ;;
    *)     REMOTE=/tmp/rc-aarch.elf ;;
esac
ssh $SSHOPTS "$ROOTCELL" 'rm -f /tmp/rc-aarch.bin /tmp/rc-aarch.elf'   # 清掉上次的残留
scp $SSHOPTS "$BIN" "$ROOTCELL:$REMOTE"

ssh $SSHOPTS "$ROOTCELL" '
  # hypervisor 没使能就先使能（已使能时 cell list 会成功，不重复 enable）
  # ★ 用 sysfs 的标志判断是否已使能（cell list 在未使能时也返回成功，不能用它判断）
  [ "$(cat /sys/devices/jailhouse/enabled 2>/dev/null)" = "1" ] || jailhouse enable /usr/share/jailhouse/cells/rpi4.cell

  # 注意：cell load/start/destroy 的第一个位置参数是 ID；按名字要用 --name
  jailhouse cell destroy --name rc-aarch 2>/dev/null || true
  jailhouse cell create /tmp/rc-aarch.cell
  # ★ 镜像必须落在 cell 的 LOADABLE 区（0xFA000000）内，否则驱动直接 EINVAL：
  #   - ELF：工具从 ELF 头取加载地址（要求链接基址 = 0xFA000000）
  #   - raw .bin：工具默认用 0 → 必须显式 -a 0xFA000000
  if [ -f /tmp/rc-aarch.bin ]; then
      IMG=/tmp/rc-aarch.bin; LOADARG="-a 0xFA000000"   # raw .bin 工具默认用 0，必须显式给地址
  else
      IMG=/tmp/rc-aarch.elf; LOADARG=""                # ELF 自带加载地址
  fi
  if ! jailhouse cell load --name rc-aarch "$IMG" $LOADARG; then
      echo ">>> LOAD 失败：检查内核链接基址是否为 0xFA000000（rust-objdump -h 看 .text 的 VMA），且镜像 < 1MB —— 不执行 start"
      jailhouse cell list; exit 1
  fi
  jailhouse cell start --name rc-aarch
  sleep 1
  jailhouse cell list
  echo "===== 虚拟控制台（hvc 输出）====="
  cat /sys/devices/jailhouse/console
'
