#!/bin/sh
# 在树莓派 root cell 上直接跑逐章验收（不依赖宿主机）
#
#   宿主机先跑： ./scripts/pack-verify.sh            # 编译 + 上传内核/cell/守护进程/本脚本
#   然后板子上： sh /root/verify-on-board.sh          # 本脚本
#
# 产物：/root/verify-board.log（虚拟机控制台）、/root/verify-board-shell.log（共享内存 shell 会话）
set -u

NAME=rc-aarch
KERNEL=${KERNEL:-/root/rc-aarch.elf}
CELL=${CELL:-/root/rc-aarch.cell}
LOG=${LOG:-/root/verify-board.log}
SHELL_LOG=${SHELL_LOG:-/root/verify-board-shell.log}

say() { printf '%s\n' "$*"; }
has() { grep -q "$2" "$1" 2>/dev/null; }

[ -f "$KERNEL" ] || { say "缺少 $KERNEL：先在宿主机跑 ./scripts/pack-verify.sh"; exit 1; }

# 1) Jailhouse 使能（重启后不保留）
[ "$(cat /sys/devices/jailhouse/enabled 2>/dev/null)" = "1" ] || \
    jailhouse enable /usr/share/jailhouse/cells/rpi4.cell

# 2) 共享内存切到 uio（给守护进程用）
if [ ! -e /dev/uio0 ]; then
    echo -n 0001:00:00.0 > /sys/bus/pci/drivers/ivshmem-net/unbind 2>/dev/null || true
    echo -n "110a 4106" > /sys/bus/pci/drivers/uio_ivshmem/new_id 2>/dev/null || true
    sleep 1
fi

# 3) 起守护进程（喂一段 shell 命令，验证 ch7+）
WITH_SHELL=0
if [ -x /root/ivshm-console ] && [ -e /dev/uio0 ]; then
    WITH_SHELL=1
    pkill -x ivshm-console 2>/dev/null || true
    rm -f "$SHELL_LOG" /root/verify-board-shell.err
    setsid nohup sh -c "printf 'help\ncmdline_args a b\ncat_filea\nexit\n' | /root/ivshm-console -i -d 120" \
        > "$SHELL_LOG" 2>/root/verify-board-shell.err < /dev/null &
    sleep 0.5
fi

# 4) 重建/复用 cell（不 destroy，避免 CPU 热插拔掉板）
if jailhouse cell list 2>/dev/null | grep -q "$NAME"; then
    jailhouse cell shutdown --name "$NAME" 2>/dev/null || true
    sleep 1
else
    jailhouse cell create "$CELL"
fi
jailhouse cell load --name "$NAME" "$KERNEL" || { say "cell load 失败"; exit 1; }

# 没有守护进程时清掉共享内存残留输入
if [ "$WITH_SHELL" = 0 ] && [ -x /root/ivshm-console ] && [ -e /dev/uio0 ]; then
    /root/ivshm-console -r >/dev/null 2>&1 || true
fi

# 5) 启动并抓控制台（增量读，2s 没输出就停）
exec 3</dev/jailhouse
drain() { i=0; while [ $i -lt 16 ]; do n=$(dd bs=8192 count=1 iflag=nonblock status=none <&3 2>/dev/null | wc -c); [ "$n" -gt 0 ] || break; i=$((i+1)); done; }
drain
jailhouse cell start --name "$NAME"
: > "$LOG"
idle=0
while [ $idle -lt 15 ]; do
    out=$(dd bs=8192 count=1 iflag=nonblock status=none <&3 2>/dev/null)
    if [ -n "$out" ]; then printf '%s' "$out" >> "$LOG"; idle=0; else sleep 0.2; idle=$((idle+1)); fi
done

# 6) 逐章断言
FAIL=0
ROWS=""
row() { ROWS="${ROWS}$1|$2|$3
"; [ "$2" = "PASS" ] || FAIL=$((FAIL+1)); }

if has "$LOG" "Started cell" && has "$LOG" "stage-1 MMU 已打开" && has "$LOG" "stext"; then
    row "ch1 裸机入口 + UART" PASS "0xfa000000 启动、MMU 恒等映射、控制台输出"
else
    row "ch1 裸机入口 + UART" FAIL "缺启动/MMU 证据"
fi

if has "$LOG" "Hello, world from EL0" && has "$LOG" "svc from EL0"; then
    row "ch2 trap/syscall/EL0" PASS "EL0 hello + svc 陷入"
else
    row "ch2 trap/syscall/EL0" FAIL "缺 EL0 hello 或 svc 日志"
fi

if has "$LOG" "ch3] 并发跑" && has "$LOG" "app1] A0" && has "$LOG" "app2] B0" && \
   has "$LOG" "第 1 次 tick（来自 EL0）"; then
    row "ch3 任务切换 + 抢占" PASS "01yield/02busy 并发、定时器 tick"
else
    row "ch3 任务切换 + 抢占" FAIL "缺并发任务/tick 证据"
fi

if has "$LOG" "remap_test passed" && has "$LOG" "app3] 新页可读可写" && \
   has "$LOG" "app3] 跨页读写 OK"; then
    row "ch4 地址空间 + sbrk" PASS "remap_test、扩页/跨页"
else
    row "ch4 地址空间 + sbrk" FAIL "缺 remap_test 或 sbrk 证据"
fi

if has "$LOG" "forktest] pass" && has "$LOG" "子进程 exec" && has "$LOG" "waitpid 返回"; then
    row "ch5 fork/exec/waitpid" PASS "forktest + 全链路"
else
    row "ch5 fork/exec/waitpid" FAIL "缺 forktest 证据"
fi

if has "$LOG" "filetest] pass" && has "$LOG" "cat] hello.txt 共 32 字节"; then
    row "ch6 文件系统" PASS "filetest + cat 读文件"
else
    row "ch6 文件系统" FAIL "缺文件系统证据"
fi

if has "$LOG" "pipetest] passed" && has "$LOG" "pipe_large] passed" && \
   has "$LOG" "校验和 382468" && has "$LOG" "sig] sig_test passed" && \
   has "$LOG" "cmdline] argc = 3"; then
    row "ch7 pipe + 信号 + argv" PASS "pipe_large(382468)/sig_test/argv"
else
    row "ch7 pipe + 信号 + argv" FAIL "缺 pipe/信号/argv 证据"
fi

if [ "$WITH_SHELL" = 1 ] && [ -s "$SHELL_LOG" ] && \
   has "$SHELL_LOG" "shell] rc-aarch user_shell" && has "$SHELL_LOG" "可用命令" && \
   has "$SHELL_LOG" "cmdline] argc = 3" && has "$SHELL_LOG" "shell] bye"; then
    row "ch7+ 共享内存控制台" PASS "stdin/stdout + user_shell 会话完整"
else
    row "ch7+ 共享内存控制台" SKIP "WITH_SHELL=0 或守护进程未就绪"
fi

if has "$LOG" "thread_test passed" && has "$LOG" "mutex_test passed" && \
   has "$LOG" "sync_test passed" && has "$LOG" "SHARED=4000" && \
   has "$LOG" "信号量验收通过" && has "$LOG" "条件变量验收通过"; then
    row "ch8 线程 + 同步" PASS "thread/mutex/sync 全过"
else
    row "ch8 线程 + 同步" FAIL "缺线程/同步证据"
fi

RATE=$(sed -n 's/.*命中率 \([0-9][0-9]*\)%.*/\1/p' "$LOG" 2>/dev/null | tail -1)
if has "$LOG" "fs_perf] 200 次" && has "$LOG" "块缓存：命中" && \
   [ -n "$RATE" ] && [ "$RATE" -ge 50 ]; then
    row "ch9 块缓存" PASS "fs_perf 跑通、命中率 ${RATE}%"
else
    row "ch9 块缓存" FAIL "缺 fs_perf 或命中率不足（${RATE:-?}%）"
fi

if ! has "$LOG" "PANIC" && ! has "$LOG" "Panicked" && \
   has "$LOG" "shutdown：initproc exit_code=0"; then
    row "全局" PASS "无 panic、initproc 正常退出"
else
    row "全局" FAIL "出现 panic 或没有正常停机"
fi

# 收尾：停掉守护进程
pkill -x ivshm-console 2>/dev/null || true

# 7) 报表
say ""
say "================= 逐章验收（板端执行） ================="
printf '%-6s %-26s %s\n' "结果" "章节" "证据"
printf '%-6s %-26s %s\n' "------" "--------------------------" "--------------------------"
printf '%s' "$ROWS" | while IFS='|' read -r ch res why; do
    [ -n "$ch" ] || continue
    printf '%-6s %-26s %s\n' "$res" "$ch" "$why"
done
say "========================================================"
say "日志：$LOG"
[ "$WITH_SHELL" = 1 ] && say "shell 会话：$SHELL_LOG"
if [ "$FAIL" -eq 0 ]; then
    say "✅ 全部通过"
    exit 0
else
    say "❌ 有 $FAIL 项失败"
    exit 1
fi
