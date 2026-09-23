//! Tauri 构建脚本。
//!
//! 两件事：
//! 1. 把当前 git 提交号注入编译期环境变量 `FORGEDESK_GIT_SHA`，
//!    供 `app_version()` 命令回报（便于用户提 Issue 时定位到具体构建）。
//!    没有 git 或不在仓库中时降级为 "unknown"，**不得**让构建失败。
//! 2. 调用 `tauri_build::build()`，它会处理 `tauri.conf.json`、
//!    权限/能力文件校验与 Windows 资源生成。
//!
//! 注意：本文件受 workspace lint 约束（禁止 unwrap/expect），因此全部走 Result/Option。

use std::process::Command;

fn main() {
    emit_git_sha();
    tauri_build::build();
}

/// 注入 `FORGEDESK_GIT_SHA`；失败时降级为 "unknown"。
fn emit_git_sha() {
    let sha = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());

    println!("cargo:rustc-env=FORGEDESK_GIT_SHA={sha}");

    // git 提交号变化时重新构建（HEAD 与 refs 都可能变化）
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
}
