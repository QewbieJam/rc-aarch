#!/bin/sh
# 在 Debian 宿主机上把 cells/*.c 编成 jailhouse 能吃的 *.cell
# 复刻 jailhouse configs/Makefile 的规则：gcc -c → objcopy -O binary
set -e

# 无论从哪里调用，都切到仓库根（脚本在 <root>/scripts/ 下）
cd "$(dirname "$0")/.."
JH=${JH:-/home/era/EMCS/build/raspberrypi4-64/tmp/work/raspberrypi4_64-openeuler-linux/jailhouse/0.12-r0/jailhouse-0.12}
src=${1:-cells/rpi4-rust-cell3.c}
cell=${src%.c}.cell
obj=${src%.c}.o
[ -d "$JH/include/jailhouse" ] || { echo "找不到 jailhouse 源码: $JH（用 JH=... 指定）"; exit 1; }
gcc -Werror -Wall -Wextra -D__LINUX_COMPILER_TYPES_H \
    -I"$JH/hypervisor/arch/arm64/include" -I"$JH/hypervisor/include" -I"$JH/include" \
    -c "$src" -o "$obj"
objcopy -O binary --remove-section=.note.gnu.property "$obj" "$cell"
echo "OK: $cell ($(stat -c%s "$cell") 字节)"
