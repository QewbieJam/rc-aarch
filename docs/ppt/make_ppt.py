# -*- coding: utf-8 -*-
"""生成 rc-aarch 软件测试课堂展示 PPT（10 页，简洁风格）。
用法: python3 make_ppt.py
输出: rc-aarch-测试展示.pptx
"""
from pptx import Presentation
from pptx.util import Inches, Pt
from pptx.dml.color import RGBColor
from pptx.enum.text import PP_ALIGN, MSO_ANCHOR
from pptx.enum.shapes import MSO_SHAPE
from pptx.oxml.ns import qn
from lxml import etree

CN = "微软雅黑"
ACCENT = RGBColor(0x1F, 0x4E, 0x79)
ACCENT2 = RGBColor(0x2E, 0x75, 0xB6)
TEXT = RGBColor(0x33, 0x33, 0x33)
MUTED = RGBColor(0x80, 0x80, 0x80)
LIGHT = RGBColor(0xF2, 0xF5, 0xF9)
WHITE = RGBColor(0xFF, 0xFF, 0xFF)
GREEN = RGBColor(0x2E, 0x7D, 0x32)
RED = RGBColor(0xC6, 0x28, 0x28)


def set_font(run, size, bold=False, color=TEXT, name=CN):
    f = run.font
    f.size = Pt(size)
    f.bold = bold
    f.color.rgb = color
    f.name = name
    rPr = run._r.get_or_add_rPr()
    ea = rPr.find(qn("a:ea"))
    if ea is None:
        ea = etree.SubElement(rPr, qn("a:ea"))
        latin = rPr.find(qn("a:latin"))
        if latin is not None:
            latin.addnext(ea)
    ea.set("typeface", name)


def textbox(slide, x, y, w, h, align=PP_ALIGN.LEFT, anchor=MSO_ANCHOR.TOP):
    tb = slide.shapes.add_textbox(Inches(x), Inches(y), Inches(w), Inches(h))
    tf = tb.text_frame
    tf.word_wrap = True
    tf.vertical_anchor = anchor
    tf.margin_left = 0
    tf.margin_right = 0
    tf.margin_top = 0
    tf.margin_bottom = 0
    return tf


def add_para(tf, first, runs, size, color=TEXT, bold=False,
             space_before=0, space_after=6, line=1.15, align=PP_ALIGN.LEFT):
    p = tf.paragraphs[0] if first else tf.add_paragraph()
    p.alignment = align
    p.space_before = Pt(space_before)
    p.space_after = Pt(space_after)
    p.line_spacing = line
    for text, b in runs:
        r = p.add_run()
        r.text = text
        set_font(r, size, bold=b, color=color)
    return p


def add_title(slide, title, sub=None):
    tf = textbox(slide, 0.6, 0.35, 12.1, 0.7)
    add_para(tf, True, [(title, True)], 26, ACCENT, bold=True, space_after=0)
    bar = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(0.62), Inches(1.02),
                                 Inches(1.1), Inches(0.055))
    bar.fill.solid()
    bar.fill.fore_color.rgb = ACCENT2
    bar.line.fill.background()
    if sub:
        tf2 = textbox(slide, 0.6, 1.14, 12.1, 0.4)
        add_para(tf2, True, [(sub, False)], 13, MUTED, space_after=0)


def footer(slide, n):
    tf = textbox(slide, 0.6, 7.05, 8, 0.3)
    add_para(tf, True, [("rc-aarch · 软件测试课堂展示", False)], 9, MUTED, space_after=0)
    tf2 = textbox(slide, 12.2, 7.05, 0.6, 0.3, align=PP_ALIGN.RIGHT)
    add_para(tf2, True, [(str(n), False)], 9, MUTED, space_after=0)


def bullets(slide, x, y, w, h, items, size=16, space=10, lead_color=ACCENT):
    tf = textbox(slide, x, y, w, h)
    for i, (lead, rest) in enumerate(items):
        add_para(tf, i == 0,
                 [(lead, True), (rest, False)],
                 size, TEXT, space_after=space, line=1.25)
    return tf


def card(slide, x, y, w, h, head, body, head_size=14, body_size=12):
    box = slide.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, Inches(x), Inches(y),
                                 Inches(w), Inches(h))
    box.adjustments[0] = 0.06
    box.fill.solid()
    box.fill.fore_color.rgb = LIGHT
    box.line.color.rgb = RGBColor(0xDC, 0xE4, 0xEE)
    box.line.width = Pt(0.75)
    tf = box.text_frame
    tf.word_wrap = True
    tf.margin_left = Inches(0.12)
    tf.margin_right = Inches(0.12)
    tf.margin_top = Inches(0.09)
    tf.margin_bottom = Inches(0.06)
    tf.vertical_anchor = MSO_ANCHOR.TOP
    add_para(tf, True, [(head, True)], head_size, ACCENT, space_after=3)
    add_para(tf, False, [(body, False)], body_size, TEXT, space_after=0, line=1.2)


def add_table(slide, x, y, w, h, rows, col_w, font_size=11, header=True):
    shape = slide.shapes.add_table(len(rows), len(rows[0]), Inches(x), Inches(y),
                                   Inches(w), Inches(h))
    table = shape.table
    tbl = table._tbl
    tblPr = tbl.tblPr
    tblPr.set("firstRow", "1" if header else "0")
    tblPr.set("bandRow", "0")
    for e in tblPr.findall(qn("a:tableStyleId")):
        tblPr.remove(e)
    tsid = etree.SubElement(tblPr, qn("a:tableStyleId"))
    tsid.text = "{2D5ABB26-0587-4C30-8999-92F81FD0307C}"  # No Style, No Grid
    for i, cw in enumerate(col_w):
        table.columns[i].width = Inches(cw)
    for r, row in enumerate(rows):
        table.rows[r].height = Inches(h / len(rows))
        for c, val in enumerate(row):
            cell = table.cell(r, c)
            cell.margin_left = Inches(0.09)
            cell.margin_right = Inches(0.06)
            cell.margin_top = Inches(0.02)
            cell.margin_bottom = Inches(0.02)
            cell.vertical_anchor = MSO_ANCHOR.MIDDLE
            if header and r == 0:
                fill = ACCENT
            else:
                fill = WHITE if r % 2 == 1 else LIGHT
            cell.fill.solid()
            cell.fill.fore_color.rgb = fill
            tf = cell.text_frame
            tf.word_wrap = True
            tf.clear()
            p = tf.paragraphs[0]
            p.alignment = PP_ALIGN.LEFT if c == 0 or c == len(row) - 1 and len(val) > 8 else PP_ALIGN.LEFT
            run = p.add_run()
            run.text = str(val)
            if header and r == 0:
                set_font(run, font_size, bold=True, color=WHITE)
            elif str(val) == "PASS":
                set_font(run, font_size, bold=True, color=GREEN)
            elif str(val) == "FAIL":
                set_font(run, font_size, bold=True, color=RED)
            else:
                set_font(run, font_size, bold=False, color=TEXT)
    return table


def arrow(slide, x, y, w, h, text="↓"):
    tf = textbox(slide, x, y, w, h, align=PP_ALIGN.CENTER, anchor=MSO_ANCHOR.MIDDLE)
    add_para(tf, True, [(text, False)], 13, ACCENT2, space_after=0, align=PP_ALIGN.CENTER)


prs = Presentation()
prs.slide_width = Inches(13.333)
prs.slide_height = Inches(7.5)
blank = prs.slide_layouts[6]

# ---------------------------------------------------------------- 1 封面
s = prs.slides.add_slide(blank)
tf = textbox(s, 0.95, 1.85, 11.4, 1.0)
add_para(tf, True, [("rc-aarch", True)], 44, ACCENT, space_after=6)
tf = textbox(s, 0.95, 2.95, 11.4, 1.2)
add_para(tf, True, [("把 rCore 移植到 AArch64，跑进 Jailhouse 隔离分区", False)], 24, TEXT, space_after=0)
bar = s.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(0.98), Inches(4.25), Inches(1.6), Inches(0.06))
bar.fill.solid(); bar.fill.fore_color.rgb = ACCENT2; bar.line.fill.background()
tf = textbox(s, 0.95, 4.55, 11.4, 0.9)
add_para(tf, True, [("软件测试课堂展示", False)], 16, MUTED, space_after=4)
add_para(tf, False, [("树莓派 4B · openEuler MCS · Jailhouse cell3", False)], 14, MUTED, space_after=0)
footer(s, 1)

# ---------------------------------------------------------------- 2 项目介绍
s = prs.slides.add_slide(blank)
add_title(s, "项目介绍", "从教材里的 RISC-V 操作系统，到真实板子上的隔离分区")
tf = textbox(s, 0.6, 1.62, 12.1, 0.75)
add_para(tf, True, [("用 AArch64 重写 rCore-Tutorial 的 9 章迷你操作系统，并在树莓派 4B 的 Jailhouse 非根 cell 里跑通。", False)],
         16, TEXT, space_after=0, line=1.25)
card(s, 0.6, 2.55, 3.85, 2.35, "从教材到真机",
     "rCore 是 RISC-V 的教材实现；本项目逐章移植到 AArch64，代码、文档、验收 app 一一对应。")
card(s, 4.72, 2.55, 3.85, 2.35, "隔离环境",
     "运行在树莓派 4B 的 Jailhouse 非根 cell 中，与主系统 Linux 共存，互不影响。")
card(s, 8.84, 2.55, 3.85, 2.35, "可测、可演示",
     "每章都有验收 app 和日志；一条命令跑完 ch1–ch9，结果自动给出 PASS/FAIL。")
tf = textbox(s, 0.6, 5.35, 12.1, 0.9)
add_para(tf, True, [("规模：", True), ("内核约 6 千行 Rust；9 个章节、17 个用户态测试程序；11 项逐章验收。", False)],
         14, TEXT, space_after=0)
footer(s, 2)

# ---------------------------------------------------------------- 3 能力全景
s = prs.slides.add_slide(blank)
add_title(s, "系统能力全景", "9 个章节 → 6 项能力；本页只做全景，后面深挖 1 项、用测试数据覆盖其余")
items = [
    ("启动与异常（ch1–ch2）", "裸机入口、异常向量表、系统调用、EL0 用户态"),
    ("进程与地址空间（ch3–ch5）", "任务切换与抢占、每任务页表、fork / exec / waitpid"),
    ("文件系统（ch6）", "easy-fs、RAM 磁盘、open / read / write、块缓存"),
    ("IPC 与信号（ch7）", "管道 pipe / dup、阻塞读写、信号、exec 传参"),
    ("线程与同步（ch8）", "线程创建与等待、mutex、semaphore、condvar"),
    ("跨 cell 共享内存（ch7+）", "共享内存字节环、stdin / stdout、交互 shell"),
]
for i, (head, body) in enumerate(items):
    x = 0.6 + (i % 3) * 4.12
    y = 1.75 + (i // 3) * 2.0
    card(s, x, y, 3.85, 1.75, head, body, head_size=13.5, body_size=11.5)
tf = textbox(s, 0.6, 5.95, 12.1, 0.6)
add_para(tf, True, [("9 章里大部分是“内核机制”，很难在 10 分钟里逐章讲；", False),
                    ("所以用逐章测试的结果来证明它们都在，只把共享内存控制台展开讲。", True)],
         13, MUTED, space_after=0)
footer(s, 3)

# ---------------------------------------------------------------- 4 深挖：共享内存控制台
s = prs.slides.add_slide(blank)
add_title(s, "功能深挖：共享内存控制台", "在 ssh 里敲命令，让隔离分区里的 shell 执行并回传输出")
bullets(s, 0.6, 1.7, 6.7, 4.6, [
    ("问题：", "板子的串口被主系统占用，虚拟机没有输入通道，只能输出。"),
    ("方案：", "主系统侧一个“传声筒”守护进程，和虚拟机通过共享内存通信：一块两边都能读写的内存，加一个字节环（写指针 / 读指针 / 代数）。"),
    ("效果：", "在 ssh 里敲命令 → 送进虚拟机 → 它的 shell 执行 → 输出原路回传。"),
    ("边界：", "这不是网络协议栈，只是共享内存上的读写约定；主系统用 uio 访问，虚拟机侧就是普通内存读写。"),
], size=15, space=12)
# 右侧流程图
boxes = [
    ("你的电脑（ssh 终端）", 1.55),
    ("树莓派主系统：传声筒守护进程", 2.85),
    ("共享内存：信箱 + 字节环", 4.15),
    ("虚拟机：shell / 应用", 5.45),
]
for text, y in boxes:
    b = s.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, Inches(7.55), Inches(y),
                           Inches(4.95), Inches(0.72))
    b.adjustments[0] = 0.12
    b.fill.solid(); b.fill.fore_color.rgb = LIGHT
    b.line.color.rgb = RGBColor(0xDC, 0xE4, 0xEE); b.line.width = Pt(0.75)
    tf = b.text_frame; tf.word_wrap = True
    tf.vertical_anchor = MSO_ANCHOR.MIDDLE
    tf.margin_top = 0; tf.margin_bottom = 0
    add_para(tf, True, [(text, False)], 12.5, TEXT, space_after=0, align=PP_ALIGN.CENTER)
for y in (2.27, 3.57, 4.87):
    arrow(s, 9.15, y, 1.7, 0.45, "⇅" if y != 2.27 else "↓")
footer(s, 4)

# ---------------------------------------------------------------- 5 测试体系
s = prs.slides.add_slide(blank)
add_title(s, "测试体系", "真机、自动化、可追溯、可回归")
bullets(s, 0.7, 1.65, 11.9, 4.9, [
    ("真机测试：", "所有用例都在树莓派 + Jailhouse 上跑，不用模拟器 —— 避免“模拟器上过、真机不过”。"),
    ("自动化：", "一条命令 ./scripts/verify-chapters.sh：启动虚拟机，按 ch1→ch9 逐项断言，输出 PASS/FAIL 表。"),
    ("可追溯：", "控制台日志 + 守护进程日志都落盘；失败能定位到具体章节和具体 app。"),
    ("回归：", "每次改动重跑全部章节（含旧章节）；本次 11 项断言全部通过。"),
], size=17, space=18)
footer(s, 5)

# ---------------------------------------------------------------- 6 测试用例
s = prs.slides.add_slide(blank)
add_title(s, "测试用例（代表）", "挑 4 个有数字、可复核的用例；其余见验收总表")
rows = [
    ["用例", "验证点", "预期 / 实际", "结果"],
    ["pipe_large_test", "32 字节管道传 3000 字节；写满阻塞、读空唤醒", "校验和 382468", "PASS"],
    ["mutex_test", "4 个线程各 1000 次加锁自增", "计数 = 4000", "PASS"],
    ["fs_perf + 缓存统计", "200 次 open + read；块缓存命中率", "命中率 83%", "PASS"],
    ["03sbrk", "sbrk 扩页、跨页读写、非法缩回失败", "全部符合预期", "PASS"],
]
add_table(s, 0.6, 1.65, 12.1, 3.4, rows, [2.5, 5.4, 2.5, 1.7], font_size=12)
tf = textbox(s, 0.6, 5.5, 12.1, 1.0)
add_para(tf, True, [("另有用例覆盖：", True),
                    ("信号处理与屏蔽、fork/exec/waitpid、argv 传参、文件读写、线程创建与等待、块缓存淘汰。", False)],
         13, MUTED, space_after=0)
footer(s, 6)

# ---------------------------------------------------------------- 7 验收总表
s = prs.slides.add_slide(blank)
add_title(s, "验收结果：11 / 11 通过", "在真机上重新跑一遍后的结果（脚本自动断言）")
rows = [
    ["章节", "结果", "主要证据"],
    ["ch1 裸机入口 + UART", "PASS", "0xfa000000 启动、MMU 恒等映射、控制台输出完整"],
    ["ch2 trap / syscall / EL0", "PASS", "EL0 打印 hello、svc 陷入路径正常"],
    ["ch3 任务切换 + 抢占", "PASS", "两个 app 交错输出、定时器 tick 来自 EL0"],
    ["ch4 地址空间 + sbrk", "PASS", "remap_test、扩页与跨页读写通过"],
    ["ch5 fork / exec / waitpid", "PASS", "forktest 全链路、子进程回收"],
    ["ch6 文件系统", "PASS", "filetest、cat 读到 hello.txt"],
    ["ch7 pipe + 信号 + argv", "PASS", "校验和 382468、sig_test、argv 正确"],
    ["ch7+ 共享内存控制台", "PASS", "shell 交互会话完整（stdin/stdout）"],
    ["ch8 线程 + 同步", "PASS", "thread / mutex / sync 三个 app 全过"],
    ["ch9 块缓存", "PASS", "fs_perf 跑通、命中率 83%"],
    ["全局", "PASS", "无 panic、initproc 正常退出、系统停机"],
]
add_table(s, 0.6, 1.55, 12.1, 5.1, rows, [3.4, 1.0, 7.7], font_size=10.5)
footer(s, 7)

# ---------------------------------------------------------------- 8 测试发现的缺陷
s = prs.slides.add_slide(blank)
add_title(s, "测试发现的典型缺陷", "4 个“看起来能跑、其实悄悄错”的问题")
rows = [
    ["现象", "根因", "修复"],
    ["同一地址换了程序，偶发执行旧指令", "AArch64 改地址空间后没有失效 I-cache", "切换页表时执行 ic iallu"],
    ["管道数据整片错，父子校验和却相同（假通过）", "内核用 SIMD 把用户寄存器现场写坏", "TrapContext 保存 / 恢复 q0–q31"],
    ["新 shell 一启动就退出、命令错乱", "共享内存残留输入；读下标写错段", "启动清环；下标写自己段"],
    ["ch8 启动即 panic“进程已回收”", "进程控制块的强引用提前归零", "创建返回前先登记"],
]
add_table(s, 0.6, 1.65, 12.1, 3.6, rows, [4.3, 4.6, 3.2], font_size=11.5)
tf = textbox(s, 0.6, 5.55, 12.1, 1.1)
add_para(tf, True, [("教训：", True),
                    ("只比较“两边各自算出的摘要”，当两边被同一个 bug 写坏时会假通过；"
                     "跨特权级、跨分区的用例要直接比内容。", False)],
         13, TEXT, space_after=0, line=1.3)
footer(s, 8)

# ---------------------------------------------------------------- 9 扩展性 & 集成性
s = prs.slides.add_slide(blank)
add_title(s, "扩展性与集成性", "不是一个封闭的演示，而是一套可以继续长的基础")
card(s, 0.6, 1.7, 5.95, 4.9, "扩展性",
     "· 章节可插拔：ch1–ch9 各自分支与文档，新增一章不影响旧章节\n"
     "· 通道可扩展：共享内存除了 stdin/stdout，还能承载日志、数据采集、控制命令\n"
     "· 块设备可替换：RAM 盘 → SD 卡 / eMMC，接口不变（BlockDevice）\n"
     "· 继续加：更多线程原语、更多 syscall、多核", head_size=16, body_size=13)
card(s, 6.75, 1.7, 5.95, 4.9, "集成性",
     "· 隔离性：虚拟机里的故障（panic、死循环）不影响板子的主系统\n"
     "· 与主系统共存：同一块板子、同一套网络和 ssh，随用随起\n"
     "· 通用通道：主系统用 uio 访问共享内存，虚拟机侧就是普通内存读写\n"
     "· 演示友好：一条命令部署，一条命令验收", head_size=16, body_size=13)
footer(s, 9)

# ---------------------------------------------------------------- 10 总结
s = prs.slides.add_slide(blank)
add_title(s, "总结")
bullets(s, 0.7, 1.6, 11.9, 3.4, [
    ("9 章全部跑通：", "从裸机 hello 到线程、同步、块缓存，都在真机上验证。"),
    ("测试驱动：", "逐章验收脚本，11 项断言全过；测试过程中发现并修掉 4 类典型缺陷。"),
    ("可复现：", "仓库含 11 篇对照文档；部署和验收各一条命令。"),
], size=17, space=16)
tf = textbox(s, 0.7, 5.25, 11.9, 1.2)
add_para(tf, True, [("仓库：", True), ("github.com/QewbieJaw/rc-aarch　｜　atomgit.com/EraFei/rc-aarch", False)],
         13, MUTED, space_after=4)
add_para(tf, False, [("文档：", True), ("docs/01 – docs/11（逐章对照 + 逐章验收）", False)],
         13, MUTED, space_after=0)
tf = textbox(s, 0.7, 6.35, 11.9, 0.6)
add_para(tf, True, [("谢谢，欢迎提问。", False)], 18, ACCENT, space_after=0)
footer(s, 10)

# ---------------------------------------------------------------- 讲稿备注（每页约 1 分钟）
NOTES = [
    "开场 20 秒：我叫…，这次展示的项目是 rc-aarch：把 rCore 教材里的 9 章迷你操作系统，用 AArch64 重写，并跑进树莓派上的 Jailhouse 隔离分区。今天重点讲测试。",
    "约 1 分钟：项目是什么、为什么做。一句话：教材是 RISC-V 的，我们把它逐章移植到 AArch64，跑在真实的树莓派隔离分区里，而不是模拟器。规模：约 6 千行 Rust，17 个测试程序。",
    "约 1 分钟：9 章压成 6 项能力。不用逐章讲，后面只深挖共享内存控制台这一项，其余能力用测试结果证明。",
    "约 1 分半：这是本项目最有特色的一项。串口被主系统占着，虚拟机没有输入；我们在主系统放一个传声筒程序，通过共享内存和虚拟机通信，于是 ssh 里就能操作虚拟机的 shell。现场可以演示 cat_filea / cmdline_args / forktest。",
    "约 1 分钟：测试体系四个关键词：真机、自动化、可追溯、可回归。一条命令跑完 ch1 到 ch9，输出 PASS/FAIL 表。",
    "约 1 分半：挑 4 个有数字的用例：32 字节管道传 3000 字节、4 线程各 1000 次自增等于 4000、块缓存命中率 83%、sbrk 扩页。每个都能在日志里复核。",
    "约 1 分钟：11 项断言全部通过，这是最近一次在真机上重跑的结果。",
    "约 1 分半：软件测试课的重点——测试抓到了什么。四个“看起来能跑、其实悄悄错”的缺陷，每个讲一句现象、根因、修法。特别强调第二个：只比校验和会假通过。",
    "约 50 秒：扩展性和集成性。隔离性、通道通用、块设备可换，后续可以继续加线程原语和多核。",
    "约 30 秒：总结三句话，给出仓库和文档，谢谢。",
]
for i, slide in enumerate(prs.slides):
    slide.notes_slide.notes_text_frame.text = NOTES[i]

out = "rc-aarch-测试展示.pptx"
prs.save(out)
print("saved:", out)
