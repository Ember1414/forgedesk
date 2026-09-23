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
