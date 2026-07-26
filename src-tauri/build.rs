use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    prepare_platform_frpc();
    tauri_build::build();
}

fn prepare_platform_frpc() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR 未设置"));
    let target = env::var("TARGET").expect("TARGET 未设置");
    let target_os = parse_target_os(&target).expect("不支持的目标系统");
    let target_arch = parse_target_arch(&target).expect("不支持的目标架构");
    let (arch_dir, output_name) = frpc_layout(target_os, target_arch).expect("不支持的目标平台组合");
    let source_file = manifest_dir
        .join("frp-binaries")
        .join(target_os)
        .join(arch_dir)
        .join(output_name);

    if !source_file.exists() {
        panic!(
            "未找到打包所需的 frpc 文件。\n目标平台: {} {}\n请将文件放到: {}",
            target_os,
            target_arch,
            source_file.display()
        );
    }

    let resources_dir = manifest_dir.join("resources");
    fs::create_dir_all(&resources_dir).expect("创建 resources 目录失败");

    let old_unix = resources_dir.join("frpc");
    if old_unix.exists() {
        fs::remove_file(&old_unix).expect("删除旧 frpc 文件失败");
    }

    let old_windows = resources_dir.join("frpc.exe");
    if old_windows.exists() {
        fs::remove_file(&old_windows).expect("删除旧 frpc.exe 文件失败");
    }

    let output_file = resources_dir.join(output_name);
    fs::copy(&source_file, &output_file).expect("复制 frpc 文件到 resources 失败");
    sign_macos_resource_if_needed(target_os, &output_file);

    println!("cargo:rerun-if-env-changed=TARGET");
    println!("cargo:rerun-if-env-changed=APPLE_SIGNING_IDENTITY");
    println!("cargo:rerun-if-changed={}", source_file.display());
}

fn parse_target_os(target: &str) -> Option<&'static str> {
    if target.contains("windows") {
        Some("windows")
    } else if target.contains("apple-darwin") {
        Some("macos")
    } else if target.contains("linux") {
        Some("linux")
    } else {
        None
    }
}

fn parse_target_arch(target: &str) -> Option<&'static str> {
    match target.split('-').next()? {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        _ => None,
    }
}

fn frpc_layout(target_os: &str, target_arch: &str) -> Option<(&'static str, &'static str)> {
    match (target_os, target_arch) {
        ("windows", "x86_64") => Some(("x64", "frpc.exe")),
        ("linux", "x86_64") => Some(("amd64", "frpc")),
        ("macos", "aarch64") => Some(("aarch64", "frpc")),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn sign_macos_resource_if_needed(target_os: &str, binary_path: &Path) {
    if target_os != "macos" {
        return;
    }

    let identity = match env::var("APPLE_SIGNING_IDENTITY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            println!(
                "cargo:warning=APPLE_SIGNING_IDENTITY 未设置，跳过资源 {} 的 codesign",
                binary_path.display()
            );
            return;
        }
    };

    let output = Command::new("codesign")
        .args([
            "--force",
            "--sign",
            identity.as_str(),
            "--timestamp",
            "--options",
            "runtime",
            binary_path.to_string_lossy().as_ref(),
        ])
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "执行 codesign 失败，无法签名资源 {}: {}",
                binary_path.display(),
                error
            )
        });

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "codesign 签名资源 {} 失败。\nstdout:\n{}\nstderr:\n{}",
            binary_path.display(),
            stdout.trim(),
            stderr.trim()
        );
    }

    println!(
        "cargo:warning=已对资源 {} 完成 macOS codesign",
        binary_path.display()
    );
}

#[cfg(not(target_os = "macos"))]
fn sign_macos_resource_if_needed(_target_os: &str, _binary_path: &Path) {}
