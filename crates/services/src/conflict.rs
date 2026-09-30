//! 冲突状态机用例（T3.1）。
//!
//! # 这一层的三件事
//!
//! 1. **continue 的前置校验**（红线）：还有未解决文件时返回
//!    `CONFLICT_UNRESOLVED` 并把文件清单放进 `hint`——git 自己也会拒绝，
//!    但那是解析 stderr 的事后行为；在动手前校验能把"哪些没解决"
//!    结构化地还给界面。
//! 2. **abort 的安全网编排**（红线 R7）：快照打在 abort **之前**，
//!    abort 之后**校验**仓库真的回到了操作前状态（对比 HEAD 与分支名）。
//!    "操作前状态"对 rebase 是 `rebase-merge/orig-head`（rebase 期间 HEAD
//!    已在被重放的位置上），对其余操作是当前 HEAD。
//! 3. **统一探测**：continue / abort / skip 都先采集一次冲突状态，
//!    "没有进行中的操作"是明确的 `VALIDATION` 错误而不是 git 的 stderr。
//!
//! # 审计在哪
//!
//! 与 stash / sync 同一分工：审计由 commands 层写（它知道 IPC 参数），
//! 本层只负责快照与校验。`mark_resolved` / `continue` / `skip` 不打快照
//! （git add 与完成操作不破坏工作区，与 stage / commit 同一取舍），
//! **只有 abort 打**（`PreHeadMove`：abort 会移动 HEAD）。

use std::path::{Path, PathBuf};

use forgedesk_domain::git::{
    ConflictAbortOutcome, ConflictContinueOutcome, ConflictFileDetail, ConflictOpKind,
    ConflictState, LineEnding, RepoId, RepoPath, TakeSide,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_git_engine::engine::enrich::resolve_git_dir;
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_snapshot::{SnapshotId, SnapshotKind, SnapshotManager, SnapshotRequest};
use forgedesk_storage::RepositoryStore;

/// 冲突状态机用例。
pub struct ConflictService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
    snapshots: &'a dyn SnapshotManager,
}

impl<'a> ConflictService<'a> {
    /// 组装服务。
    pub fn new(
        engines: &'a GitEngines,
        store: RepositoryStore<'a>,
        snapshots: &'a dyn SnapshotManager,
    ) -> Self {
        Self {
            engines,
            store,
            snapshots,
        }
    }

    /// 记录 id → 工作区路径。
    fn resolve_workdir(&self, repo_id: i64) -> AppResult<PathBuf> {
        let record = self.store.find_by_id(repo_id)?.ok_or(
            AppError::new(ErrorCode::NotFound, "the repository record does not exist")
                .with_detail(format!("repo_id: {repo_id}")),
        )?;
        Ok(PathBuf::from(record.path))
    }

    /// abort 前打一个 `PreHeadMove` 快照；失败**不阻断**（与 stash / sync 同一策略）。
    fn snapshot(&self, repo_id: i64, workdir: &Path) -> Option<SnapshotId> {
        let request = SnapshotRequest {
            repo_id,
            workdir,
            label: SnapshotKind::PreHeadMove.key(),
            kind: SnapshotKind::PreHeadMove,
        };
        match self.snapshots.create(&request) {
            Ok(outcome) => Some(outcome.id),
            Err(error) => {
                tracing::warn!(
                    error = %error.message(),
                    kind = SnapshotKind::PreHeadMove.key(),
                    "冲突中止前未能创建快照"
                );
                None
            }
        }
    }

    // ------------------------------------------------------------ 查询

    /// 采集冲突状态（只读）。
    pub fn state(&self, repo_id: i64) -> AppResult<ConflictState> {
        let repo = RepoId::new(self.resolve_workdir(repo_id)?);
        self.engines.write().conflict_state(&repo)
    }

    // ------------------------------------------------------------ 解决

    /// 标记文件已解决（`git add` + 校验 stage 清空，校验在引擎层）。
    pub fn mark_resolved(&self, repo_id: i64, paths: &[RepoPath]) -> AppResult<()> {
        let repo = RepoId::new(self.resolve_workdir(repo_id)?);
        self.engines.write().conflict_mark_resolved(&repo, paths)
    }

    /// 继续进行中的操作。
    ///
    /// 再次停在冲突上（序列重放撞新的冲突）是**正常结果**：`outcome.
    /// has_conflicts()` 为真时界面应刷新冲突状态，而不是弹错误。
    pub fn continue_operation(&self, repo_id: i64) -> AppResult<ConflictContinueOutcome> {
        let repo = RepoId::new(self.resolve_workdir(repo_id)?);
        let engine = self.engines.write();
        let state = engine.conflict_state(&repo)?;
        let op = require_operation(&state)?;

        if state.has_unresolved_files() {
            return Err(unresolved_error(&state));
        }
        engine.conflict_continue(&repo, op)
    }

    /// 跳过当前提交（只有 rebase 支持）。
    pub fn skip(&self, repo_id: i64) -> AppResult<ConflictContinueOutcome> {
        let repo = RepoId::new(self.resolve_workdir(repo_id)?);
        let engine = self.engines.write();
        let state = engine.conflict_state(&repo)?;
        let op = require_operation(&state)?;
        engine.conflict_skip(&repo, op)
    }

    /// 单个冲突文件的详情（三方 blob + 工作区形状 + 合并块）。只读。
    pub fn file_detail(&self, repo_id: i64, path: &RepoPath) -> AppResult<ConflictFileDetail> {
        let repo = RepoId::new(self.resolve_workdir(repo_id)?);
        self.engines.write().conflict_file_detail(&repo, path)
    }

    /// 整个文件采用一方（二进制 / 删除类冲突的"保留一方"）。
    pub fn take_side(&self, repo_id: i64, path: &RepoPath, side: TakeSide) -> AppResult<()> {
        let repo = RepoId::new(self.resolve_workdir(repo_id)?);
        self.engines.write().conflict_take_side(&repo, path, side)
    }

    /// 把编辑器结果写回工作区并标记已解决（EOL/BOM 由引擎按原文件形状重建）。
    pub fn apply_resolution(
        &self,
        repo_id: i64,
        path: &RepoPath,
        content: &str,
        eol: LineEnding,
        bom: bool,
        trailing_newline: bool,
    ) -> AppResult<()> {
        let repo = RepoId::new(self.resolve_workdir(repo_id)?);
        self.engines.write().conflict_apply_resolution(
            &repo,
            path,
            content,
            eol,
            bom,
            trailing_newline,
        )
    }

    /// 以"删除该文件"解决删除类冲突。
    pub fn remove_file(&self, repo_id: i64, path: &RepoPath) -> AppResult<()> {
        let repo = RepoId::new(self.resolve_workdir(repo_id)?);
        self.engines.write().conflict_remove_file(&repo, path)
    }

    /// 中止进行中的操作：快照 → abort → 校验回到操作前状态。
    ///
    /// 校验失败时返回 `INTERNAL` 错误并带上**实际到达的状态**：这不是
    /// 用户能自己修的问题（git 的 abort 正常情况下必然成功），如实上报
    /// 比假装成功重要。
    pub fn abort_operation(&self, repo_id: i64) -> AppResult<ConflictAbortOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());
        let engine = self.engines.write();

        let state = engine.conflict_state(&repo)?;
        let op = require_operation(&state)?;

        // 基线 = "操作开始前"的状态，abort 后必须回到它：
        // - rebase：HEAD 已在被重放的位置，"操作前"记录在 rebase 目录的 orig-head，
        //   分支名在 head-name；
        // - 其余操作：冲突期间 HEAD 未动，当前 HEAD / 分支就是操作前状态。
        let (expected_head, expected_branch) = match op {
            ConflictOpKind::Rebase => {
                let git_dir = resolve_git_dir(&workdir);
                let rebase_dir = if git_dir.join("rebase-merge").is_dir() {
                    git_dir.join("rebase-merge")
                } else {
                    git_dir.join("rebase-apply")
                };
                let orig_head = std::fs::read_to_string(rebase_dir.join("orig-head"))
                    .ok()
                    .map(|text| text.trim().to_owned())
                    .filter(|text| !text.is_empty());
                // head-name 形如 `refs/heads/main`；branch_list 给短名，比较前归一化
                let branch = state
                    .head_name
                    .as_deref()
                    .and_then(|name| name.strip_prefix("refs/heads/"))
                    .map(|name| name.to_owned());
                (orig_head, branch)
            }
            _ => (engine.head_oid(&repo)?, state.into_branch.clone()),
        };

        let snapshot_id = self.snapshot(repo_id, &workdir);
        // 引擎返回的 outcome 不直接采用：abort 后重新读 HEAD / 分支，
        // 用**校验过**的值构造结果（校验失败时已返回错误，走不到这里）
        engine.conflict_abort(&repo, op)?;

        let head_now = engine.head_oid(&repo)?;
        let branch_now = engine
            .branch_list(&repo)?
            .into_iter()
            .find(|branch| !branch.is_remote && branch.is_head)
            .map(|branch| branch.name);

        if head_now != expected_head || branch_now != expected_branch {
            return Err(AppError::new(
                ErrorCode::Internal,
                "git abort did not restore the pre-operation state",
            )
            .with_detail(format!(
                "expected head {expected_head:?} / branch {expected_branch:?}, got head \
                 {head_now:?} / branch {branch_now:?}"
            )));
        }

        Ok(ConflictAbortOutcome {
            head_oid: head_now,
            head_ref: branch_now,
            snapshot_id,
        })
    }
}

/// 没有进行中的操作时给出明确错误（四个动作共用的前置条件）。
fn require_operation(state: &ConflictState) -> AppResult<ConflictOpKind> {
    state.op_kind.ok_or_else(|| {
        AppError::new(
            ErrorCode::Validation,
            "no merge / rebase / cherry-pick / revert operation is in progress",
        )
    })
}

/// `CONFLICT_UNRESOLVED`：带未解决文件清单（`hint` 只放数据，见 CODING_STYLE §2.1）。
fn unresolved_error(state: &ConflictState) -> AppError {
    let files: Vec<String> = state
        .files
        .iter()
        .map(|file| file.path.to_string_lossy().into_owned())
        .collect();
    AppError::new(
        ErrorCode::ConflictUnresolved,
        "the operation cannot continue: some conflicts are still unresolved",
    )
    .with_hint(files.join(", "))
}
