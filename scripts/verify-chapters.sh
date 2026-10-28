#!/bin/sh
# 逐章验收：在真实树莓派（Jailhouse cell3）上按 ch1→ch9 检查每一章的功能
#
#   ./scripts/verify-chapters.sh                 # 默认连 root@192.168.31.75
#   BOARD=root@192.168.31.78 ./scripts/verify-chapters.sh
#   WITH_SHELL=0 ./scripts/verify-chapters.sh    # 跳过 ch7+ 的 ivshmem 控制台（不启动守护进程）
#
# 做三件事：
#   1) 编译内核 + root 侧 ivshmem 守护进程（ch7+ 用）；
#   2) 在 root cell 上把 ivshmem 切到 uio、起守护进程（喂一段 shell 脚本命令）；
#   3) RELOAD 部署 cell3，抓完整控制台日志，逐章对证据做断言，最后打印 PASS/FAIL 表。
#
# 依赖：aarch64-linux-gnu-gcc（编守护进程）、能 ssh 到板子。
set -u

cd "$(dirname "$0")/.."
BOARD=${BOARD:-root@192.168.31.75}
SSHOPTS=${SSHOPTS:-"-F /dev/null -o StrictHostKeyChecking=accept-new"}
WITH_SHELL=${WITH_SHELL:-1}
LOG=${LOG:-$PWD/verify-chapters.log}
SHELL_LOG=${SHELL_LOG:-$PWD/verify-shell.log}
DAEMON=${DAEMON:-$PWD/target/ivshm-console}

say() { printf '%s\n' "$*"; }

# ---------------------------------------------------------------------------
# 0) 编译
# ---------------------------------------------------------------------------
say "== [0/3] 编译内核 =="
(cd os-cell3 && RUSTUP_TOOLCHAIN=stable cargo build --release >/dev/null) || {
    say "内核编译失败"; exit 1;
}

if [ "$WITH_SHELL" = 1 ]; then
    say "== [0/3] 编译 root 侧 ivshmem 守护进程 =="
    mkdir -p "$(dirname "$DAEMON")"
    aarch64-linux-gnu-gcc -O2 -static -o "$DAEMON" tools/ivshm-console.c || {
        say "守护进程编译失败（跳过 ch7+）"; WITH_SHELL=0;
    }
fi

# ---------------------------------------------------------------------------
# 1) root 侧准备：jailhouse enable → ivshmem 切 uio → 起守护进程
# ---------------------------------------------------------------------------
if [ "$WITH_SHELL" = 1 ]; then
    say "== [1/3] root 侧准备（jailhouse / uio / 守护进程）=="
    ssh $SSHOPTS "$BOARD" '
        [ "$(cat /sys/devices/jailhouse/enabled 2>/dev/null)" = "1" ] || \
            jailhouse enable /usr/share/jailhouse/cells/rpi4.cell
        pkill -x ivshm-console 2>/dev/null || true
        sleep 0.3
    ' >/dev/null 2>&1
    ROOTCELL=$BOARD SSHOPTS=$SSHOPTS ./scripts/setup-ivshm-uio.sh >/dev/null 2>&1 || {
        say "uio 准备失败（跳过 ch7+）"; WITH_SHELL=0;
    }
    if [ "$WITH_SHELL" = 1 ]; then
        scp $SSHOPTS "$DAEMON" "$BOARD:/root/ivshm-console" >/dev/null 2>&1
        ssh $SSHOPTS "$BOARD" '
            chmod +x /root/ivshm-console
            rm -f /root/verify-shell.out /root/verify-shell.err
            setsid nohup sh -c "printf \"help\ncmdline_args a b\ncat_filea\nexit\n\" | /root/ivshm-console -i -d 120" \
                > /root/verify-shell.out 2> /root/verify-shell.err < /dev/null &
        ' >/dev/null 2>&1
    fi
fi

# ---------------------------------------------------------------------------
# 2) 部署 + 抓日志
# ---------------------------------------------------------------------------
say "== [2/3] 部署 cell3 并抓控制台（约 30s）=="
ROOTCELL=$BOARD RELOAD=1 SSHOPTS=$SSHOPTS timeout 300 ./scripts/deploy-cell3.sh > "$LOG" 2>&1
say "   控制台日志: $LOG（$(wc -l < "$LOG") 行）"

if [ "$WITH_SHELL" = 1 ]; then
    sleep 2
    ssh $SSHOPTS "$BOARD" 'cat /root/verify-shell.out 2>/dev/null' > "$SHELL_LOG" 2>/dev/null || true
    ssh $SSHOPTS "$BOARD" 'pkill -x ivshm-console 2>/dev/null || true' >/dev/null 2>&1
    say "   守护进程输出: $SHELL_LOG（$(wc -l < "$SHELL_LOG") 行）"
fi

# ---------------------------------------------------------------------------
# 3) 逐章断言
# ---------------------------------------------------------------------------
FAIL=0
ROWS=""

# row 章节 结果 说明
row() { ROWS="$ROWS$1|$2|$3
"; [ "$2" = "PASS" ] || FAIL=$((FAIL + 1)); }

# has 文件 正则
has() { grep -qE "$2" "$1" 2>/dev/null; }

# 通用：没有 panic
PANIC=OK
has "$LOG" "PANIC|Panicked|panicked at" && PANIC=BAD

need() { # need 文件 正则 描述
    if has "$1" "$2"; then return 0; else return 1; fi
}

# ---- ch1：裸机入口 / 链接加载地址 / UART ----
if need "$LOG" "Started cell" && need "$LOG" "stage-1 MMU 已打开" && \
   need "$LOG" "stext   = 0xfa000000"; then
    row "ch1 裸机入口 + UART" PASS "0xfa000000 启动、MMU 恒等映射、控制台输出完整"
else
    row "ch1 裸机入口 + UART" FAIL "缺少启动/MMU/地址证据"
fi

# ---- ch2：异常向量 + 系统调用 + EL0 app ----
if need "$LOG" "Hello, world from EL0" && need "$LOG" "svc from EL0"; then
    row "ch2 trap/syscall/EL0" PASS "00hello_world 在 EL0 输出、svc 陷入路径正常"
else
    row "ch2 trap/syscall/EL0" FAIL "缺少 EL0 hello 或 svc 日志"
fi

# ---- ch3：yield + 时间片抢占（两个 app 并发交错）----
SEQ=$(grep -oE "\[app[12]\] [AB]" "$LOG" 2>/dev/null | sed 's/.*\] //' | tr -d '\n')
if need "$LOG" "\[ch3\] 并发跑" && need "$LOG" "\[app1\] A0" && \
   need "$LOG" "\[app2\] B0" && need "$LOG" "第 1 次 tick（来自 EL0）"; then
    # 交错：序列里同时出现 AB 和 BA
    case "$SEQ" in
        *AB*BA*|*BA*AB*) row "ch3 任务切换 + 抢占" PASS "01yield/02busy 交错输出（$SEQ）" ;;
        *)               row "ch3 任务切换 + 抢占" FAIL "两个 app 没有交错（$SEQ）" ;;
    esac
else
    row "ch3 任务切换 + 抢占" FAIL "缺少并发任务/tick 证据"
fi

# ---- ch4：每任务页表 + sbrk ----
if need "$LOG" "remap_test passed" && need "$LOG" "\[app3\] 新页可读可写" && \
   need "$LOG" "\[app3\] 跨页读写 OK"; then
    row "ch4 地址空间 + sbrk" PASS "remap_test、sbrk 扩页/跨页读写通过"
else
    row "ch4 地址空间 + sbrk" FAIL "缺少 remap_test 或 sbrk 证据"
fi

# ---- ch5：fork/exec/waitpid ----
if need "$LOG" "\[forktest\] pass" && need "$LOG" "子进程 exec" && need "$LOG" "waitpid 返回"; then
    row "ch5 fork/exec/waitpid" PASS "forktest + initproc 全链路"
else
    row "ch5 fork/exec/waitpid" FAIL "缺少 forktest/exec/waitpid 证据"
fi

# ---- ch6：easy-fs ----
if need "$LOG" "\[filetest\] pass" && need "$LOG" "\[cat\] hello\.txt 共 32 字节"; then
    row "ch6 文件系统" PASS "filetest_simple + cat_filea 读 hello.txt"
else
    row "ch6 文件系统" FAIL "缺少文件系统证据"
fi

# ---- ch7：pipe / dup / 信号 / argv ----
if need "$LOG" "\[pipetest\] passed" && need "$LOG" "\[pipe_large\] passed" && \
   need "$LOG" "校验和 382468" && need "$LOG" "\[sig\] sig_test passed" && \
   need "$LOG" "\[cmdline\] argc = 3"; then
    row "ch7 pipe + 信号 + argv" PASS "pipetest/pipe_large(382468)/sig_test/argv"
else
    row "ch7 pipe + 信号 + argv" FAIL "缺少 pipe/信号/argv 证据"
fi

# ---- ch7+：ivshmem 控制台 + user_shell（root 侧守护进程）----
if [ "$WITH_SHELL" = 1 ] && [ -s "$SHELL_LOG" ]; then
    if need "$SHELL_LOG" "\[shell\] rc-aarch user_shell" && \
       need "$SHELL_LOG" "可用命令" && need "$SHELL_LOG" "\[cmdline\] argc = 3" && \
       need "$SHELL_LOG" "\[cat\] hello\.txt" && need "$SHELL_LOG" "\[shell\] bye"; then
        row "ch7+ ivshmem 控制台" PASS "stdin/stdout + user_shell 交互会话完整"
    else
        row "ch7+ ivshmem 控制台" FAIL "shell 会话不完整（看 $SHELL_LOG）"
    fi
else
    row "ch7+ ivshmem 控制台" SKIP "WITH_SHELL=0 或守护进程起不来"
fi

# ---- ch8：线程 + 同步 ----
if need "$LOG" "thread_test passed" && need "$LOG" "mutex_test passed" && \
   need "$LOG" "sync_test passed" && need "$LOG" "SHARED=4000" && \
   need "$LOG" "信号量验收通过" && need "$LOG" "条件变量验收通过"; then
    row "ch8 线程 + 同步" PASS "thread/mutex/sync 三个 app 全过"
else
    row "ch8 线程 + 同步" FAIL "缺少线程/同步证据"
fi

# ---- ch9：块缓存 ----
RATE=$(grep -oE "命中率 [0-9]+%" "$LOG" 2>/dev/null | tail -1 | tr -dc '0-9')
if need "$LOG" "\[fs_perf\] 200 次" && need "$LOG" "块缓存：命中" && \
   [ -n "$RATE" ] && [ "$RATE" -ge 50 ]; then
    row "ch9 块缓存" PASS "fs_perf 跑通、缓存命中率 ${RATE}%"
else
    row "ch9 块缓存" FAIL "缺少 fs_perf 或命中率不足（${RATE:-?}%）"
fi

# ---- 全局：无 panic + 正常停机 ----
if [ "$PANIC" = OK ] && need "$LOG" "shutdown：initproc exit_code=0"; then
    row "全局" PASS "无 panic，initproc 正常退出、系统停机"
else
    row "全局" FAIL "出现 panic 或没有正常停机"
fi

# ---------------------------------------------------------------------------
# 4) 报表
# ---------------------------------------------------------------------------
say ""
say "================= 逐章验收结果 ================="
printf '%-6s %-26s %s\n' "结果" "章节" "证据"
printf '%-6s %-26s %s\n' "------" "--------------------------" "--------------------------"
printf '%s' "$ROWS" | while IFS='|' read -r ch res why; do
    [ -n "$ch" ] || continue
    printf '%-6s %-26s %s\n' "$res" "$ch" "$why"
done
say "================================================"
if [ "$FAIL" -eq 0 ]; then
    say "✅ 全部通过（日志: $LOG）"
    exit 0
else
    say "❌ 有 $FAIL 项失败（日志: $LOG）"
    exit 1
fi
