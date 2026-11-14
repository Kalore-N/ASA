//! 为 Windows 可执行文件嵌入应用图标。
//!
//! 只在交叉编译 MSVC 目标时执行；Linux/macOS 构建不受影响。

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/asa.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let icon = manifest.join("assets").join("asa.ico");
    let rc = out.join("asa.rc");
    let res = out.join("asa.res");
    let obj = out.join("asa-res.obj");

    fs::write(&rc, format!("1 ICON \"{}\"\n", icon.display())).expect("写入资源脚本失败");

    let rc_tool = env::var("RC").unwrap_or_else(|_| "llvm-rc".to_string());
    let status = Command::new(&rc_tool)
        .arg("/fo")
        .arg(&res)
        .arg(&rc)
        .status()
        .unwrap_or_else(|err| panic!("无法运行 {rc_tool}: {err}"));
    assert!(status.success(), "llvm-rc 编译图标失败");

    let machine = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86") => "x86",
        _ => "x64",
    };
    let status = Command::new("llvm-cvtres")
        .arg(format!("/machine:{machine}"))
        .arg(format!("/out:{}", obj.display()))
        .arg(&res)
        .status()
        .expect("无法运行 llvm-cvtres");
    assert!(status.success(), "llvm-cvtres 转换失败");

    println!("cargo:rustc-link-arg-bins={}", obj.display());
}
