#!/bin/sh
# 板子 IP 变了怎么办：按 MAC 在局域网里找它
#
#   ./scripts/find-board.sh            # 默认扫 192.168.31.0/24
#   SUBNET=192.168.31 ./scripts/find-board.sh
#
# 原理：并发 ping 一遍网段（把邻居表填满），再从 `ip neigh` 里按 MAC 查。
# 板子 root cell 是 initramfs，IP 走 DHCP → 路由器重启/租约过期就可能换地址；
# 想彻底固定，在路由器上给这个 MAC 做 DHCP 静态绑定（或修改 SD 卡的 initramfs 网络配置）。
set -e
SUBNET=${SUBNET:-192.168.31}
MAC=${MAC:-dc:a6:32:cc:41:b4}
i=1
while [ "$i" -le 254 ]; do
    ping -c1 -W1 "$SUBNET.$i" >/dev/null 2>&1 &
    i=$((i + 1))
done
wait 2>/dev/null || true
sleep 1
ip neigh | grep -i "$MAC" | awk '{print $1}' | while read -r ip; do
    echo "板子在线：$ip（MAC $MAC）"
done
