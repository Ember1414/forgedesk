// 打印一行结果，这就是命令行工具的职责（工作区的 `print_stdout` 禁令针对的是应用代码）
#![allow(clippy::print_stdout)]

//! 用"应用自己那条路"打开一个本地仓库：`RepositoryService::open`。
//!
//! 为什么需要它：界面上的"打开仓库"入口还没做（仓库选择器仍是 M0 的示例数据），
//! 而验收、演示与排障都需要一个真实仓库进入本地记录。命令行工具调用的是
//! **与界面完全相同**的服务层代码（审计、版本检查、登记一条不少），
//! 因此它产生的状态与点界面上的按钮没有区别——这也让"界面还没做"不会
//! 变成"功能还没验证"。
//!
//! 用法：
//!   cargo run --release -p forgedesk-services --example open_repo -- <仓库路径>
//!
//! 输出：`{"repoId":1,"name":"forgedesk","path":"E:\\Projects\\ForgeDesk","branch":"main"}`

use std::path::PathBuf;

use forgedesk_services::repository::OpenRepoRegistry;
use forgedesk_services::{GitEngines, RepositoryService};
use forgedesk_storage::{migrate, Database, RepositoryStore};

/// 应用数据目录（与 `src-tauri` 的 `app_data_dir()` 同一个位置）。
///
/// 这里必须**手动**算出来：服务层拿不到 Tauri 的路径 API，而这个工具要打开
/// 用户真正在用的那个数据库，而不是一个新建的空库。
fn app_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("io.github.ember1414.forgedesk")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("缺少仓库路径参数")?;

    let directory = app_data_dir();
    std::fs::create_dir_all(&directory)?;

    let database = Database::open(directory.join("forgedesk.db"))?;
    migrate(&database)?;

    let engines = GitEngines::new()?;
    let open = OpenRepoRegistry::new();
    let repository = RepositoryService::new(&engines, RepositoryStore::new(&database), &open);
    let opened = repository.open(&path)?;

    println!(
        "{{\"repoId\":{},\"name\":\"{}\",\"path\":\"{}\",\"branch\":\"{}\"}}",
        opened.record_id,
        opened
            .info
            .workdir
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        opened
            .info
            .workdir
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        opened.info.head.as_deref().unwrap_or(""),
        // 注意：这里打印的是分支**短名**（HEAD 指向的分支），不是"默认分支"
        // —— 打开仓库时用户关心的是"我现在在哪个分支上"
    );
    Ok(())
}
