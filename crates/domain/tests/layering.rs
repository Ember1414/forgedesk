//! 分层约束的自动化断言。
//!
//! 为什么用测试而不是文档：文档会被忽略，测试会红。
//! `crates/domain` 必须保持"纯逻辑"，一旦有人在里面引入 IO 依赖，这里的断言立刻失败。
//!
//! 实现方式：读取本 crate 的 `Cargo.toml`，解析依赖列表，断言不含被禁止的 crate。
//! `cargo test` 的工作目录是 crate 根目录，因此用 `CARGO_MANIFEST_DIR` 定位更稳妥。
//!
//! 集成测试是独立 crate，测试代码中允许 panic 式写法（失败即断言失败），
//! 因此在此显式放行 workspace 级别的相关 lint。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

/// domain 层禁止出现的依赖（IO / 运行时 / 平台）。
const FORBIDDEN: &[&str] = &[
    "git2",
    "rusqlite",
    "sqlx",
    "reqwest",
    "tokio",
    "tokio-util",
    "hyper",
    "octocrab",
    "keyring",
    "notify",
    "portable-pty",
    "wasmtime",
    "tauri",
    "forgedesk-git-engine",
    "forgedesk-storage",
    "forgedesk-services",
    "forgedesk-commands",
    "forgedesk-provider",
];

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

/// 提取 `[dependencies]` 段落中的 crate 名。
fn declared_dependencies(manifest: &str) -> Vec<String> {
    let mut in_deps = false;
    let mut names = Vec::new();

    for raw_line in manifest.lines() {
        let line = raw_line.trim();

        if line.starts_with('[') {
            in_deps = line == "[dependencies]";
            continue;
        }
        if !in_deps || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = line.split_once('.') {
            names.push(name.trim().to_owned());
        }
    }

    names
}

#[test]
fn domain_has_no_io_dependencies() {
    let manifest = fs::read_to_string(manifest_path()).expect("读取 domain 的 Cargo.toml 失败");
    let deps = declared_dependencies(&manifest);

    assert!(
        !deps.is_empty(),
        "未能从 Cargo.toml 解析出任何依赖，说明解析逻辑与文件格式已不一致，请同步更新本测试"
    );

    let violations: Vec<&String> = deps
        .iter()
        .filter(|name| FORBIDDEN.contains(&name.as_str()))
        .collect();

    assert!(
        violations.is_empty(),
        "domain 层引入了被禁止的 IO/运行时依赖：{violations:?}\n\
         领域逻辑必须保持纯净（见 crates/domain/src/lib.rs 的分层约束说明）。\n\
         如果需要 IO，请把逻辑放到对应的 infra crate，并在 domain 中只保留 trait 与纯数据结构。"
    );
}

#[test]
fn domain_dependency_list_is_expected() {
    let manifest = fs::read_to_string(manifest_path()).expect("读取 domain 的 Cargo.toml 失败");
    let mut deps = declared_dependencies(&manifest);
    deps.sort();

    let expected = ["serde", "serde_json", "thiserror", "time", "uuid"];
    assert_eq!(
        deps, expected,
        "domain 的依赖集合发生变化。若是有意为之，请同步更新本测试的期望值，\
         并在 docs/ARCHITECTURE.md 中说明理由。"
    );
}
