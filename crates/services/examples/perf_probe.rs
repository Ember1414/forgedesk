// 输出一行 JSON 到 stdout 就是这个程序的唯一职责：工作区的 `print_stdout` 禁令
// 针对的是应用代码（那里必须走日志与 IPC），对"给人看的命令行工具"不适用。
#![allow(clippy::print_stdout)]

//! M1 性能探针（T1.12 第 4 条）。
//!
//! 在**真实仓库**上量四个数字，输出一行 JSON（便于脚本汇总成基线表）：
//!
//!   - `openMs`：`RepositoryService::open`（读 HEAD、判断仓库形状、登记记录）；
//!   - `statusMs`：`WorkspaceService::status`（状态面板一屏所需的数据）；
//!   - `diffMs`：单个文件的完整 diff（查看器打开时的那次请求）；
//!   - `stagingMs`：整文件暂存 + 取消暂存各一次（写路径 + 补丁通道）；
//!   - `watchCpuMs`（可选）：启动文件监听后空闲 `--watch-seconds` 秒所消耗的
//!     **进程 CPU 时间**（读 `/proc` 或系统调用都要外部取样，因此由调用方
//!     用 PowerShell 采样，探针只负责"开着监听空转"）。
//!
//! 为什么用 example 而不是单元测试：这些数字要在**发布配置**下量
//! （`cargo run --release`），而测试二进制永远是 debug——debug 的 git 子进程
//! 启动开销与内存占用都不代表用户看到的东西。
//!
//! 用法：
//!   cargo run --release -p forgedesk-services --example perf_probe -- <仓库路径> [--watch-seconds 10]
//!
//! 输出（stdout 一行 JSON）：
//!   {"openMs":3.1,"statusMs":12.4,"diffMs":8.0,"stagingMs":21.5,"fileCount":1234,"watchCpuMs":null}

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use forgedesk_domain::git::{DiffSpec, DiffTarget, RepoPath};
use forgedesk_platform::{FileWatcher, NotifyFileWatcher, WatchOptions};
use forgedesk_services::repository::OpenRepoRegistry;
use forgedesk_services::{GitEngines, RepositoryService, StagingService, WorkspaceService};
use forgedesk_storage::{migrate, Database, RepositoryStore};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().ok_or("缺少仓库路径参数")?);

    let mut watch_seconds: u64 = 0;
    let mut force_full = false;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--watch-seconds" => {
                watch_seconds = args.next().ok_or("--watch-seconds 需要一个秒数")?.parse()?;
            }
            "--force-full" => force_full = true,
            other => return Err(format!("未知参数：{other}").into()),
        }
    }

    // 探针自己用临时数据库：它不该动用户的真实记录（也不能依赖应用数据目录存在）
    let database = Database::open_in_memory()?;
    migrate(&database)?;

    let engines = GitEngines::new()?;
    let open = OpenRepoRegistry::new();
    let store = RepositoryStore::new(&database);

    // 1) 打开仓库
    let started = Instant::now();
    let repository = RepositoryService::new(&engines, RepositoryStore::new(&database), &open);
    let opened = repository.open(&path)?;
    let open_ms = millis(started);

    let workspace = WorkspaceService::new(&engines, store, &open);

    // 2) 状态刷新
    let started = Instant::now();
    let report = workspace.status(opened.record_id, false)?;
    let status_ms = millis(started);

    let file_count = report.entries.len() as u64;
    let first_path = report
        .entries
        .iter()
        .find(|entry| entry.kind != forgedesk_domain::git::EntryKind::Untracked)
        .map(|entry| entry.path.clone());

    // 3) 单文件 diff（有已跟踪变更时才做）
    let diff_ms = match first_path.clone() {
        None => None,
        Some(path) => {
            let spec = DiffSpec {
                target: DiffTarget::Unstaged,
                paths: vec![path],
                context_lines: forgedesk_domain::git::DEFAULT_CONTEXT_LINES,
                ignore_whitespace: false,
                detect_renames: true,
                force_full,
            };
            let started = Instant::now();
            let _ = workspace.diff(opened.record_id, spec)?;
            Some(millis(started))
        }
    };

    // 4) 暂存 + 取消暂存（整文件粒度：走 git add / git reset，最快的一档）
    let staging_ms = match first_path {
        None => None,
        Some(path) => {
            let staging = StagingService::new(
                WorkspaceService::new(&engines, RepositoryStore::new(&database), &open),
                &engines,
            );
            let paths = [RepoPath::from(path.to_string_lossy().as_ref())];
            let started = Instant::now();
            workspace.stage(opened.record_id, &paths)?;
            workspace.unstage(opened.record_id, &paths)?;
            let _ = staging;
            Some(millis(started))
        }
    };

    // 5) 监听空闲（可选）：只启动、不消费，CPU 由外部采样
    if watch_seconds > 0 {
        let watcher = NotifyFileWatcher;
        let repo_root = path.clone();
        let handle = watcher.watch(&repo_root, WatchOptions::default(), Arc::new(|_| {}))?;
        std::thread::sleep(std::time::Duration::from_secs(watch_seconds));
        drop(handle);
    }

    println!(
        "{{\"openMs\":{open_ms:.1},\"statusMs\":{status_ms:.1},\"diffMs\":{},\"stagingMs\":{},\"fileCount\":{file_count},\"watchSeconds\":{watch_seconds}}}",
        diff_ms.map_or("null".to_owned(), |value| format!("{value:.1}")),
        staging_ms.map_or("null".to_owned(), |value| format!("{value:.1}")),
    );

    Ok(())
}

fn millis(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}
