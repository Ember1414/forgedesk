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

use forgedesk_domain::git::{
    DiffReport, DiffSpec, DiscardSpec, RepoId, RepoPath, StatusQuery, StatusReport,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};

use crate::engines::GitEngines;
use crate::repository::OpenRepoRegistry;
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_storage::RepositoryStore;

/// 状态读切到 CLI 引擎的索引条目阈值（方案 B，用户 2026-09-29 批准）。
///
/// 取值与前端性能模式（`performanceMode`，2000 条）呼应：两边描述的是同一个
/// "开始变慢"的位置。低于阈值时 libgit2 无进程开销的优势占主导（11ms 级）；
/// 超过后 libgit2 的状态计算进入秒级而 CLI 仍是亚秒，切换没有悬念。
pub const STATUS_CLI_ENTRY_THRESHOLD: u64 = 2_000;

/// 分流决策（纯函数，便于单测）：索引条目数达到阈值即走 CLI。
pub fn should_route_status_to_cli(index_entry_count: u64) -> bool {
    index_entry_count >= STATUS_CLI_ENTRY_THRESHOLD
}

/// 工作区用例：状态读取与文件级变更操作（暂存 / 取消暂存 / 放弃）。
pub struct WorkspaceService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
    /// 打开仓库注册表（与 RepositoryService 共享；状态与暂存不要求"已打开"，
    /// 保留引用是为了 M2 的"最近列表高亮"决策不必再改签名）。
    #[allow(dead_code)]
    open: &'a OpenRepoRegistry,
}

impl WorkspaceService<'_> {
    /// 组装服务（与 [`crate::repository::RepositoryService::new`] 同一批基础设施）。
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
    pub fn resolve_workdir(&self, repo_id: i64) -> AppResult<PathBuf> {
        let record = self.store.find_by_id(repo_id)?.ok_or(
            AppError::new(ErrorCode::NotFound, "the repository record does not exist")
                .with_detail(format!("repo_id: {repo_id}")),
        )?;
        Ok(PathBuf::from(record.path))
    }

    /// 读取工作区状态。
    ///
    /// 读路径**默认走 libgit2**（无进程开销，状态面板会高频刷新）；
    /// `include_ignored` 的成本说明见 [`StatusQuery`]。
    ///
    /// # 大仓库分流（方案 B，T2.10 后的用户决策）
    ///
    /// libgit2 的状态计算在"索引上万条、工作区全改"的形状上比 git CLI 慢约
    /// 10 倍（6.0s vs 0.61s，`docs/PERF-BASELINE.md` §3.1）。每次刷新前先用
    /// `index_entry_count` 做一次**只解析索引文件**的廉价探测（1 万条约
    /// 5ms，实测），超过 [`STATUS_CLI_ENTRY_THRESHOLD`] 就把这一次读切到
    /// CLI——两条路径的结果一致性由差分测试逐仓库对拍，富化（enrich）对
    /// 两条路径共用，因此界面上没有任何可感知差异，只有快慢。
    pub fn status(&self, repo_id: i64, include_ignored: bool) -> AppResult<StatusReport> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);
        let query = StatusQuery { include_ignored };

        let entry_count = self.engines.read().index_entry_count(&repo)?;
        if should_route_status_to_cli(entry_count) {
            self.engines.write().status(&repo, &query)
        } else {
            self.engines.read().status(&repo, &query)
        }
    }

    /// 读取 diff（行级内容）。
    ///
    /// **刻意路由到 CLI 引擎**：T1.5 要求补丁文本与用户终端一致
    /// （hooks / attributes / 外部 diff 都要生效），libgit2 的格式化输出被任务明确排除。
    /// libgit2 的 diff 实现保留用于差分测试中的统计对拍。
    pub fn diff(&self, repo_id: i64, spec: DiffSpec) -> AppResult<DiffReport> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);
        self.engines.write().diff(&repo, spec)
    }

    /// 生成原始补丁文本（复制 / 导出 .patch；T1.6 部分暂存的底稿）。
    ///
    /// 与 [`Self::diff`] 同一 spec；补丁是否截断由 spec.force_full 决定，
    /// 调用方在导出场景应显式传 true（用户要求的就是完整文件）。
    /// 返回原始字节——补丁里的路径与内容都可能是非 UTF-8。
    pub fn diff_patch(&self, repo_id: i64, spec: DiffSpec) -> AppResult<Vec<u8>> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);
        self.engines.write().diff_patch(&repo, &spec)
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
    let workdir = service.resolve_workdir(repo_id)?;
    let repo = RepoId::new(workdir);
    operation(&repo, service.engines)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{should_route_status_to_cli, STATUS_CLI_ENTRY_THRESHOLD};

    #[test]
    fn routing_kicks_in_exactly_at_the_threshold() {
        assert!(!should_route_status_to_cli(0));
        assert!(!should_route_status_to_cli(STATUS_CLI_ENTRY_THRESHOLD - 1));
        assert!(should_route_status_to_cli(STATUS_CLI_ENTRY_THRESHOLD));
        assert!(should_route_status_to_cli(STATUS_CLI_ENTRY_THRESHOLD + 1));
        assert!(should_route_status_to_cli(u64::MAX));
    }
}
