//! 用户程序加载（ch5）—— 对照 rCore ch5 的 `os/src/loader.rs`
//!
//! ch5 的 loader 只剩两件事：
//!   ① 从 `link_app.S` 的 `_num_app` 表读"第 i 个 app 的 ELF 在哪"；
//!   ② 从 `_app_names` 表**按名字**找 app —— `sys_exec(path)` 需要它。
//!
//! 内核栈 ch5 起不在这里了：它跟着进程走（每个进程一块，可回收），见 `task/pid.rs`。
//!
//! ★ ELF 解析：对照 upstream 用的 `xmas_elf`，我们手写一个只认 ELF64/PT_LOAD 的
//!   极简版本，只需要 `e_entry`、`p_offset/p_vaddr/p_filesz/p_memsz/p_flags`。

/// 链接器符号 `_num_app`：`[num_app, app0_start, ..., appN_end]`
fn app_table() -> (&'static [usize], usize) {
    unsafe extern "C" {
        safe fn _num_app();
    }
    let num_app_ptr = _num_app as *const () as *const usize;
    let num_app = unsafe { num_app_ptr.read_volatile() };
    let starts = unsafe { core::slice::from_raw_parts(num_app_ptr.add(1), num_app + 1) };
    (starts, num_app)
}

/// app 个数（对照 upstream 的 `get_num_app`）
#[allow(dead_code)]
pub fn get_num_app() -> usize {
    app_table().1
}

/// 第 i 个 app 的 ELF 字节（对照 upstream 的 `get_app_data`）
#[allow(dead_code)]
pub fn get_app_data(app_id: usize) -> &'static [u8] {
    let (starts, num_app) = app_table();
    assert!(app_id < num_app);
    unsafe {
        core::slice::from_raw_parts(
            starts[app_id] as *const u8,
            starts[app_id + 1] - starts[app_id],
        )
    }
}

// ===========================================================================
// 极简 ELF64 解析（对照 upstream 用的 `xmas_elf`）
//
// 只认 ELF64 / little-endian / PT_LOAD，够 ch4/ch5 用。
// 需要的字段：e_entry、e_phoff、e_phentsize、e_phnum；
// 每个 PT_LOAD 的 p_flags / p_offset / p_vaddr / p_filesz / p_memsz。
// ===========================================================================

/// 一个 PT_LOAD 段（字段名和 ELF 规范一致）
#[derive(Copy, Clone, Debug)]
pub struct ProgramHeader {
    pub flags: u32,
    pub offset: usize,
    pub vaddr: usize,
    pub filesz: usize,
    pub memsz: usize,
}

/// 极简 ELF64 解析器
pub struct Elf<'a> {
    data: &'a [u8],
    pub entry: usize,
    phoff: usize,
    phentsize: usize,
    phnum: usize,
}

impl<'a> Elf<'a> {
    pub fn parse(data: &'a [u8]) -> Self {
        assert_eq!(&data[0..4], b"\x7fELF", "不是 ELF 文件！");
        assert_eq!(data[4], 2, "只支持 ELF64");
        let entry = read_u64(data, 0x18) as usize;
        let phoff = read_u64(data, 0x20) as usize;
        let phentsize = read_u16(data, 0x36) as usize;
        let phnum = read_u16(data, 0x38) as usize;
        Self {
            data,
            entry,
            phoff,
            phentsize,
            phnum,
        }
    }

    /// 所有 PT_LOAD 段
    pub fn load_segments(&self) -> alloc::vec::Vec<ProgramHeader> {
        let mut v = alloc::vec::Vec::new();
        for i in 0..self.phnum {
            let base = self.phoff + i * self.phentsize;
            if read_u32(self.data, base) != 1 {
                continue; // 只要 PT_LOAD
            }
            v.push(ProgramHeader {
                flags: read_u32(self.data, base + 0x4),
                offset: read_u64(self.data, base + 0x8) as usize,
                vaddr: read_u64(self.data, base + 0x10) as usize,
                filesz: read_u64(self.data, base + 0x20) as usize,
                memsz: read_u64(self.data, base + 0x28) as usize,
            });
        }
        v
    }
}

fn read_u16(d: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([d[off], d[off + 1]])
}
fn read_u32(d: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]])
}
fn read_u64(d: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        d[off],
        d[off + 1],
        d[off + 2],
        d[off + 3],
        d[off + 4],
        d[off + 5],
        d[off + 6],
        d[off + 7],
    ])
}

// ===========================================================================
// ch5：按名字找 app（`sys_exec` 用）
// ===========================================================================

// 链接器符号 `_app_names`：一串以 '\0' 结尾的 app 名字，顺序和 `_num_app` 一致
// （build.rs 生成；sys_exec 按名字找 app 就靠它）
unsafe extern "C" {
    safe fn _app_names();
}

/// 遍历 `_app_names` 表，找到名字对应的 ELF
pub fn get_app_data_by_name(name: &str) -> Option<&'static [u8]> {
    let (starts, num_app) = app_table();
    let mut p = _app_names as *const () as *const u8;
    for i in 0..num_app {
        // 找这个字符串的结尾
        let mut end = p;
        while unsafe { end.read_volatile() } != 0 {
            end = unsafe { end.add(1) };
        }
        let bytes = unsafe { core::slice::from_raw_parts(p, end as usize - p as usize) };
        if core::str::from_utf8(bytes).ok() == Some(name) {
            return Some(unsafe {
                core::slice::from_raw_parts(
                    starts[i] as *const u8,
                    starts[i + 1] - starts[i],
                )
            });
        }
        p = unsafe { end.add(1) };
    }
    None
}

/// 打印所有内嵌的 app 名字（对照 upstream 的 `list_apps`）
pub fn list_apps() {
    let (_, num_app) = app_table();
    let mut p = _app_names as *const () as *const u8;
    print!("[kernel] apps:");
    for _ in 0..num_app {
        let mut end = p;
        while unsafe { end.read_volatile() } != 0 {
            end = unsafe { end.add(1) };
        }
        let bytes = unsafe { core::slice::from_raw_parts(p, end as usize - p as usize) };
        if let Ok(s) = core::str::from_utf8(bytes) {
            print!(" {}", s);
        }
        p = unsafe { end.add(1) };
    }
    println!("");
}
