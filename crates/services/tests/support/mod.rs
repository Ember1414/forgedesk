// 测试支撑模块：允许测试惯用的 unwrap/expect/panic（与其它测试文件同一约定）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

//! 集成测试共用的辅助设施（临时目录、git 夹具、内存数据库）。
//!
//! 这个目录不是测试目标（`tests/` 下的子目录不会各自变成测试二进制）。
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use forgedesk_git_engine::engine::BlockingBridge;
use forgedesk_git_engine::process::{GitOutput, GitProcess, GitRunOpts};
use forgedesk_storage::{migrate, Database};

/// 进程内唯一的临时目录，`Drop` 时递归删除。
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// 在系统临时目录下创建 `<prefix>-<pid>-<n>`。
    pub fn new(prefix: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "forgedesk-svc-{prefix}-{}-{unique}",
            std::process::id()
        ));

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

/// 同步执行一条 git 命令（只用于夹具准备，走产品自己的执行器）。
pub fn git(dir: &Path, command: &[&str]) -> GitOutput {
    let bridge = BlockingBridge::new().expect("创建桥失败");
    let process = GitProcess::new();
    let args: Vec<String> = command.iter().map(|item| (*item).to_owned()).collect();

    bridge
        .block_on(process.run(&args, GitRunOpts::new(dir).with_optional_locks(true)))
        .expect("桥执行失败")
        .unwrap_or_else(|error| panic!("git {command:?} 失败: {error}"))
}

/// 执行一条 git 命令并断言退出码为 0。
pub fn git_ok(dir: &Path, command: &[&str]) {
    let output = git(dir, command);
    assert!(
        output.success(),
        "git {command:?} 退出码 {:?}，stderr: {}",
        output.exit_code,
        output.stderr_lossy()
    );
}

/// 初始化一个仓库并配置好身份。
pub fn init_repo(dir: &Path) {
    git_ok(dir, &["init", "-q", "-b", "main", "."]);
    git_ok(dir, &["config", "user.name", "Fixture Author"]);
    git_ok(dir, &["config", "user.email", "author@example.com"]);
    git_ok(dir, &["config", "core.autocrlf", "false"]);
}

/// 写一个文件（自动建父目录）。
pub fn write(dir: &Path, relative: &str, content: &[u8]) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("创建目录失败");
    }
    std::fs::write(path, content).expect("写文件失败");
}

/// 暂存全部改动并提交。
pub fn commit_all(dir: &Path, message: &str) {
    git_ok(dir, &["add", "--all"]);
    git_ok(dir, &["commit", "-q", "-m", message]);
}

/// 把一个本地目录变成 `file://` URL（`--depth` 对本地路径克隆无效，必须走 file://）。
pub fn file_url(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.starts_with('/') {
        format!("file://{normalized}")
    } else {
        format!("file:///{normalized}")
    }
}

/// 打开一个已完成迁移的内存数据库。
pub fn memory_database() -> Database {
    let database = Database::open_in_memory().expect("打开内存库失败");
    migrate(&database).expect("迁移失败");
    database
}
