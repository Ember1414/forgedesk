//! 合并用例（T3.4）。
//!
//! # 这一层的三件事
//!
//! 1. **prepare 出计划**（预览 → 确认 → 执行的两段式，红线 R7 的计划形态）：
//!    快进裁决、source 独有提交清单、`merge-tree` 冲突预检、默认合并信息、
//!    等价命令，全部在**不碰工作区**的前提下给出；计划存注册表，
//!    execute 只认 plan_id，HEAD 变了即 `PLAN_STALE`（与重置同一约定）。
//! 2. **execute 前打快照**（`PreSync`，与 pull 同一语义——合并会产生提交或
//!    移动 HEAD）：快照失败不阻断（如实降级，安全网不是闸门）。
//! 3. **冲突后的 continue**：默认信息由 execute 时的 `-m` 写进 MERGE_MSG；
//!    用户要改信息时走 `git commit -m`（MERGE_HEAD 存在时它就是合并的
//!    continue），这是"默认信息允许编辑"的实现方式。
//!
//! # 审计在哪
//!
//! 与 stash / conflict 同一分工：审计由 commands 层写（它知道 IPC 参数），
//! 本层负责计划注册表、快照与 PLAN_STALE 校验。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::MutexGuard;

use forgedesk_domain::git::{
    default_merge_message, equivalent_merge_command, CommitSummary, ConflictState, LogQuery,
    MergeOutcome, MergePlan, MergePreviewReport, MergeSpec, RangeCommit, RebaseOutcome, RebasePlan,
    RebasePreview, RepoId,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_snapshot::{SnapshotId, SnapshotKind, SnapshotManager, SnapshotRequest};
use forgedesk_storage::RepositoryStore;

/// 待执行的合并计划（进程内；prepare 与 execute 是两次独立调用）。
#[derive(Debug, Default)]
pub struct MergePlanRegistry {
    plans: Mutex<HashMap<String, MergePlan>>,
}

impl MergePlanRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前持有的计划数（诊断与测试用）。
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// 是否没有待执行的计划。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 存入一份计划，返回 plan_id。
    pub fn insert(&self, plan: MergePlan) -> String {
        let plan_id = plan.plan_id.clone();
        self.lock().insert(plan_id.clone(), plan);
        plan_id
    }

    /// 取走一份计划（取走即失效：一个 plan_id 只对应"用户按下的那一次合并"）。
    pub fn take(&self, plan_id: &str) -> Option<MergePlan> {
        self.lock().remove(plan_id)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, MergePlan>> {
        self.plans
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// 合并用例。
pub struct MergeService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
    snapshots: &'a dyn SnapshotManager,
    plans: &'a MergePlanRegistry,
}

/// 独有提交清单的展示上限（与重置计划同一取舍：清单是给人看的，精确总数另给）。
const MAX_LISTED_COMMITS: usize = 30;

impl<'a> MergeService<'a> {
    /// 组装服务。
    pub fn new(
        engines: &'a GitEngines,
        store: RepositoryStore<'a>,
        snapshots: &'a dyn SnapshotManager,
        plans: &'a MergePlanRegistry,
    ) -> Self {
        Self {
            engines,
            store,
            snapshots,
            plans,
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

    /// execute 前打一个 `PreSync` 快照；失败**不阻断**（与 stash / sync 同一策略）。
    fn snapshot(&self, repo_id: i64, workdir: &std::path::Path) -> Option<SnapshotId> {
        let request = SnapshotRequest {
            repo_id,
            workdir,
            label: SnapshotKind::PreSync.key(),
            kind: SnapshotKind::PreSync,
        };
        match self.snapshots.create(&request) {
            Ok(outcome) => Some(outcome.id),
            Err(error) => {
                tracing::warn!(
                    error = %error.message(),
                    kind = SnapshotKind::PreSync.key(),
                    "合并执行前未能创建快照"
                );
                None
            }
        }
    }

    /// 生成合并计划（只读：不碰工作区与索引）。
    pub fn prepare(&self, repo_id: i64, spec: &MergeSpec) -> AppResult<MergePlan> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());
        let engine = self.engines.write();

        // 预检一并给 verdict（source 不存在 / 空仓库在这里被拦下）
        let report: MergePreviewReport = engine.merge_preview(&repo, &spec.revision)?;
        let head = engine
            .head_oid(&repo)?
            .ok_or_else(|| AppError::new(ErrorCode::Validation, "the repository has no commits"))?;

        // source 独有提交：精确计数 + 展示清单（≤30 条）
        let range = format!("HEAD..{}", spec.revision);
        let source_commit_count = engine.count_commits(&repo, &range, &[])?;
        let page = engine.log(
            &repo,
            LogQuery {
                revision: Some(range),
                limit: MAX_LISTED_COMMITS,
                skip: 0,
                all_branches: false,
                paths: Vec::new(),
                author: None,
                since: None,
                ..LogQuery::default()
            },
        )?;
        let source_only_commits: Vec<CommitSummary> = page
            .items
            .iter()
            .map(|commit| CommitSummary {
                oid: commit.oid.clone(),
                subject: commit.subject.clone(),
                author_time: commit.author.time,
            })
            .collect();

        // 默认信息的"并入分支"= 当前分支名（游离 HEAD 时不带 into）
        let into = engine
            .branch_list(&repo)?
            .into_iter()
            .find(|branch| !branch.is_remote && branch.is_head)
            .map(|branch| branch.name);

        let plan = MergePlan {
            plan_id: uuid::Uuid::new_v4().to_string(),
            source: spec.revision.clone(),
            strategy: spec.strategy,
            verdict: report.verdict,
            source_only_commits,
            source_commit_count: source_commit_count as usize,
            preview: report.preview,
            default_message: default_merge_message(&spec.revision, into.as_deref()),
            equivalent_command: equivalent_merge_command(
                &spec.revision,
                spec.strategy,
                spec.message.as_deref(),
            ),
            head_before: head,
        };
        self.plans.insert(plan.clone());
        Ok(plan)
    }

    /// 执行计划：PLAN_STALE 校验 → `PreSync` 快照 → 合并。
    ///
    /// `user_message` 是用户在预览里编辑过的合并信息；`None` 用默认信息。
    /// 冲突是**结果不是错误**（T2.8 约定）：`outcome.kind == Conflicted` 时
    /// 界面把用户送到冲突页。
    pub fn execute(
        &self,
        repo_id: i64,
        plan_id: &str,
        user_message: Option<String>,
    ) -> AppResult<MergeOutcome> {
        let plan = self.plans.take(plan_id).ok_or_else(|| {
            AppError::new(
                ErrorCode::NotFound,
                "the merge plan does not exist (it may have been executed already)",
            )
            .with_hint(plan_id.to_owned())
        })?;
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());
        let engine = self.engines.write();

        let head_now = engine.head_oid(&repo)?;
        if head_now.as_deref() != Some(plan.head_before.as_str()) {
            return Err(AppError::new(
                ErrorCode::PlanStale,
                "the repository changed after the merge plan was prepared",
            )
            .with_hint("prepare again"));
        }

        let snapshot = self.snapshot(repo_id, &workdir);

        // 信息优先级：用户编辑 > 默认信息。`--squash` / `--ff-only` 不产生
        // 合并提交，-m 传了也无害（squash 会写进 MERGE_MSG 供后续提交用）。
        let mut spec = MergeSpec::new(&plan.source).with_strategy(plan.strategy);
        if let Some(message) = user_message.or_else(|| Some(plan.default_message.clone())) {
            spec = spec.with_message(message);
        }
        let mut outcome = engine.merge(&repo, spec)?;
        outcome.snapshot_id = snapshot;
        Ok(outcome)
    }

    /// 冲突解决后的"继续合并"。
    ///
    /// `message` 为 `Some` 时走 `git commit -m`（用户编辑过的合并信息；
    /// MERGE_HEAD 存在时它就是合并的 continue），`None` 时沿用 MERGE_MSG。
    pub fn continue_merge(&self, repo_id: i64, message: Option<String>) -> AppResult<MergeOutcome> {
        let repo = RepoId::new(self.resolve_workdir(repo_id)?);
        let engine = self.engines.write();
        let state: ConflictState = engine.conflict_state(&repo)?;

        match state.op_kind {
            Some(forgedesk_domain::git::ConflictOpKind::Merge) => {}
            _ => {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    "no merge operation is in progress",
                ));
            }
        }
        if state.has_unresolved_files() {
            let files: Vec<String> = state
                .files
                .iter()
                .map(|file| file.path.to_string_lossy().into_owned())
                .collect();
            return Err(AppError::new(
                ErrorCode::ConflictUnresolved,
                "the merge cannot continue: some conflicts are still unresolved",
            )
            .with_hint(files.join(", ")));
        }

        match message {
            Some(message) => {
                // MERGE_HEAD 存在时的 commit 就是"继续合并"（git 内置语义）
                engine.commit(&repo, forgedesk_domain::git::CommitSpec::new(message))?;
            }
            None => {
                engine.conflict_continue(&repo, forgedesk_domain::git::ConflictOpKind::Merge)?;
            }
        }
        // 完成后的 HEAD 就是合并提交（merge 的 continue 必然产生合并提交）
        Ok(MergeOutcome {
            kind: forgedesk_domain::git::MergeKind::MergeCommit,
            oid: engine.head_oid(&repo)?,
            conflicts: Vec::new(),
            snapshot_id: None,
        })
    }
}

// ---------------------------------------------------------------- rebase（T3.7）

/// rebase 计划执行（T3.7）：快照 → 注入 todo → 三种结局的编排。
///
/// 实际的 todo 注入与结局判定在引擎（`GitEngine::rebase`）；这一层负责：
/// 执行前的 `PreHeadMove` 快照（rebase 重写历史、移动 HEAD——回滚的唯一依据）、
/// edit 暂停的恢复编排（`commit --amend` 接住用户改好的内容，再 continue）、
/// 以及继续的幂等（没有进行中的 rebase 时明确拒绝）。
pub struct RebaseService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
    snapshots: &'a dyn SnapshotManager,
}

impl<'a> RebaseService<'a> {
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

    fn snapshot(&self, repo_id: i64, workdir: &std::path::Path) -> Option<SnapshotId> {
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
                    "rebase 执行前未能创建快照"
                );
                None
            }
        }
    }

    /// 预演（不修改仓库）：GraphView 装配 + 校验 + 预览，给 T3.6 面板做
    /// "执行前校验"。走引擎的 `rebase_preview`（只读）。
    pub fn preview_only(&self, repo_id: i64, plan: &RebasePlan) -> AppResult<RebasePreview> {
        let repo = RepoId::new(self.resolve_workdir(repo_id)?);
        self.engines.write().rebase_preview(&repo, plan)
    }

    /// 区间的全部提交（T3.6 面板打开时的初始清单；从旧到新）。
    ///
    /// 面板必须列出区间内**每一个**提交：todo 里缺一个，git rebase 就把它
    /// 当 drop 丢掉——这是数据丢失级别的边界，所以清单由引擎直接从仓库
    /// 装着（不依赖前端已加载的分页数据）。
    pub fn range(&self, repo_id: i64, base: &str, head: &str) -> AppResult<Vec<RangeCommit>> {
        let repo = RepoId::new(self.resolve_workdir(repo_id)?);
        self.engines.write().rebase_range(&repo, base, head)
    }

    /// 执行 rebase 计划：快照 → 引擎执行 → 结局（完成 / 冲突 / edit 暂停）。
    ///
    /// 快照 id 随结果回传：T3.6 面板要拿它做"中止并还原"的兜底入口，
    /// T3.10 的"最近可回滚点"也要能指认它。快照创建失败时为 `None`
    ///（安全网降级不阻断执行，但界面必须如实显示"本次没有回滚点"）。
    /// 注意：引擎以**错误**结束（非暂停）时 `?` 提前返回，快照 id 不随
    /// 错误回传——错误路径下仓库未被改写（todo 被拒等场景），界面引导
    /// 用户去快照列表即可。
    pub fn execute(
        &self,
        repo_id: i64,
        plan: &RebasePlan,
    ) -> AppResult<(RebaseOutcome, Option<SnapshotId>)> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());
        let snapshot_id = self.snapshot(repo_id, &workdir);
        let outcome = self.engines.write().rebase(&repo, plan.clone())?;
        Ok((outcome, snapshot_id))
    }

    /// edit 暂停的恢复：用户改完工作区内容后调用——
    /// `git commit --amend`（接住新内容，沿用原信息）→ `git rebase --continue`。
    ///
    /// 幂等：不在 edit 停点时明确拒绝（重复调用不会产生重复提交）。
    pub fn continue_after_edit(&self, repo_id: i64) -> AppResult<RebaseOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());
        let engine = self.engines.write();

        let git_dir = forgedesk_git_engine::engine::enrich::resolve_git_dir(&workdir);
        let edit_paused = git_dir.join("rebase-merge/amend").is_file()
            || git_dir.join("rebase-merge/am").is_file();
        if !edit_paused {
            return Err(AppError::new(
                ErrorCode::Validation,
                "the rebase is not paused on an edit step",
            ));
        }

        // amend 接住用户的工作区改动。rebase 的 edit 语义是"改内容、信息
        // 不变"——而引擎的 commit 总是经 --file=- 喂信息，所以把 REBASE_HEAD
        // 的原信息读出来原样喂回（--amend + 同信息 = 信息不变）。
        let edit_commit = engine.show(&repo, "REBASE_HEAD")?;
        let original_message = match &edit_commit.body {
            Some(body) if !body.trim().is_empty() => {
                // Git 惯例：subject + 空行 + body。这里曾在写入时把 `\n\n`
                // 的转义写成了真实换行（信息变成 `subject\nbody`，缺少空行
                // 分隔），带 body 的提交在 edit 恢复后信息格式被改写。
                // 修复由带 body 的 edit 测试逐字断言钉住。
                format!("{}\n\n{body}", edit_commit.subject)
            }
            _ => edit_commit.subject.clone(),
        };
        engine.commit(
            &repo,
            forgedesk_domain::git::CommitSpec {
                message: original_message,
                paths: Vec::new(),
                amend: true,
                allow_empty: false,
                sign: None,
                sign_off: false,
                amend_mode: forgedesk_domain::git::AmendMode::IncludeStaged,
                author: None,
                no_verify: false,
            },
        )?;
        engine.conflict_continue(&repo, forgedesk_domain::git::ConflictOpKind::Rebase)?;
        Ok(RebaseOutcome::Completed {
            oid: engine.head_oid(&repo)?.unwrap_or_default(),
        })
    }
}
