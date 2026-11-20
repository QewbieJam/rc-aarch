#!/bin/sh
# 把验收需要的东西一次性传到板子上（宿主机执行）
#
#   ./scripts/pack-verify.sh
#   然后到板子上： sh /root/verify-on-board.sh
set -e
cd "$(dirname "$0")/.."
BOARD=${BOARD:-root@192.168.31.75}
SSHOPTS=${SSHOPTS:-"-F /dev/null -o StrictHostKeyChecking=accept-new"}

echo "== 编译内核 =="
(cd os-cell3 && RUSTUP_TOOLCHAIN=stable cargo build --release)

echo "== 编译 cell 配置 =="
./scripts/build-cell.sh

echo "== 编译 ivshm 守护进程 =="
aarch64-linux-gnu-gcc -O2 -static -o /tmp/ivshm-console tools/ivshm-console.c

echo "== 上传到板子 /root/ =="
scp $SSHOPTS os-cell3/target/aarch64-unknown-none/release/rc-aarch-os "$BOARD:/root/rc-aarch.elf"
scp $SSHOPTS cells/rpi4-rust-cell3.cell "$BOARD:/root/rc-aarch.cell"
scp $SSHOPTS /tmp/ivshm-console "$BOARD:/root/ivshm-console"
scp $SSHOPTS scripts/verify-on-board.sh "$BOARD:/root/verify-on-board.sh"
ssh $SSHOPTS "$BOARD" 'chmod +x /root/ivshm-console /root/verify-on-board.sh'

echo ""
echo "✅ 已上传。到板子上执行："
echo "   ssh $BOARD"
echo "   sh /root/verify-on-board.sh"
