# -*- coding: utf-8 -*-
"""生成《rc-aarch 测试模块清单与日志术语说明》Word 文档。"""
from docx import Document
from docx.shared import Pt, Cm, RGBColor
from docx.enum.text import WD_ALIGN_PARAGRAPH, WD_LINE_SPACING
from docx.enum.table import WD_TABLE_ALIGNMENT
from docx.oxml.ns import qn
from docx.oxml import OxmlElement

HEI = "黑体"
SONG = "宋体"
MONO = "Consolas"
ACCENT = RGBColor(0x1F, 0x4E, 0x79)
TEXT = RGBColor(0x22, 0x22, 0x22)
GRAY = RGBColor(0x66, 0x66, 0x66)


def set_run(run, name=SONG, size=10.5, bold=False, color=TEXT):
    run.font.name = name
    run.font.size = Pt(size)
    run.font.bold = bold
    run.font.color.rgb = color
    rPr = run._element.get_or_add_rPr()
    rFonts = rPr.find(qn("w:rFonts"))
    if rFonts is None:
        rFonts = OxmlElement("w:rFonts")
        rPr.append(rFonts)
    rFonts.set(qn("w:ascii"), name)
    rFonts.set(qn("w:hAnsi"), name)
    rFonts.set(qn("w:eastAsia"), name)


def para(doc, text="", name=SONG, size=10.5, bold=False, color=TEXT,
         align=WD_ALIGN_PARAGRAPH.LEFT, space_before=0, space_after=6, line=1.35):
    p = doc.add_paragraph()
    p.alignment = align
    p.paragraph_format.space_before = Pt(space_before)
    p.paragraph_format.space_after = Pt(space_after)
    p.paragraph_format.line_spacing = line
    if text:
        r = p.add_run(text)
        set_run(r, name, size, bold, color)
    return p


def heading(doc, text, level=1):
    size = 15 if level == 1 else 12.5
    p = doc.add_paragraph()
    p.paragraph_format.space_before = Pt(14 if level == 1 else 10)
    p.paragraph_format.space_after = Pt(6)
    r = p.add_run(text)
    set_run(r, HEI, size, True, ACCENT)
    # 标题下细线
    if level == 1:
        pPr = p._p.get_or_add_pPr()
        pbdr = OxmlElement("w:pBdr")
        bottom = OxmlElement("w:bottom")
        bottom.set(qn("w:val"), "single")
        bottom.set(qn("w:sz"), "6")
        bottom.set(qn("w:space"), "2")
        bottom.set(qn("w:color"), "2E75B6")
        pbdr.append(bottom)
        pPr.append(pbdr)
    return p


def shade(cell, hexfill):
    tcPr = cell._tc.get_or_add_tcPr()
    shd = OxmlElement("w:shd")
    shd.set(qn("w:val"), "clear")
    shd.set(qn("w:color"), "auto")
    shd.set(qn("w:fill"), hexfill)
    tcPr.append(shd)


def table(doc, rows, widths, header=True, size=9, mono_cols=()):
    t = doc.add_table(rows=len(rows), cols=len(rows[0]))
    t.style = "Table Grid"
    t.alignment = WD_TABLE_ALIGNMENT.CENTER
    t.autofit = False
    for i, w in enumerate(widths):
        for row in t.rows:
            row.cells[i].width = Cm(w)
    for r, row in enumerate(rows):
        for c, val in enumerate(row):
            cell = t.cell(r, c)
            cell.paragraphs[0].text = ""
            cell.paragraphs[0].paragraph_format.space_before = Pt(1)
            cell.paragraphs[0].paragraph_format.space_after = Pt(1)
            cell.paragraphs[0].paragraph_format.line_spacing = 1.1
            run = cell.paragraphs[0].add_run(str(val))
            if header and r == 0:
                set_run(run, HEI, size, True, RGBColor(0xFF, 0xFF, 0xFF))
                shade(cell, "1F4E79")
            else:
                name = MONO if c in mono_cols else SONG
                set_run(run, name, size, False, TEXT)
                if r % 2 == 0:
                    shade(cell, "F2F5F9")
    # 表后留白
    para(doc, "", space_after=2, line=1.0)
    return t


def code_block(doc, lines):
    p = doc.add_paragraph()
    p.paragraph_format.space_before = Pt(2)
    p.paragraph_format.space_after = Pt(6)
    p.paragraph_format.line_spacing = 1.2
    pPr = p._p.get_or_add_pPr()
    shd = OxmlElement("w:shd")
    shd.set(qn("w:val"), "clear")
    shd.set(qn("w:color"), "auto")
    shd.set(qn("w:fill"), "F4F6F8")
    pPr.append(shd)
    for i, line in enumerate(lines):
        if i:
            p.add_run("\n")
        r = p.add_run(line)
        set_run(r, MONO, 9, False, RGBColor(0x33, 0x33, 0x33))


def bullet(doc, text, bold_prefix=None):
    p = doc.add_paragraph(style="List Bullet")
    p.paragraph_format.space_after = Pt(3)
    p.paragraph_format.line_spacing = 1.3
    if bold_prefix:
        r = p.add_run(bold_prefix)
        set_run(r, HEI, 10.5, True, TEXT)
    r = p.add_run(text)
    set_run(r, SONG, 10.5, False, TEXT)
    return p


def add_page_number(section):
    p = section.footer.paragraphs[0]
    p.alignment = WD_ALIGN_PARAGRAPH.CENTER
    run = p.add_run()
    fld1 = OxmlElement("w:fldChar"); fld1.set(qn("w:fldCharType"), "begin")
    instr = OxmlElement("w:instrText"); instr.set(qn("xml:space"), "preserve")
    instr.text = " PAGE "
    fld2 = OxmlElement("w:fldChar"); fld2.set(qn("w:fldCharType"), "end")
    run._r.append(fld1); run._r.append(instr); run._r.append(fld2)
    set_run(run, SONG, 9, False, GRAY)
    # 页脚说明
    p2 = section.footer.add_paragraph()
    p2.alignment = WD_ALIGN_PARAGRAPH.CENTER
    r = p2.add_run("rc-aarch · 测试模块清单与日志术语说明")
    set_run(r, SONG, 8, False, GRAY)


doc = Document()
sec = doc.sections[0]
sec.page_width = Cm(21.0)
sec.page_height = Cm(29.7)
sec.top_margin = Cm(2.2)
sec.bottom_margin = Cm(2.2)
sec.left_margin = Cm(2.2)
sec.right_margin = Cm(2.2)

# 正文默认样式
style = doc.styles["Normal"]
style.font.name = SONG
style.font.size = Pt(10.5)
style.element.rPr.rFonts.set(qn("w:eastAsia"), SONG)

add_page_number(sec)

# ---------------- 标题 ----------------
para(doc, "rc-aarch 测试模块清单与日志术语说明", HEI, 19, True, ACCENT,
     WD_ALIGN_PARAGRAPH.CENTER, 0, 4)
para(doc, "软件测试课堂展示 · 真机（树莓派 4B / Jailhouse cell3）", SONG, 10.5, False, GRAY,
     WD_ALIGN_PARAGRAPH.CENTER, 0, 2)
para(doc, "2026-11", SONG, 10, False, GRAY, WD_ALIGN_PARAGRAPH.CENTER, 0, 14)

para(doc, "本文把 verify-chapters.log 里的输出按“测试模块”重新归类，并解释 [trap] 日志行里"
          "每个专用名词的含义，方便答辩时对照日志讲。")

# ---------------- 一 ----------------
heading(doc, "一、启动自检（单元 / 组件级）")
para(doc, "这三项在内核启动时自己跑，属于单元/组件级；失败会直接 panic，不会继续往下执行。")
table(doc, [
    ["模块", "日志里的行", "验证什么"],
    ["页表权限自检", "[mm] remap_test passed", "内核页权限：.text 只读可执行、.data 可写不可执行"],
    ["内核堆自检", "[mm] heap_test passed", "空闲链表分配器：分配/释放/合并、反复分配不泄漏"],
    ["文件系统初始化", "[fs] 写入 /xxx + 挂载成功", "easy-fs 建盘、写文件、重新挂载校验"],
], [3.0, 5.4, 8.2], mono_cols=(1,))

# ---------------- 二 ----------------
heading(doc, "二、initproc 调度的验收 app（集成 / 系统级）")
para(doc, "日志里每一次“[initproc] 子进程 exec X”就是一个测试模块；跑完看 exit_code=0 判通过。")
table(doc, [
    ["#", "app", "对应章节", "验证什么", "日志特征"],
    ["1", "00hello_world", "ch2", "EL0 用户态 + write/exit 系统调用", "Hello, world from EL0"],
    ["2", "conc_test", "ch3", "同时跑两个 app，验证 yield 与时间片抢占", "[app1] A0 和 [app2] B0 交错"],
    ["3", "01yield / 02busy", "ch3", "被 conc_test 用 exec 拉起来的两个被测对象", "[app1] A… / [app2] B…"],
    ["4", "03sbrk", "ch4", "sbrk 扩页、跨页读写、非法缩回", "[app3] …"],
    ["5", "filetest_simple", "ch6", "文件创建 / 写 / 读 / 截断", "[filetest] pass"],
    ["6", "cat_filea", "ch6", "读 hello.txt", "[cat] hello.txt 共 32 字节"],
    ["7", "pipetest", "ch7", "管道 + dup，小数据", "[pipetest] passed"],
    ["8", "pipe_large_test", "ch7", "32 字节环传 3000 字节，阻塞/唤醒，父子校验和", "校验和 382468"],
    ["9", "simd_test", "ch7+ 回归", "SIMD 现场跨系统调用、自动向量化结果", "[simd] ② ③"],
    ["10", "sig_test", "ch7", "信号：handler/sigreturn、屏蔽、非法信号、SIGKILL", "[sig] ①②③④"],
    ["11", "cmdline_args", "ch7", "exec 传 argv", "[cmdline] argc = 3"],
    ["12", "forktest", "ch5", "fork 5 个子进程、exit code、waitpid 回收", "[forktest] pass"],
    ["13", "thread_test", "ch8", "线程创建 / 参数 / tid / waittid、共享变量", "[thread] thread_test passed"],
    ["14", "mutex_test", "ch8", "互斥锁，4 线程 × 1000 次", "SHARED=4000"],
    ["15", "sync_test", "ch8", "信号量阻塞/放行、条件变量等待/唤醒", "[sem] / [cond] 两段"],
    ["16", "fs_perf", "ch9", "文件读写耗时 + 块缓存统计", "[fs_perf] 两行"],
    ["17", "user_shell", "ch7+", "共享内存控制台 + shell 交互（stdin/stdout）", ">> + [shell] bye"],
], [0.9, 2.9, 1.8, 6.8, 4.2], size=8.5)

# ---------------- 三 ----------------
heading(doc, "三、测试框架本身（不在 app 里，但由它驱动）")
table(doc, [
    ["模块", "作用"],
    ["initproc", "测试调度器：按顺序 fork 每个 app、waitpid、收集 exit_code"],
    ["scripts/verify-chapters.sh", "真机测试入口：起虚拟机、抓日志、逐章断言、打 PASS/FAIL 表"],
    ["共享内存通道（[shm]）", "把虚拟机输出回传到主系统（Stdout 的 tee）"],
    ["停机统计", "页帧、内核堆、块缓存命中率（块缓存：命中 … 83%）"],
], [5.0, 11.6], mono_cols=(1,))

# ---------------- 四 ----------------
heading(doc, "四、日志里的“噪声”是什么")
bullet(doc, "每一次系统调用的现场，是证据不是错误（说明 trap 路径在跑）。",
       "[trap] EC=0x15 … svc from EL0：")
bullet(doc, "定时器中断，ch3 抢占的证据。", "[irq] 第 1 次 tick：")
bullet(doc, "多进程并发写同一个 UART 属于正常现象；校验和一致就说明数据没错。",
       "输出交错（如 [pipe_large] 父进程发出 与 子进程收到 混在一起）：")

# ---------------- 五 ----------------
heading(doc, "五、[trap] 行里的专用名词到底是什么")
para(doc, "先看日志里真实的两行：")
code_block(doc, [
    "[trap] EC=0x15 ESR=0x0000000056000000 ELR=0x10184 FAR=0x0 SPSR=0x80000340 (from EL0)",
    "[trap]   -> svc from EL0: id=172 返回 1 (写回 x0)",
])
para(doc, "一行一条异常记录。把它拆成字段：")
table(doc, [
    ["字段", "英文全称", "大白话", "本例取值"],
    ["EC", "Exception Class", "异常类型编号（取自 ESR 高 6 位）", "0x15 = SVC64，即 AArch64 的系统调用"],
    ["ESR", "Exception Syndrome Register", "异常的“病历”：原因 + 附加信息", "0x56000000；0x56 << 26 正好是 0x15"],
    ["ELR", "Exception Link Register", "异常返回地址：处理完 eret 回到这里", "0x10184（svc 的下一条指令）"],
    ["FAR", "Fault Address Register", "出错的内存地址；只有访存异常才有意义", "0x0（SVC 用不到）"],
    ["SPSR", "Saved Program Status Register", "异常发生前的 CPU 状态：在哪个特权级、中断是否屏蔽", "0x80000340；低 4 位 0 = EL0"],
    ["from EL0", "—", "这次异常来自用户态", "用户程序执行了 svc"],
], [2.0, 3.6, 6.0, 5.0], size=8.5, mono_cols=(0, 3))

para(doc, "后半行里的词：")
table(doc, [
    ["词", "大白话"],
    ["svc #0", "一条指令：“请求系统调用”，从用户态进入内核"],
    ["x8", "存放“调用号”的寄存器（本例 172 = getpid）"],
    ["x0", "第一个参数；系统调用的返回值也放在这里"],
    ["id=172", "这次调用的编号"],
    ["返回 1", "系统调用处理完的返回值，被写回用户寄存器 x0"],
], [2.8, 13.8], mono_cols=(0,))

para(doc, "常见 EC 值（本项目的 trap 日志主要出现 0x15；其余供对照）：")
table(doc, [
    ["EC", "含义", "什么时候出现"],
    ["0x15", "SVC64：AArch64 系统调用", "用户程序执行 svc（本项目每次系统调用）"],
    ["0x18", "SYS64：特权寄存器访问被拦", "访问了不该碰的系统寄存器"],
    ["0x00", "未定义指令（Unknown）", "取到坏指令、I-cache 陈旧等"],
    ["0x20 / 0x21", "取指异常（IABT，来自 lower / current EL）", "跳到了没映射或无权限的地址"],
    ["0x24 / 0x25", "数据访问异常（DABT，来自 lower / current EL）", "读写没映射或无权限的地址"],
    ["0x16", "HVC64：给 hypervisor 的调用", "EL0 执行 hvc（非法）"],
], [2.8, 7.2, 6.6], mono_cols=(0,))

para(doc, "SPSR 那串数字怎么看（本例 0x80000340）：")
bullet(doc, "低 4 位 M[3:0] = 0 → EL0t：用户态，用 SP_EL0 作栈指针。", "")
bullet(doc, "bit9 D、bit8 A、bit7 I、bit6 F：0x340 表示 D/A/F 被屏蔽、I = 0（放行 IRQ），"
            "所以定时器能打断用户程序。", "")
bullet(doc, "bit31 = 1 是 N 条件标志，用户代码留下的，和特权级无关。", "")

# ---------------- 六 ----------------
heading(doc, "六、怎么快速自查日志")
code_block(doc, [
    "grep -E \"remap_test|heap_test\" verify-chapters.log   # 启动自检",
    "grep -E \"passed|pass\\.\" verify-chapters.log          # 所有通过点",
    "grep \"子进程 exec\" verify-chapters.log                # 测试模块清单",
    "grep \"块缓存\" verify-chapters.log                     # ch9 结果",
    "grep \"EC=0x\" verify-chapters.log | head               # trap 现场（取样）",
])

doc.save("rc-aarch-测试模块清单与日志术语.docx")
print("saved")
