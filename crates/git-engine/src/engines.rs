//! 两个 Git 引擎的组合根。
//!
//! # 为什么在 `git-engine` 而不是 `services`
//!
//! 最初它放在 `services`（T1.2），理由是"只有 services 同时需要两个引擎"。
//! T1.9 打破了这个前提：快照管理器（`crates/snapshot`）同样要读（status/write-tree）
//! 与写（update-ref / reset / read-tree），而它的实现**不能**依赖 services
//! ——依赖方向是 services → snapshot，反过来就成环了。引擎的组合本身是引擎层的
//! 事实（"这个项目有哪两个引擎"），放在这里让 services 与 snapshot 都只是它的用户。
//!
//! # 为什么持有具体类型而不是 `dyn GitEngine`
//!
//! `discover` 之外的读操作确实只依赖 trait，但"打开仓库"要用到 CLI 侧的
//! 独有能力（`git --version`、`git config --local`），它们**不在** trait 上
//! （理由见 `CliGitEngine::version` 的文档）。持具体类型让这些调用点保持直白，
//! 而不是把 trait 扩成一个"什么都得实现、libgit2 只能返回 unsupported"的接口。

use crate::engine::{CliGitEngine, Libgit2Engine};
use forgedesk_domain::git::GitVersion;
use forgedesk_domain::AppResult;

/// 读引擎与写引擎。
#[derive(Debug)]
pub struct GitEngines {
    read: Libgit2Engine,
    write: CliGitEngine,
}

impl GitEngines {
    /// 创建两个引擎。
    ///
    /// 失败只可能来自 CLI 引擎（它需要起一个线程来驱动异步的进程执行器）。
    /// 这里不缓存 `git --version`：读版本只在"打开仓库"时发生一次，
    /// 而缓存会把"用户中途装了新版本 git"这件事变成需要重启才能生效的怪问题。
    pub fn new() -> AppResult<Self> {
        Ok(Self {
            read: Libgit2Engine::new(),
            write: CliGitEngine::new()?,
        })
    }

    /// 用现成的引擎组合（测试注入用）。
    pub fn with_engines(read: Libgit2Engine, write: CliGitEngine) -> Self {
        Self { read, write }
    }

    /// 读引擎（libgit2）。
    pub fn read(&self) -> &Libgit2Engine {
        &self.read
    }

    /// 写引擎（系统 git CLI）。
    pub fn write(&self) -> &CliGitEngine {
        &self.write
    }

    /// 系统 git 的版本。
    pub fn git_version(&self) -> AppResult<GitVersion> {
        self.write.version()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use crate::engine::GitEngine;

    use super::GitEngines;

    #[test]
    fn both_engines_are_available_and_the_cli_reports_a_version() {
        let engines = GitEngines::new().expect("创建引擎失败");

        // 读引擎必须能回答"这不是仓库"（而不是 panic 或返回 Ok）
        let missing = std::env::temp_dir().join("forgedesk-engines-does-not-exist");
        assert!(engines.read().discover(&missing).is_err());

        let version = engines.git_version().expect("读取 git 版本失败");
        assert_eq!(version.major, 2, "测试环境的 git 主版本应为 2");
    }
}
