//! 集成测试共用的辅助设施。
//!
//! 这个目录不是测试目标（`tests/` 下的子目录不会各自变成测试二进制），
//! 只提供 `TempDir` 与参数转换这两个小工具。
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// 进程内唯一的临时目录，`Drop` 时递归删除。
///
/// 为什么不用 `tempfile` 之类的 crate：本项目只需要"建一个唯一目录、用完删掉"，
/// 为它引入依赖不划算（AGENTS.md §8 要求新增依赖说明理由与替代方案）。
/// 名字里带进程 id 与进程内计数器，因此并行测试之间不会互相覆盖。
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// 在系统临时目录下创建 `<prefix>-<pid>-<n>`。
    pub fn new(prefix: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "forgedesk-{prefix}-{}-{unique}",
            std::process::id()
        ));

        // 上一次运行若被强杀可能留下同名目录，先清掉再建
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("创建临时目录失败");

        Self { path }
    }

    /// 目录路径。
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // 清理失败不应该是测试失败的原因（Windows 上文件可能还被占用）
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 把 `&[&str]` 转成 `GitProcess::run` 需要的 `Vec<String>`。
pub fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// 同步执行一条 git 命令（**只用于测试夹具的准备**）。
///
/// 为什么仍然走产品自己的 `GitProcess` + `BlockingBridge`，而不是
/// `std::process::Command`：AGENTS §7 要求"所有 git 调用都经 GitProcess"，
/// 这条规则没有"测试例外"——而且用同一个执行器还顺带保证了夹具与产品
/// 看到的是同一套环境（locale、`GIT_TERMINAL_PROMPT`、可选锁）。
pub fn git_with_env(
    dir: &std::path::Path,
    command: &[&str],
    env: &[(&str, &str)],
) -> forgedesk_git_engine::process::GitOutput {
    let bridge = forgedesk_git_engine::engine::BlockingBridge::new().expect("创建桥失败");
    let process = forgedesk_git_engine::process::GitProcess::new();
    let mut opts = forgedesk_git_engine::process::GitRunOpts::new(dir).with_optional_locks(true);
    for (key, value) in env {
        opts = opts.with_env(*key, *value);
    }

    bridge
        .block_on(process.run(&args(command), opts))
        .expect("桥执行失败")
        .unwrap_or_else(|error| panic!("git {command:?} 失败: {error}"))
}

/// 同步执行一条 git 命令。
pub fn git(dir: &std::path::Path, command: &[&str]) -> forgedesk_git_engine::process::GitOutput {
    git_with_env(dir, command, &[])
}

/// 执行一条 git 命令并断言退出码为 0。
pub fn git_ok(dir: &std::path::Path, command: &[&str]) {
    let output = git(dir, command);
    assert!(
        output.success(),
        "git {command:?} 退出码 {:?}，stderr: {}",
        output.exit_code,
        output.stderr_lossy()
    );
}

/// 在临时目录里初始化一个仓库并配置好身份（夹具的统一起点）。
pub fn init_repo(dir: &std::path::Path) {
    git_ok(dir, &["init", "-q", "-b", "main", "."]);
    git_ok(dir, &["config", "user.name", "Fixture Author"]);
    git_ok(dir, &["config", "user.email", "author@example.com"]);
    // 行尾转换会让同一个工作区在两个引擎下产生不同的 blob，进而污染 diff 对比
    git_ok(dir, &["config", "core.autocrlf", "false"]);
}

/// 写一个文件（自动建父目录）。
pub fn write(dir: &std::path::Path, relative: &str, content: &[u8]) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("创建目录失败");
    }
    std::fs::write(path, content).expect("写文件失败");
}

/// 暂存全部改动并提交，时间戳逐个递增。
///
/// 时间戳必须**逐个递增**：同一秒内的多个提交在 git 与 libgit2 里的排序平局
/// 规则可能不同，那会让差分测试在"提交顺序"上产生假阳性。
pub fn commit_all(dir: &std::path::Path, message: &str, sequence: u32) {
    git_ok(dir, &["add", "--all"]);
    let date = format!("2024-01-02T03:04:{sequence:02}+00:00");
    let output = git_with_env(
        dir,
        &["commit", "-q", "-m", message],
        &[
            ("GIT_AUTHOR_DATE", date.as_str()),
            ("GIT_COMMITTER_DATE", date.as_str()),
        ],
    );
    assert!(output.success(), "commit 失败: {}", output.stderr_lossy());
}
