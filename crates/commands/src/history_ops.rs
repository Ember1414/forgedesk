//! 历史操作命令（T2.8）：储藏、拣选、反转、重置、reflog。
//!
//! # 这一层做什么
//!
//! 1. **IPC 形状**：请求 DTO（`*Request`）把前端的 camelCase 翻成领域 spec；
//!    响应 DTO 把领域结果翻成 camelCase（docs/API.md §1）。领域的 `ResetSpec` /
//!    `StashSpec` 只派生 `Serialize`——它们带着保真的字节路径，线上表示由这里定。
//! 2. **审计**：每个写操作一条记录（红线 R7 的"可追溯"那一半）。快照由 services
//!    层在动手之前打（另一半），因为"什么时候算动手之前"只有编排层知道。
//! 3. **广播变化**：写完之后发 `repo:changed`，让其它面板（状态、历史、分支）
//!    立刻失效重查——用户不会理解"我这里刚重置完，历史页还是旧的"。
//! 4. **不产出用户可见文案**：错误经 `to_app_error` 分类与脱敏，界面按错误码走 i18n。
//!
//! # 为什么这些命令是同步的（不是长任务）
//!
//! 它们全是**本地**操作：拣选/反转/重置/储藏都不碰网络，最慢的是重置一份大工作区
//! （几秒），而长任务通道（`job:*`）的代价是界面必须为进度与取消多做一套状态。
//! 真正可能很慢的部分（克隆、fetch）已经有任务通道了。

use forgedesk_domain::git::{
    CherryPickSpec, MergeOutcome, ReflogEntry, RepoPath, ResetMode, ResetOutcome, ResetPlan,
    ResetSpec, RevertSpec, StashEntry, StashOutcome, StashSpec,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_platform::watcher::WatchKind;
use forgedesk_services::audit::op_type;
use forgedesk_services::{AuditArgs, AuditEntry, StashDiscardOutcome, StashSaveOutcome};
use forgedesk_snapshot::SnapshotKind;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::audit;
use crate::state::AppState;
use crate::workspace::{emit_changed, DiffReportDto, FileChangeDto};

// ---------------------------------------------------------------- 请求 DTO

/// `git_stash_save` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashSaveRequest {
    /// 描述信息（缺省时 git 自己生成 `WIP on <branch>: …`）。
    #[serde(default)]
    pub message: Option<String>,
    /// 把未跟踪文件一起储藏（`-u`）。
    #[serde(default)]
    pub include_untracked: bool,
    /// 保留索引（`--keep-index`）：已经 `git add` 的内容留在暂存区。
    #[serde(default)]
    pub keep_index: bool,
    /// 只储藏这些路径（空 = 全部）。
    #[serde(default)]
    pub paths: Vec<String>,
}

impl StashSaveRequest {
    fn into_spec(self) -> StashSpec {
        StashSpec {
            include_untracked: self.include_untracked,
            keep_index: self.keep_index,
            ..StashSpec::push(self.message)
        }
        .with_paths(
            self.paths
                .iter()
                .map(|path| RepoPath::from(path.as_str()))
                .collect(),
        )
    }
}

/// `git_stash_apply` / `git_stash_pop` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashApplyRequest {
    /// `stash@{n}` 里的 n。
    pub index: usize,
    /// 同时恢复索引（`--index`）：把"当初暂存过的内容"还原成暂存态。
    #[serde(default)]
    pub restore_index: bool,
}

impl StashApplyRequest {
    fn into_spec(self, pop: bool) -> StashSpec {
        let base = if pop {
            StashSpec::pop(self.index)
        } else {
            StashSpec::apply(self.index)
        };
        base.with_restore_index(self.restore_index)
    }
}

/// `git_stash_branch` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashBranchRequest {
    /// `stash@{n}` 里的 n。
    pub index: usize,
    /// 新分支名。
    pub name: String,
}

/// `git_reset_prepare` 的重置参数。
///
/// `ResetSpec` 本身不能反序列化（它带 `RepoPath`），因此请求体在这里定义。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetRequest {
    /// 目标提交（oid / 分支名 / `HEAD~2`）。
    pub revision: String,
    /// 模式：`soft` / `mixed` / `hard`。
    pub mode: ResetMode,
}

impl ResetRequest {
    fn into_spec(self) -> ResetSpec {
        ResetSpec::to(self.revision, self.mode)
    }
}

/// `git_cherry_pick` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CherryPickRequest {
    /// 单个 rev 或 `A..B` 区间。
    pub revision: String,
    /// `-x`：在提交信息里记录来源。
    #[serde(default)]
    pub record_source: bool,
    /// `--no-commit`：只应用改动。
    #[serde(default)]
    pub no_commit: bool,
}

impl CherryPickRequest {
    fn into_spec(self) -> CherryPickSpec {
        CherryPickSpec {
            record_source: self.record_source,
            no_commit: self.no_commit,
            ..CherryPickSpec::new(self.revision)
        }
    }
}

/// `git_revert` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertRequest {
    /// 单个 rev 或 `A..B` 区间。
    pub revision: String,
    /// 反转合并提交时的主父（从 1 开始）。
    #[serde(default)]
    pub mainline: Option<u32>,
    /// `--no-commit`：只应用改动。
    #[serde(default)]
    pub no_commit: bool,
}

impl RevertRequest {
    fn into_spec(self) -> RevertSpec {
        RevertSpec {
            mainline: self.mainline,
            no_commit: self.no_commit,
            ..RevertSpec::new(self.revision)
        }
    }
}

// ---------------------------------------------------------------- 响应 DTO

/// 储藏的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StashSaveDto {
    /// 是否真的产生了新条目（`false` = 没有可储藏的内容，不是失败）。
    pub stashed: bool,
    /// 新条目。
    pub entry: Option<StashEntry>,
    /// 操作前打的快照 id（审计表与前端回滚入口都靠它；T2.10 补上）。
    pub snapshot_id: Option<i64>,
}

impl From<StashSaveOutcome> for StashSaveDto {
    fn from(outcome: StashSaveOutcome) -> Self {
        Self {
            stashed: outcome.stashed,
            entry: outcome.entry,
            snapshot_id: outcome.snapshot_id,
        }
    }
}

/// 某条 stash 的 diff（含未跟踪文件那一份）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StashShowDto {
    /// 这条 stash。
    pub entry: StashEntry,
    /// 相对 base 的变更（已跟踪文件）。
    pub diff: DiffReportDto,
    /// 未跟踪文件的变更（`-u` 创建的 stash 才有）。
    pub untracked: Option<DiffReportDto>,
}

/// 丢弃的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StashDiscardDto {
    /// 被丢掉的条目（含 oid：`gc` 回收之前还能按它找回）。
    pub dropped: Vec<StashEntry>,
    /// 丢弃之前打的快照（T3.11）；打不出来时为 `None`。
    ///
    /// 有它才谈得上"能不能回滚"：回滚靠快照里记下的 `refs/stash` oid 清单
    /// 把条目重新登记回栈（`git stash store`）。
    pub snapshot_id: Option<i64>,
}

impl From<StashDiscardOutcome> for StashDiscardDto {
    /// 只有条目、没有快照 id 的形状（服务层自己不打包点）。
    fn from(outcome: StashDiscardOutcome) -> Self {
        Self {
            dropped: outcome.dropped,
            snapshot_id: None,
        }
    }
}

/// 计划里的一条提交摘要。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummaryDto {
    /// 提交 oid。
    pub oid: String,
    /// 提交信息首行。
    pub subject: String,
    /// 作者时间（Unix 秒）。
    pub author_time: Option<i64>,
}

/// 远端影响。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetRemoteImpactDto {
    /// 上游短名。
    pub upstream: Option<String>,
    /// 将被丢弃、且远端也没有的提交数。
    pub not_on_remote: usize,
}

/// 重置计划的 DTO（字段含义见 `domain::git::ResetPlan`）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPlanDto {
    /// 计划句柄。
    pub plan_id: String,
    /// 模式。
    pub mode: ResetMode,
    /// 目标提交 oid。
    pub target_oid: String,
    /// 目标提交的首行信息。
    pub target_subject: String,
    /// 计划生成时的 HEAD。
    pub head_before: String,
    /// 将被丢弃的提交（最多 30 条）。
    pub discarded: Vec<CommitSummaryDto>,
    /// 是否还有更多未列出。
    pub discarded_truncated: bool,
    /// 将被丢弃的提交总数。
    pub discarded_count: usize,
    /// 将被丢弃的已暂存改动。
    pub lost_staged: Vec<FileChangeDto>,
    /// 将被丢弃的工作区改动。
    pub lost_worktree: Vec<FileChangeDto>,
    /// 会被覆盖的未跟踪文件。
    pub untracked_to_remove: Vec<String>,
    /// 远端影响。
    pub remote: ResetRemoteImpactDto,
    /// 是否需要输入确认词。
    pub requires_confirmation: bool,
    /// 需要输入的确认词（`requiresConfirmation` 为真时才有）。
    ///
    /// 由后端给出而不是前端写死：这个词是**执行闸门的一部分**，前端抄一份
    /// 迟早会在某次改动里和后端不一致——用户照着界面输入却被拒绝。
    pub confirmation_word: Option<String>,
    /// 执行前是否必须打快照。
    pub snapshot_required: bool,
}

impl From<ResetPlan> for ResetPlanDto {
    fn from(plan: ResetPlan) -> Self {
        Self {
            plan_id: plan.plan_id,
            mode: plan.mode,
            target_oid: plan.target_oid,
            target_subject: plan.target_subject,
            head_before: plan.head_before,
            discarded: plan
                .discarded
                .into_iter()
                .map(|summary| CommitSummaryDto {
                    oid: summary.oid,
                    subject: summary.subject,
                    author_time: summary.author_time,
                })
                .collect(),
            discarded_truncated: plan.discarded_truncated,
            discarded_count: plan.discarded_count,
            lost_staged: plan
                .lost_staged
                .iter()
                .map(FileChangeDto::from_entry)
                .collect(),
            lost_worktree: plan
                .lost_worktree
                .iter()
                .map(FileChangeDto::from_entry)
                .collect(),
            untracked_to_remove: plan
                .untracked_to_remove
                .iter()
                .map(ToString::to_string)
                .collect(),
            remote: ResetRemoteImpactDto {
                upstream: plan.remote.upstream,
                not_on_remote: plan.remote.not_on_remote,
            },
            requires_confirmation: plan.requires_confirmation,
            confirmation_word: if plan.requires_confirmation {
                Some(ResetPlan::CONFIRMATION_WORD.to_owned())
            } else {
                None
            },
            snapshot_required: plan.snapshot_required,
        }
    }
}

/// 重置的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetOutcomeDto {
    /// 实际执行的模式。
    pub mode: ResetMode,
    /// 执行前的 HEAD。
    pub head_before: String,
    /// 执行后的 HEAD。
    pub head_after: String,
    /// 被丢弃的提交数。
    pub discarded_count: usize,
    /// 执行前打的快照 id。
    pub snapshot_id: Option<i64>,
}

impl From<ResetOutcome> for ResetOutcomeDto {
    fn from(outcome: ResetOutcome) -> Self {
        Self {
            mode: outcome.mode,
            head_before: outcome.head_before,
            head_after: outcome.head_after,
            discarded_count: outcome.discarded_count,
            snapshot_id: outcome.snapshot_id,
        }
    }
}

/// 从 stash / reflog 建分支的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewBranchDto {
    /// 新分支名。
    pub branch: String,
}

/// 仓储 id 的校验（命令层统一入口，避免每个命令重复写 NotFound）。
pub(crate) fn require_repo(repo_id: i64) -> AppResult<i64> {
    if repo_id <= 0 {
        return Err(AppError::new(
            ErrorCode::Validation,
            "a repository id must be a positive number",
        )
        .with_hint("repoId"));
    }
    Ok(repo_id)
}

// ---------------------------------------------------------------- stash 命令

/// 储藏当前改动。写操作：快照（services）+ 审计。
#[tauri::command]
pub fn git_stash_save(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: StashSaveRequest,
) -> AppResult<StashSaveDto> {
    let repo_id = require_repo(repo_id)?;
    let args = AuditArgs::new()
        .text("message", spec.message.as_deref().unwrap_or_default())
        .flag("includeUntracked", spec.include_untracked)
        .flag("keepIndex", spec.keep_index)
        .number("paths", spec.paths.len() as i64);
    let spec = spec.into_spec();

    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::STASH_SAVE).with_args(args),
        || {
            let outcome = state.stash_service().save(repo_id, &spec)?;
            emit_changed(&app, repo_id, WatchKind::Workspace, Vec::new());
            Ok(StashSaveDto::from(outcome))
        },
    )
}

/// stash 列表。只读。
#[tauri::command(async)]
pub fn git_stash_list(state: State<'_, AppState>, repo_id: i64) -> AppResult<Vec<StashEntry>> {
    let repo_id = require_repo(repo_id)?;
    state.stash_service().list(repo_id)
}

/// 某条 stash 的 diff（相对 base + 未跟踪文件）。只读。
#[tauri::command(async)]
pub fn git_stash_show(
    state: State<'_, AppState>,
    repo_id: i64,
    index: usize,
) -> AppResult<StashShowDto> {
    let repo_id = require_repo(repo_id)?;
    let outcome = state.stash_service().show(repo_id, index)?;
    Ok(StashShowDto {
        entry: outcome.entry,
        diff: DiffReportDto::from_domain(&outcome.diff),
        untracked: outcome.untracked.as_ref().map(DiffReportDto::from_domain),
    })
}

/// 应用某条 stash 并保留它。写操作：快照 + 审计。冲突时返回冲突清单。
#[tauri::command]
pub fn git_stash_apply(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: StashApplyRequest,
) -> AppResult<StashOutcome> {
    let repo_id = require_repo(repo_id)?;
    let args = AuditArgs::new()
        .number("index", spec.index as i64)
        .flag("restoreIndex", spec.restore_index)
        .text("action", "apply");
    let spec = spec.into_spec(false);

    run_stash_action(&state, &app, repo_id, op_type::STASH_APPLY, args, || {
        state.stash_service().apply(repo_id, &spec)
    })
}

/// 应用某条 stash 并删除它。写操作：快照 + 审计。冲突时**不会**删除。
#[tauri::command]
pub fn git_stash_pop(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: StashApplyRequest,
) -> AppResult<StashOutcome> {
    let repo_id = require_repo(repo_id)?;
    let args = AuditArgs::new()
        .number("index", spec.index as i64)
        .flag("restoreIndex", spec.restore_index)
        .text("action", "pop");
    let spec = spec.into_spec(true);

    run_stash_action(&state, &app, repo_id, op_type::STASH_APPLY, args, || {
        state.stash_service().pop(repo_id, &spec)
    })
}

/// 丢弃某条 stash（不可逆）。写操作：快照 + 审计。
#[tauri::command]
pub fn git_stash_drop(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    index: usize,
) -> AppResult<StashDiscardDto> {
    let repo_id = require_repo(repo_id)?;
    // 被丢 oid 先记进 args：它是"用户绕过界面自己 git 操作"时的自救线索，
    // 因此不能只有 index——那条记录在丢弃后就什么都不剩了
    let target_oid = state
        .stash_service()
        .list(repo_id)?
        .into_iter()
        .find(|entry| entry.index == index)
        .map(|entry| entry.oid);
    let mut args = AuditArgs::new()
        .number("index", index as i64)
        .text("scope", "one");
    if let Some(oid) = &target_oid {
        args = args.text("oid", oid);
    }

    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::STASH_DROP).with_args(args),
        || {
            // T3.11 起 drop 也打快照。T2.8 当初不打，理由是"工作区快照找不回
            // stash 内容"；从快照会记下栈上每一条的 oid（引用类事实）之后，
            // 回滚可以用 `git stash store` 把它们重新登记——那条理由不再成立，
            // 而"丢掉的东西回不来"是用户最不能接受的一种不可逆。
            let snapshot_id = stash_snapshot(&state, repo_id);
            let outcome = state.stash_service().drop_one(repo_id, index)?;
            emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
            Ok(StashDiscardDto {
                dropped: outcome.dropped,
                snapshot_id,
            })
        },
    )
}

/// 丢弃全部 stash（不可逆）。写操作：快照 + 审计。
#[tauri::command]
pub fn git_stash_clear(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
) -> AppResult<StashDiscardDto> {
    let repo_id = require_repo(repo_id)?;
    // 同 drop：clear 之前把将丢的 oid 清单记进 args（上限内）
    let entries = state.stash_service().list(repo_id)?;
    let mut args = AuditArgs::new()
        .text("scope", "all")
        .number("count", entries.len() as i64);
    for entry in entries.iter().take(16) {
        args = args.text("oid", &entry.oid);
    }

    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::STASH_DROP).with_args(args),
        || {
            let snapshot_id = stash_snapshot(&state, repo_id);
            let outcome = state.stash_service().clear(repo_id)?;
            emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
            Ok(StashDiscardDto {
                dropped: outcome.dropped,
                snapshot_id,
            })
        },
    )
}

/// 丢弃 stash 之前的快照（T3.11）。
///
/// 归到 `PreWorktreeChange`：drop / clear **不动 HEAD、也不动工作区**，
/// 变的只是"。git 里的 stash 栈"——回滚一份这样的快照不需要移动 HEAD，
/// 界面上的说明也就该是"把储藏放回去"，而不是"回到某个提交"。
fn stash_snapshot(state: &State<'_, AppState>, repo_id: i64) -> Option<i64> {
    crate::snapshots::snapshot_before(state, repo_id, SnapshotKind::PreWorktreeChange)
}

/// 从某条 stash 创建分支并应用它（会切换分支）。写操作：快照 + 审计。
#[tauri::command]
pub fn git_stash_branch(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: StashBranchRequest,
) -> AppResult<NewBranchDto> {
    let repo_id = require_repo(repo_id)?;
    let args = AuditArgs::new()
        .number("index", spec.index as i64)
        .text("branch", &spec.name);

    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::STASH_BRANCH).with_args(args),
        || {
            let branch = state
                .stash_service()
                .branch(repo_id, spec.index, &spec.name)?;
            // 切换了分支 + 工作区变了
            emit_changed(&app, repo_id, WatchKind::Large, Vec::new());
            Ok(NewBranchDto { branch })
        },
    )
}

/// 应用/弹出储藏的公共尾巴：冲突时也要通知界面刷新（仓库进了冲突状态）。
fn run_stash_action(
    state: &State<'_, AppState>,
    app: &AppHandle,
    repo_id: i64,
    op_type: &'static str,
    args: AuditArgs,
    run: impl FnOnce() -> AppResult<StashOutcome>,
) -> AppResult<StashOutcome> {
    audit::record(
        state,
        AuditEntry::new(repo_id, op_type).with_args(args),
        || {
            let outcome = run()?;
            // 冲突时仓库进了冲突状态、工作区也变了：两种事件都要发
            emit_changed(app, repo_id, WatchKind::Workspace, Vec::new());
            if outcome.has_conflicts() {
                emit_changed(app, repo_id, WatchKind::Refs, Vec::new());
            }
            Ok(outcome)
        },
    )
}

// ---------------------------------------------------------------- 拣选 / 反转

/// 拣选提交（区间可行）。写操作：快照 + 审计。冲突时返回冲突清单。
#[tauri::command]
pub fn git_cherry_pick(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: CherryPickRequest,
) -> AppResult<MergeOutcome> {
    let repo_id = require_repo(repo_id)?;
    let args = AuditArgs::new()
        .text("revision", &spec.revision)
        .flag("recordSource", spec.record_source)
        .flag("noCommit", spec.no_commit);
    let spec = spec.into_spec();

    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::CHERRY_PICK).with_args(args),
        || {
            let outcome = state.history_ops_service().cherry_pick(repo_id, &spec)?;
            emit_changed(&app, repo_id, WatchKind::Large, Vec::new());
            Ok(outcome)
        },
    )
}

/// 反转提交（区间可行）。写操作：快照 + 审计。
#[tauri::command]
pub fn git_revert(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: RevertRequest,
) -> AppResult<MergeOutcome> {
    let repo_id = require_repo(repo_id)?;
    let args = AuditArgs::new()
        .text("revision", &spec.revision)
        .number("mainline", spec.mainline.map_or(0, i64::from))
        .flag("noCommit", spec.no_commit);
    let spec = spec.into_spec();

    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::REVERT).with_args(args),
        || {
            let outcome = state.history_ops_service().revert(repo_id, &spec)?;
            emit_changed(&app, repo_id, WatchKind::Large, Vec::new());
            Ok(outcome)
        },
    )
}

// ---------------------------------------------------------------- 重置

/// 生成重置计划（只读，不写仓库）。
#[tauri::command]
pub fn git_reset_prepare(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: ResetRequest,
) -> AppResult<ResetPlanDto> {
    let repo_id = require_repo(repo_id)?;
    let plan = state
        .history_ops_service()
        .reset_prepare(repo_id, &spec.into_spec())?;
    Ok(ResetPlanDto::from(plan))
}

/// 执行重置计划。写操作：快照（services）+ 审计。
///
/// `confirmation`：`--hard` 时必须等于计划要求的确认词
/// （见 `ResetPlanDto.requiresConfirmation`）。
#[tauri::command]
pub fn git_reset_execute(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    plan_id: String,
    confirmation: Option<String>,
) -> AppResult<ResetOutcomeDto> {
    let repo_id = require_repo(repo_id)?;
    let args = AuditArgs::new()
        .text("plan", &plan_id)
        .text("confirmation", confirmation.as_deref().unwrap_or_default());

    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::RESET).with_args(args),
        || {
            let outcome = state.history_ops_service().reset_execute(
                repo_id,
                &plan_id,
                confirmation.as_deref(),
            )?;
            // 工作区、索引与 HEAD 都可能变了
            emit_changed(&app, repo_id, WatchKind::Large, Vec::new());
            Ok(ResetOutcomeDto::from(outcome))
        },
    )
}

// ---------------------------------------------------------------- reflog

/// reflog（新的在前）。只读。
#[tauri::command(async)]
pub fn git_reflog(
    state: State<'_, AppState>,
    repo_id: i64,
    limit: Option<usize>,
) -> AppResult<Vec<ReflogEntry>> {
    let repo_id = require_repo(repo_id)?;
    // 上限不是"怕多"，而是 reflog 在大仓库上确实可能有几十万条
    let limit = limit.unwrap_or(100).clamp(1, 1_000);
    state.history_ops_service().reflog(repo_id, limit)
}

/// 把 reflog 里的某一条恢复成**新分支**（最安全的恢复方式）。写操作：审计。
#[tauri::command]
pub fn git_reflog_create_branch(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    index: usize,
    name: String,
) -> AppResult<NewBranchDto> {
    let repo_id = require_repo(repo_id)?;
    let args = AuditArgs::new()
        .number("index", index as i64)
        .text("branch", &name);

    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::REFLOG_BRANCH).with_args(args),
        || {
            let branch = state
                .history_ops_service()
                .create_branch_from_reflog(repo_id, index, &name)?;
            emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
            Ok(NewBranchDto { branch })
        },
    )
}
