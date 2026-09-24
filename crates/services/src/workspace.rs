//! 工作区用例：状态读取与文件级变更操作（T1.4）。
//!
//! # 与 `RepositoryService` 的分工
//!
//! `RepositoryService` 管"仓库的生命周期"（打开 / 克隆 / 最近列表），
//! 本模块管"仓库里面的东西"（工作区状态、暂存、放弃）。
//! 二者共享同一批基础设施（引擎、存储、打开仓库注册表），但不互相依赖——
//! 状态面板的刷新频率远高于打开仓库，拆开后各自的测试与演化互不拖累。
//!
//! # repo_id 的语义
//!
//! 与 `repo_close` 一致：`repo_id` 是**存储层记录的 id**（`repositories.id`），
//! 不是 git 的任何标识。从这里解析出工作区根目录后，git 操作只认路径。
//!
//! # 安全网边界（红线 R7）
//!
//! 本模块的 `discard` 会**销毁工作区修改**。快照编排（SnapshotManager）在 M3 落地，
//! 当前版本的安全网是：调用方（命令层）必须先经确认对话框把完整路径清单交给用户，
//! 且只接受显式分组的 tracked / untracked 两条列表；引擎层再按组分别执行
//! （tracked 走 `git restore --worktree`，untracked 才碰磁盘删除）。
//! M3 接入快照后，这里在执行前补一次"自动快照"调用即可，签名不变。

use std::path::PathBuf;

use forgedesk_domain::git::{DiscardSpec, RepoId, RepoPath, StatusQuery, StatusReport};
use forgedesk_domain::{AppError, AppResult, ErrorCode};

use crate::engines::GitEngines;
use crate::repository::OpenRepoRegistry;
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_storage::RepositoryStore;

pub struct WorkspaceService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
    #[allow(dead_code)]
    open: &'a OpenRepoRegistry,
}

impl WorkspaceService<'_> {
    pub fn new<'a>(
        engines: &'a GitEngines,
        store: RepositoryStore<'a>,
        open: &'a OpenRepoRegistry,
    ) -> WorkspaceService<'a> {
        WorkspaceService {
            engines,
            store,
            open,
        }
    }

    /// 解析记录 id 为工作区路径；记录不存在时返回 `NOT_FOUND`。
    fn resolve(&self, repo_id: i64) -> AppResult<PathBuf> {
        let record = self.store.find_by_id(repo_id)?.ok_or(
            AppError::new(ErrorCode::NotFound, "the repository record does not exist")
                .with_detail(format!("repo_id: {repo_id}")),
        )?;
        Ok(PathBuf::from(record.path))
    }

    /// 读取工作区状态。
    ///
    /// 读路径走 libgit2（无进程开销，状态面板会高频刷新）；
    /// `include_ignored` 的成本说明见 [`StatusQuery`]。
    pub fn status(&self, repo_id: i64, include_ignored: bool) -> AppResult<StatusReport> {
        let workdir = self.resolve(repo_id)?;
        let repo = RepoId::new(workdir);
        self.engines
            .read()
            .status(&repo, &StatusQuery { include_ignored })
    }

    /// 暂存指定路径（新增 / 修改 / 删除统一由 `git add -A --` 处理）。
    pub fn stage(&self, repo_id: i64, paths: &[RepoPath]) -> AppResult<()> {
        mutate(self, repo_id, |repo, engines| {
            engines.write().stage(repo, stage_spec(paths))
        })
    }

    /// 取消暂存（索引 ← HEAD，工作区不动）。
    pub fn unstage(&self, repo_id: i64, paths: &[RepoPath]) -> AppResult<()> {
        mutate(self, repo_id, |repo, engines| {
            engines.write().unstage(repo, stage_spec(paths))
        })
    }

    /// 放弃工作区修改。
    ///
    /// `tracked` / `untracked` 必须由调用方按 StatusReport 分组后传入
    /// （语义不同：前者可由 git 恢复，后者是磁盘删除）；
    /// 空请求是调用方的 bug，在这里显式拒绝而不是静默成功。
    pub fn discard(&self, repo_id: i64, spec: DiscardSpec) -> AppResult<()> {
        if spec.is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "the discard request has no paths",
            ));
        }
        self.discard_inner(repo_id, spec)
    }

    fn discard_inner(&self, repo_id: i64, spec: DiscardSpec) -> AppResult<()> {
        mutate(self, repo_id, |repo, engines| {
            engines.write().discard_worktree(repo, &spec)
        })
    }
}

/// `StageSpec::Paths` 的便捷构造（stage / unstage 共用同一形状）。
fn stage_spec(paths: &[RepoPath]) -> forgedesk_domain::git::StageSpec {
    forgedesk_domain::git::StageSpec::Paths(paths.to_vec())
}

/// 三个写操作的公共骨架：解析 → 路由到写引擎 → 收尾。
fn mutate<F>(service: &WorkspaceService<'_>, repo_id: i64, operation: F) -> AppResult<()>
where
    F: FnOnce(&RepoId, &GitEngines) -> AppResult<()>,
{
    let workdir = service.resolve(repo_id)?;
    let repo = RepoId::new(workdir);
    operation(&repo, service.engines)
}
