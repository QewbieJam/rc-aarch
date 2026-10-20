//! 构建脚本（ch4）—— 对照 rCore ch4 的 `os/build.rs`
//!
//! 职责：
//!   1) 编译 `../user`（aarch64 用户程序）—— ch4 起**所有 app 都链接到 0x10000**
//!   2) 生成 `src/link_app.S`：把每个 app 的 **ELF** 内嵌进内核镜像，
//!      并导出 `_num_app` / `app_N_start` / `app_N_end`
//!
//! ★★ ch4 和 ch3 的两个关键变化：
//!
//!   ① **内嵌 ELF，不再内嵌裸二进制**。
//!      ch3 的 app 是 raw bin，`.bss` 不占文件大小，内核不知道要映射多少页；
//!      开了分页之后必须知道每个段的 `p_memsz`（= 要映射多大）和
//!      `p_filesz`（= 要拷多少），所以改成内嵌 ELF、由内核自己解析。
//!      （upstream ch4 也是从 .bin 改成内嵌 ELF + `MemorySet::from_elf`。）
//!
//!   ② **不再需要"每个 app 一个链接基址"**。
//!      ch3 多个任务共用一套物理地址，必须把 app_i 链到 `0xFA100000 + i*0x20000`；
//!      ch4 每个任务有**自己的页表**，所有 app 都链接到同一个用户 VA（0x10000），
//!      物理页由 frame allocator 各给各的 —— 于是 build.rs 回到"编一次就完事"。

use std::fs::{read_dir, File};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const USER_DIR: &str = "../user";
const USER_BIN_DIR: &str = "../user/src/bin";
const USER_TARGET: &str = "../user/target/aarch64-unknown-none/release";
/// 生成的汇编文件（被 main.rs 的 global_asm! 包含；已加进 .gitignore）
const LINK_APP_S: &str = "src/link_app.S";

fn main() {
    println!("cargo:rerun-if-changed={USER_DIR}/src");
    println!("cargo:rerun-if-changed={USER_DIR}/src/linker.ld");
    println!("cargo:rerun-if-changed=build.rs");

    build_user_apps();
    insert_app_data().expect("生成 link_app.S 失败");
}

/// 编 user crate（一次编完所有 bin）
fn build_user_apps() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());

    // ★ 改了 user/src/linker.ld（比如换链接基址）时，cargo 不会自己重新链接
    //   （它只跟踪源文件，不跟踪 -T 指定的脚本）。这里手动把新的 app 源文件
    //   "碰"一下，强制重编+重链。
    let ld = std::path::Path::new(USER_DIR).join("src/linker.ld");
    let ld_mtime = std::fs::metadata(&ld).and_then(|m| m.modified()).ok();
    for app in app_names() {
        let elf = std::path::Path::new(USER_TARGET).join(&app);
        let need = match (ld_mtime, std::fs::metadata(&elf).and_then(|m| m.modified())) {
            (Some(l), Ok(e)) => l > e,
            _ => true,
        };
        if need {
            let src = format!("{USER_BIN_DIR}/{app}.rs");
            let data = std::fs::read(&src).unwrap();
            std::fs::write(&src, data).unwrap();
        }
    }
    let status = Command::new(&cargo)
        .args(["build", "--release"])
        .current_dir(USER_DIR)
        // 清掉父 cargo 传下来的环境，避免污染子构建
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_MAKEFLAGS")
        .status()
        .expect("无法执行 cargo（编译 user）");
    assert!(status.success(), "user crate 编译失败");
}

/// 读出 `user/src/bin/` 下的 app 名字（去掉 .rs），按名字排序
fn app_names() -> Vec<String> {
    let mut apps: Vec<String> = read_dir(USER_BIN_DIR)
        .expect("找不到 user/src/bin")
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".rs"))
        .map(|n| n.trim_end_matches(".rs").to_string())
        .collect();
    apps.sort();
    apps
}

/// 生成 link_app.S：`_num_app` 表 + 把每个 app 的 ELF 内嵌进 `.data`
fn insert_app_data() -> std::io::Result<()> {
    let apps = app_names();
    let mut f = File::create(LINK_APP_S)?;

    writeln!(f, "/* 本文件由 build.rs 自动生成，请勿手改 */")?;
    writeln!(
        f,
        r#"    .section .data
    .balign 8
    .global _num_app
_num_app:
    .quad {}"#,
        apps.len()
    )?;
    for i in 0..apps.len() {
        writeln!(f, "    .quad app_{i}_start")?;
    }
    writeln!(f, "    .quad app_{}_end", apps.len() - 1)?;

    for (idx, app) in apps.iter().enumerate() {
        let elf = PathBuf::from(format!("{USER_TARGET}/{app}"));
        let abs = std::fs::canonicalize(&elf)
            .unwrap_or_else(|e| panic!("找不到 {}: {e}", elf.display()));
        writeln!(
            f,
            r#"
    .section .data
    .balign 8
    .global app_{idx}_start
    .global app_{idx}_end
app_{idx}_start:
    .incbin "{}"
app_{idx}_end:"#,
            abs.display()
        )?;
        println!("cargo:rerun-if-changed={}", abs.display());
    }
    Ok(())
}
