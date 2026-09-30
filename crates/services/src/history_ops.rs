//! 历史操作用例（T2.8）：拣选、反转、重置、reflog 恢复。
//!
//! # 为什么这四件事在一个服务里
//!
//! 它们共享同一套"动手之前"的机制：解析记录 → 打快照 → 执行 → 冲突/失败诊断，
//! 而且界面上的入口也在一起（历史页的提交右键菜单 + reflog 面板）。拆成四个服务会
//! 让同一段快照编排出现四份，而"某一份忘了打快照"正是红线 R7 最怕的事。
//!
//! # 重置为什么是两段式
//!
//! `git reset --hard` 丢弃的东西没有任何预告。红线 R7 要求"计划预览 → 快照 → 执行 →
//! 可回滚"，因此 `reset_prepare` 先把将被丢弃的东西算清楚（提交清单、已暂存改动、
//! 工作区改动、会被删掉的未跟踪文件、**远端是否已有这些提交**），
//! `reset_execute` 只认 `plan_id`——用户看到的与最终执行的是同一份数据。
//!
//! # 计划的有效期
//!
//! 与提交计划不同，这里**不设** TTL：reset 的作用只取决于"HEAD + 模式 + 目标提交"，
//! 而执行前会重新比对 HEAD（不一致即 [`ErrorCode::PlanStale`]）。HEAD 没变，
//! 五分钟前算出的"会丢弃哪些提交"就依然准确；多一个 TTL 只会制造"对话框开着没动，
//! 回来点确认却说过期"的困惑。

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use forgedesk_domain::git::{
    validate_ref_name, BranchCreateSpec, CherryPickSpec, CommitSummary, DiffSpec, DiffTarget,
    EntryKind, LogQuery, MergeOutcome, ReflogEntry, RepoId, ResetMode, ResetOutcome, ResetPlan,
    ResetRemoteImpact, ResetSpec, RevertSpec, StatusQuery, EMPTY_TREE_OID,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_snapshot::{SnapshotId, SnapshotKind, SnapshotManager, SnapshotRequest};
use forgedesk_storage::RepositoryStore;

use crate::repository::{system_clock, MillisClock};

/// 计划句柄的序号（同一毫秒内多次准备也不会撞）。
static PLAN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 待执行的重置计划（进程内）。
///
/// 取走即失效（`take`）：一个计划只对应"用户按下的那一次重置"。允许重复执行意味着
/// 同一个 plan_id 重置两次，第二次的 HEAD 比对必然失败，但用户会看到两次结果。
#[derive(Debug, Default)]
pub struct ResetPlanRegistry {
    plans: Mutex<HashMap<String, ResetPlan>>,
}

impl ResetPlanRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前持有的计划数（诊断与测试用）。
    pub fn len(&self) -> usize {
        lock_plans(&self.plans).len()
    }

    /// 是否没有待执行的计划。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 存入一份计划。
    pub fn insert(&self, plan: ResetPlan) -> String {
        let plan_id = plan.plan_id.clone();
        lock_plans(&self.plans).insert(plan_id.clone(), plan);
        plan_id
    }

    /// 取走一份计划（取走即失效）。
    pub fn take(&self, plan_id: &str) -> Option<ResetPlan> {
        lock_plans(&self.plans).remove(plan_id)
    }
}

/// 取锁：中毒（持锁线程 panic 过）时照常继续。
///
/// 这个注册表里没有"必须整体一致"的跨字段约束，中毒后继续用比让整个应用崩掉更合理。
fn lock_plans(
    mutex: &Mutex<HashMap<String, ResetPlan>>,
) -> MutexGuard<'_, HashMap<String, ResetPlan>> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// 历史操作用例。
pub struct HistoryOpsService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
    snapshots: &'a dyn SnapshotManager,
    plans: &'a ResetPlanRegistry,
    clock: MillisClock,
}

impl<'a> HistoryOpsService<'a> {
    /// 组装服务。
    pub fn new(
        engines: &'a GitEngines,
        store: RepositoryStore<'a>,
        snapshots: &'a dyn SnapshotManager,
        plans: &'a ResetPlanRegistry,
    ) -> Self {
        Self {
            engines,
            store,
            snapshots,
            plans,
            clock: Arc::new(system_clock),
        }
    }

    /// 替换时间源（测试用）。
    #[must_use]
    pub fn with_clock(mut self, clock: MillisClock) -> Self {
        self.clock = clock;
        self
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// 记录 id → 工作区路径。
    fn resolve_workdir(&self, repo_id: i64) -> AppResult<std::path::PathBuf> {
        let record = self.store.find_by_id(repo_id)?.ok_or(
            AppError::new(ErrorCode::NotFound, "the repository record does not exist")
                .with_detail(format!("repo_id: {repo_id}")),
        )?;
        Ok(std::path::PathBuf::from(record.path))
    }

    /// 快照（失败不阻断，返回 `None`）。
    fn snapshot(
        &self,
        repo_id: i64,
        workdir: &std::path::Path,
        kind: SnapshotKind,
    ) -> Option<SnapshotId> {
        let request = SnapshotRequest {
            repo_id,
            workdir,
            label: kind.key(),
            kind,
        };
        match self.snapshots.create(&request) {
            Ok(outcome) => Some(outcome.id),
            Err(error) => {
                tracing::warn!(error = %error.message(), kind = kind.key(), "历史操作前未能创建快照");
                None
            }
        }
    }

    // ------------------------------------------------------------ 拣选 / 反转

    /// 拣选提交（单个或区间）。
    ///
    /// 冲突**不是**错误：返回的 [`MergeOutcome`] 里带着冲突文件清单，
    /// 界面据此把用户送到冲突页（M3 的向导会接管）。
    pub fn cherry_pick(&self, repo_id: i64, spec: &CherryPickSpec) -> AppResult<MergeOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());

        // `--no-commit` 只改索引与工作区；否则会产生新提交（HEAD 移动）
        let kind = if spec.no_commit {
            SnapshotKind::PreWorktreeChange
        } else {
            SnapshotKind::PreHeadMove
        };
        let snapshot_id = self.snapshot(repo_id, &workdir, kind);

        let mut outcome = self.engines.write().cherry_pick(&repo, spec.clone())?;
        outcome.snapshot_id = snapshot_id;
        Ok(outcome)
    }

    /// 反转提交（单个或区间）。
    pub fn revert(&self, repo_id: i64, spec: &RevertSpec) -> AppResult<MergeOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());

        let kind = if spec.no_commit {
            SnapshotKind::PreWorktreeChange
        } else {
            SnapshotKind::PreHeadMove
        };
        let snapshot_id = self.snapshot(repo_id, &workdir, kind);

        let mut outcome = self.engines.write().revert(&repo, spec.clone())?;
        outcome.snapshot_id = snapshot_id;
        Ok(outcome)
    }

    // ------------------------------------------------------------ 重置（两段式）

    /// 生成重置计划（**不写仓库**）。
    pub fn reset_prepare(&self, repo_id: i64, spec: &ResetSpec) -> AppResult<ResetPlan> {
        if spec.is_path_scoped() {
            // 路径级 reset 只动索引里那几个文件，和"重置到某个提交"完全不是一回事。
            // 它的影响摘要应由暂存页给出（T1.6 已有取消暂存通道），不在这里做。
            return Err(AppError::new(
                ErrorCode::Validation,
                "a path-scoped reset does not need a plan",
            )
            .with_hint("paths"));
        }

        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);

        // 目标提交：用 `show` 解析（分支名、tag、`HEAD~3`、oid 都认）
        let target = self.engines.read().show(&repo, &spec.revision)?;
        let head_before = self
            .engines
            .read()
            .status(&repo, &StatusQuery::default())?
            .branch
            .oid
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::Validation,
                    "the repository has no commits to reset from yet",
                )
            })?;

        let range = format!("{}..{}", target.oid, head_before);
        // 走 `.write()`（= CLI 引擎）：`count_commits` 的 `--not` 语义只有 CLI 实现
        // 有（libgit2 侧明确拒绝，见该 trait 方法的说明）。与 diff 取 CLI 是同一个理由。
        let discarded_count = self.engines.write().count_commits(&repo, &range, &[])? as usize;

        // 只取前 MAX+1 条：多取一条用来判断"还有更多"，不必把几千条提交都读出来。
        //
        // 同样走 `.write()`（CLI）：`A..B` 区间只有 `git log` 认，libgit2 的
        // revparse 把它当成非法 pattern（实测 `failed to parse revision specifier`）。
        // 服务层不去"猜哪个引擎支持什么"：需要的语义只有一条实现，就用那一条。
        let page = self.engines.write().log(
            &repo,
            LogQuery {
                limit: ResetPlan::MAX_LISTED_COMMITS + 1,
                revision: Some(range.clone()),
                ..LogQuery::default()
            },
        )?;
        let discarded_truncated = page.items.len() > ResetPlan::MAX_LISTED_COMMITS;
        let discarded: Vec<CommitSummary> = page
            .items
            .into_iter()
            .take(ResetPlan::MAX_LISTED_COMMITS)
            .map(|commit| CommitSummary {
                oid: commit.oid,
                subject: commit.subject,
                author_time: commit.author.time,
            })
            .collect();

        // 远端影响：这些提交远端有没有（决定"丢弃后还能不能取回"）
        let upstream = self
            .engines
            .read()
            .branch_list(&repo)?
            .into_iter()
            .find(|branch| branch.is_head)
            .and_then(|branch| branch.upstream);
        let not_on_remote = match upstream.as_deref() {
            Some(upstream) => {
                self.engines
                    .write()
                    .count_commits(&repo, &range, &[upstream.to_owned()])? as usize
            }
            // 没有上游：本地没有跟踪目标，如实报告"全部只在本地"
            None => discarded_count,
        };

        let status = self.engines.read().status(&repo, &StatusQuery::default())?;
        let lost_staged = if spec.mode == ResetMode::Soft {
            // --soft 只移动 HEAD，索引原样保留
            Vec::new()
        } else {
            status
                .entries
                .iter()
                .filter(|entry| entry.index_status.is_changed())
                .cloned()
                .collect()
        };
        let lost_worktree = if spec.mode.is_destructive() {
            status
                .entries
                .iter()
                .filter(|entry| entry.worktree_status.is_changed())
                .cloned()
                .collect()
        } else {
            // --soft / --mixed 不动工作区
            Vec::new()
        };

        // `--hard` 会删掉"挡在路上的"未跟踪文件（与 `checkout -f` 同一行为）：
        // 挡路 = 目标提交里也有这个路径。只有真有未跟踪文件时才去算（那是一次 diff）
        let untracked_to_remove = if spec.mode.is_destructive() {
            let untracked: Vec<_> = status
                .entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::Untracked)
                .map(|entry| entry.path.clone())
                .collect();
            if untracked.is_empty() {
                Vec::new()
            } else {
                let tree = self.engines.write().diff(
                    &repo,
                    DiffSpec::new(DiffTarget::Between {
                        from: EMPTY_TREE_OID.to_owned(),
                        to: target.oid.clone(),
                    }),
                )?;
                let in_target: BTreeSet<String> = tree
                    .files
                    .iter()
                    .map(|file| file.path.to_string_lossy().into_owned())
                    .collect();
                untracked
                    .into_iter()
                    .filter(|path| in_target.contains(&path.to_string_lossy().into_owned()))
                    .collect()
            }
        } else {
            Vec::new()
        };

        let plan = ResetPlan {
            plan_id: format!(
                "reset-{}-{}",
                self.now(),
                PLAN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            mode: spec.mode,
            target_oid: target.oid,
            target_subject: target.subject,
            head_before,
            discarded,
            discarded_truncated,
            discarded_count,
            lost_staged,
            lost_worktree,
            untracked_to_remove,
            remote: ResetRemoteImpact {
                upstream,
                not_on_remote,
            },
            requires_confirmation: spec.mode.is_destructive(),
            // 所有模式都会移动 HEAD，因此都需要快照
            snapshot_required: true,
        };

        self.plans.insert(plan.clone());
        Ok(plan)
    }

    /// 执行重置计划。
    ///
    /// `confirmation`：`--hard` 时要求用户输入的确认词（见
    /// [`ResetPlan::CONFIRMATION_WORD`]）。
    pub fn reset_execute(
        &self,
        repo_id: i64,
        plan_id: &str,
        confirmation: Option<&str>,
    ) -> AppResult<ResetOutcome> {
        let plan = self.plans.take(plan_id).ok_or_else(|| {
            // 计划不存在或已经被用过：两者的下一步都是"重新预览"
            AppError::new(
                ErrorCode::PlanStale,
                "this reset plan is no longer available",
            )
            .with_detail(plan_id.to_owned())
        })?;

        if plan.requires_confirmation
            && !ResetPlan::confirmation_matches(confirmation.unwrap_or_default())
        {
            return Err(
                AppError::new(ErrorCode::Validation, "this reset must be confirmed")
                    .with_detail(plan.mode.as_flag().to_owned())
                    .with_hint(ResetPlan::CONFIRMATION_WORD.to_owned()),
            );
        }

        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());

        // 计划必须仍然对得上当前仓库：HEAD 变了说明"将被丢弃的东西"已经不同
        let head_now = self
            .engines
            .read()
            .status(&repo, &StatusQuery::default())?
            .branch
            .oid;
        if head_now.as_deref() != Some(plan.head_before.as_str()) {
            return Err(AppError::new(
                ErrorCode::PlanStale,
                "the branch moved after this plan was prepared",
            )
            .with_detail(plan.head_before.clone())
            .with_hint(head_now.unwrap_or_default()));
        }

        let snapshot_id = self.snapshot(repo_id, &workdir, SnapshotKind::PreHeadMove);

        // 用计划里的 **oid** 而不是用户当初输入的 revision：用户确认的目标就是它，
        // 再解析一次的话，期间引用被移动会让执行落到另一个提交上
        self.engines
            .write()
            .reset(&repo, ResetSpec::to(plan.target_oid.clone(), plan.mode))?;

        let head_after = self
            .engines
            .read()
            .status(&repo, &StatusQuery::default())?
            .branch
            .oid;

        Ok(ResetOutcome {
            mode: plan.mode,
            head_before: plan.head_before,
            head_after: head_after.unwrap_or_default(),
            discarded_count: plan.discarded_count,
            snapshot_id,
        })
    }

    // ------------------------------------------------------------ reflog

    /// reflog（新的在前）。
    pub fn reflog(&self, repo_id: i64, limit: usize) -> AppResult<Vec<ReflogEntry>> {
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines.read().reflog(&RepoId::new(workdir), limit)
    }

    /// 把 reflog 里的某一条恢复成一个**新分支**（最安全的恢复方式）。
    ///
    /// 不移动任何现有引用，因此不可能丢东西——界面上应当优先推荐它，
    /// "重置当前分支到此处"才是危险的那条路。
    pub fn create_branch_from_reflog(
        &self,
        repo_id: i64,
        index: usize,
        name: &str,
    ) -> AppResult<String> {
        validate_ref_name(name).map_err(|reason| {
            AppError::new(ErrorCode::Validation, "the branch name is not valid")
                .with_detail(reason.to_owned())
                .with_hint(name.to_owned())
        })?;

        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);

        let entry = self
            .engines
            .read()
            .reflog(&repo, index + 1)?
            .into_iter()
            .find(|entry| entry.index == index)
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::NotFound,
                    "there is no reflog entry at that index",
                )
                .with_detail(format!("HEAD@{{{index}}}"))
            })?;

        // 从该条记录的 oid 建分支（`start_point`），不切换过去：
        // 用户接下来多半要先看一眼那个状态，而不是立刻离开当前分支
        self.engines.write().branch_create(
            &repo,
            &BranchCreateSpec {
                name: name.to_owned(),
                start_point: Some(entry.oid.clone()),
                checkout: false,
                track_upstream: None,
            },
        )?;

        Ok(name.to_owned())
    }
}
