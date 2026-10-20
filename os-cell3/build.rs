//! 构建脚本（ch3）—— 对照 rCore ch3 的 `os/build.rs` + `user/build.py`
//!
//! 职责：
//!   1) 编译 `../user`（aarch64 用户程序）—— ★ ch3 起**每个 app 用各自的链接基址**
//!   2) 用 `rust-objcopy` 把每个 app 转成**裸二进制** `.bin`
//!      （内核按字节把它拷到 0xFA100000 + i*0x20000，然后 eret 到 EL0 执行）
//!   3) 生成 `src/link_app.S`：把 `.bin` 内嵌进内核镜像，
//!      并导出 `_num_app` / `app_N_start` / `app_N_end`（loader.rs 靠这些符号找 app）
//!
//! ★★ ch3 和 ch2 最大的区别：**多任务同时驻留 → 每个 app 必须链到自己的地址**
//!   upstream 靠 `user/build.py`：每编一个 app 就把 linker.ld 里的 BASE_ADDRESS
//!   改一次，编完再改回去。我们不改文件，改用链接器的 `--defsym`：
//!
//!       cargo rustc --release --bin <app> -- -Clink-arg=--defsym=__app_base=0xFA120000
//!
//!   linker.ld 里写的是 `BASE_ADDRESS = DEFINED(__app_base) ? __app_base : 0xFA100000`。
//!   好处：一个链接脚本、不用改文件、不会把仓库弄脏。
//!
//!   为什么非这样不可：AArch64 上代码里的函数调用是 PC 相对的，整块搬走还能跑；
//!   但**绝对地址**（start_bss/end_bss、.data 里的指针）不会跟着搬 ——
//!   app_1 会去清 app_0 的 bss，最后莫名其妙跳到 0x0（我们真踩到了）。

use std::fs::{read_dir, File};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const APP_BASE_ADDRESS: usize = 0xFA100000;
const APP_SIZE_LIMIT: usize = 0x20000;

const USER_DIR: &str = "../user";
const USER_BIN_DIR: &str = "../user/src/bin";
const USER_TARGET: &str = "../user/target/aarch64-unknown-none/release";
/// 生成的汇编文件（被 main.rs 的 global_asm! 包含；已加进 .gitignore）
const LINK_APP_S: &str = "src/link_app.S";

fn main() {
    println!("cargo:rerun-if-changed={USER_DIR}/src");
    println!("cargo:rerun-if-changed=build.rs");

    build_user_apps();
    insert_app_data().expect("生成 link_app.S 失败");
}

/// 逐个编 app（每个 app 一个链接基址）+ 逐个 objcopy 成 .bin
fn build_user_apps() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());

    for (idx, app) in app_names().iter().enumerate() {
        // ★ 每个 app 的链接基址 = 加载地址（和 loader.rs 的 get_base_i 必须一致）
        let addr = APP_BASE_ADDRESS + idx * APP_SIZE_LIMIT;
        let link_arg = format!("-Clink-arg=--defsym=__app_base={addr:#x}");

        // cargo 对 `cargo rustc` 的额外参数不一定算进 fingerprint，
        // 为了保证换了基址真的会重新链接，先把源文件 mtime 碰一下。
        touch(&format!("{USER_BIN_DIR}/{app}.rs"));

        let status = Command::new(&cargo)
            .args(["rustc", "--release", "--bin", app, "--", &link_arg])
            .current_dir(USER_DIR)
            // 清掉父 cargo 传下来的环境，避免污染子构建
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_MAKEFLAGS")
            .status()
            .unwrap_or_else(|e| panic!("无法执行 cargo（编译 user/{app}）: {e}"));
        assert!(status.success(), "user/{app} 编译失败");

        println!("[build.rs] app_{idx} = {app} @ {addr:#x}");
    }

    let objcopy = std::env::var("RUST_OBJCOPY").unwrap_or_else(|_| "rust-objcopy".to_string());
    for app in app_names() {
        let elf = format!("{USER_TARGET}/{app}");
        let bin = format!("{USER_TARGET}/{app}.bin");
        let status = Command::new(&objcopy)
            .args([elf.as_str(), "--strip-all", "-O", "binary", bin.as_str()])
            .status()
            .unwrap_or_else(|e| panic!("无法执行 {objcopy}: {e}（需要它在 PATH 里）"));
        assert!(status.success(), "objcopy {app} 失败");
    }
}

/// 碰一下文件 mtime（强制 cargo 重编这一个 bin）
fn touch(path: &str) {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("读不到 {path}: {e}"));
    std::fs::write(path, data).unwrap_or_else(|e| panic!("写不了 {path}: {e}"));
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

/// 生成 link_app.S（对照 rCore 的 `insert_app_data`）
fn insert_app_data() -> std::io::Result<()> {
    let apps = app_names();
    let mut f = File::create(LINK_APP_S)?;

    // _num_app: app 个数 + (num_app+1) 个边界地址
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

    // 每个 app 的原始字节（.incbin 用绝对路径，免得受汇编器 cwd 影响）
    for (idx, app) in apps.iter().enumerate() {
        let bin = PathBuf::from(format!("{USER_TARGET}/{app}.bin"));
        let abs = std::fs::canonicalize(&bin)
            .unwrap_or_else(|e| panic!("找不到 {}: {e}", bin.display()));
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
