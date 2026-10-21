#!/bin/sh
# 把 root cell 的 ivshmem PCI 设备从 ivshmem-net 切成 uio_ivshmem，
# 让用户态能 mmap 共享内存（ch7+ phase 1/2 的控制台通道）。
#
#   ROOTCELL=root@192.168.31.75 ./scripts/setup-ivshm-uio.sh
#
# 背景（见 docs/07 的 ivshmem 一节）：
#   - `uio_ivshmem` 驱动只在 PCI class 匹配 (0x00, 0x4001 RPMSG) 时自动绑定，
#     我们的设备是 VETH（class 0x0001），所以要用 `new_id` 动态加一条 ID；
#   - 一次只能绑一个驱动：先 unbind ivshmem-net，再 new_id，uio 会自动 probe；
#   - ★ ivshmem PCI 设备只有在 `jailhouse enable` 之后才开始应答 config cycle，
#     所以这个脚本必须在 jailhouse 使能之后跑（首次部署的完整流程里会自动 enable）。
set -e
ROOTCELL=${ROOTCELL:-root@192.168.31.75}
SSHOPTS=${SSHOPTS:--F /dev/null -o StrictHostKeyChecking=accept-new}

ssh $SSHOPTS "$ROOTCELL" '
  BDF=0001:00:00.0
  if [ ! -e /dev/uio0 ]; then
      [ "$(cat /sys/bus/pci/devices/$BDF/driver 2>/dev/null | xargs basename 2>/dev/null)" = "ivshmem-net" ] \
          && echo -n $BDF > /sys/bus/pci/drivers/ivshmem-net/unbind
      echo -n "110a 4106" > /sys/bus/pci/drivers/uio_ivshmem/new_id 2>/dev/null || true
      sleep 1
  fi
  if [ -e /dev/uio0 ]; then
      echo "OK: /dev/uio0 就绪"
      for m in /sys/class/uio/uio0/maps/map*; do
          echo "  $(basename $m): $(cat $m/name) addr=$(cat $m/addr) size=$(cat $m/size)"
      done
  else
      echo "失败：/dev/uio0 没出现"; exit 1
  fi
'
