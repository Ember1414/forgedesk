//! 提交用例：`prepare` → `execute` 两段式（T1.7）。
//!
//! # 为什么是两段而不是一次调用
//!
//! 见 `domain::git::commit_plan` 的模块头：用户看到的与最终执行的必须是**同一份数据**。
//! 本模块补上两件只有编排层才做得到的事：
//!
//! 1. **计划只活在内存里**（[`CommitPlanRegistry`]）：它只活 5 分钟，属于"这个窗口里的
//!    这一次操作"。落库要额外处理过期清理与多窗口语义，而收益为零。
//! 2. **执行前重新核对索引指纹**：界面上的 diff 是几秒前取的，而"用户刚在终端里又
//!    `git add` 了一次"是完全正常的用法。指纹不一致就当计划过期（`PLAN_STALE`），
//!    让用户重新看一眼再决定——这比"照旧执行一份已经不对应的计划"安全得多。
//!
//! # 安全网（红线 R7）的当前形态
//!
//! 快照由 [`SnapshotManager`] 负责，M3 / T1.9 之前注入的是"未启用"实现：它返回
//! `Ok(None)`，审计里如实记为 `snapshot_id = NULL` 与 `reversible = false`。
//! 因此当前的安全网是**计划预览 + 索引指纹校验**，而不是假装有快照。
//!
//! # 审计不能省，也不许失败得悄无声息
//!
//! 每次执行都写一条 `operation_records`：先 `begin`（这样崩在中间也留得下痕迹），
//! 再 `finish`。反过来，**收尾写失败不改变提交结果**——用户看到成没成，必须来自 git。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use forgedesk_diagnostics::sanitize_log;
use forgedesk_domain::git::{
    compose_message, equivalent_command, review_message, ChangeKind, CommitPlan, CommitSpec,
    EntryKind, EquivalentCommandInput, LogQuery, PlannedFile, RepoId, RepoPath, SignMode,
    Signature, StatusQuery, StatusReport, EMPTY_TREE_OID,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode, FixAction};
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_git_engine::probe::executable_commit_hooks;
use forgedesk_snapshot::{SnapshotKind, SnapshotManager, SnapshotRequest};
use forgedesk_storage::{NewOperation, OperationOutcome, OperationStore, RepositoryStore};

use crate::engines::GitEngines;
use crate::repository::{system_clock, MillisClock};

/// 准备提交计划的输入（命令层已把 IPC 参数收敛到这个形状）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareRequest {
    /// 提交信息首行。
    pub subject: String,
    /// 正文（空字符串与 `None` 等价）。
    pub description: Option<String>,
    /// 是否 amend 上一个提交。
    pub amend: bool,
    /// 是否追加 `Signed-off-by`。
    pub sign_off: bool,
    /// 是否跳过钩子。
    pub no_verify: bool,
    /// GPG 签名模式。
    pub sign: SignMode,
    /// 覆盖作者身份。
    pub author: Option<Signature>,
}

/// 一次成功提交的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOutcome {
    /// 存储层记录 id（命令层据此发布 `repo:changed`）。
    pub repo_id: i64,
    /// 新提交的 oid。
    pub oid: String,
    /// 提交信息首行（界面提示用）。
    pub subject: String,
    /// 本次提交关联的快照 id（M3 之前恒为 `None`）。
    pub snapshot_id: Option<i64>,
    /// 本次提交涉及的路径（命令层据此发布 `repo:changed`）。
    pub paths: Vec<RepoPath>,
}

/// 提交信息的风格提示。
///
/// 全部是**本地规则**（最近 20 条提交、分支名前缀），不涉及任何模型推理（红线 R1）：
/// 它的用途是"提醒用户这个仓库习惯怎么写"，而不是替用户写。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MessageHint {
    /// 最近若干条提交的首行（新 → 旧）。
    pub recent_messages: Vec<String>,
    /// 建议的信息模板前缀（例如分支是 `feat/login` 时给出 `feat: `）。
    pub template: Option<String>,
    /// 从分支名推断出的风格前缀。
    pub branch_style: Option<String>,
}

/// 待执行的提交计划（进程内、带有效期）。
///
/// # 为什么用 `take`（一次性）而不是反复读取
///
/// 一个计划只对应"用户按下的那一次提交"。允许重复执行意味着"同一个 plan_id 提交两次"，
/// 第二次必然失败（索引指纹已变），但用户会看到两次报错、不知道哪次生效。
/// 取走即失效把这件事变成不可能的用法。
#[derive(Debug, Default)]
pub struct CommitPlanRegistry {
    plans: Mutex<HashMap<String, CommitPlan>>,
}

impl CommitPlanRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前持有的计划数（诊断与测试用）。
    pub fn len(&self) -> usize {
        lock(&self.plans).len()
    }

    /// 是否没有待执行的计划。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 生成一个计划 id（不登记）。
    ///
    /// 让调用方先拿 id 再构造计划：计划里必须带着自己的 id，
    /// 否则就得"先插入、再把 id 回填进去"，那是两次写同一份数据。
    pub fn next_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// 登记一份计划（键是它自己的 `plan_id`）。
    fn insert(&self, plan: CommitPlan) {
        lock(&self.plans).insert(plan.plan_id.clone(), plan);
    }

    /// 取走一份计划（取走即失效）。
    fn take(&self, plan_id: &str) -> Option<CommitPlan> {
        lock(&self.plans).remove(plan_id)
    }

    /// 清理已过期的计划。
    ///
    /// 用户反复"改一下信息、重新预览"会留下多份计划；虽然它们都会过期，
    /// 但没人清理就一直是内存里的垃圾。
    fn prune(&self, now_ms: i64) {
        lock(&self.plans).retain(|_, plan| !plan.is_expired(now_ms));
    }
}

/// 提交用例。
pub struct CommitService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
    operations: OperationStore<'a>,
    snapshots: &'a dyn SnapshotManager,
    plans: &'a CommitPlanRegistry,
    clock: MillisClock,
}

impl<'a> CommitService<'a> {
    /// 组装服务。
    ///
    /// 不需要 [`crate::repository::OpenRepoRegistry`]：提交不要求"仓库在本会话里打开过"
    /// （`repo_id` 已经指向磁盘上的一个仓库），多传一个用不上的依赖只会让人以为它有用。
    pub fn new(
        engines: &'a GitEngines,
        store: RepositoryStore<'a>,
        operations: OperationStore<'a>,
        snapshots: &'a dyn SnapshotManager,
        plans: &'a CommitPlanRegistry,
    ) -> Self {
        Self {
            engines,
            store,
            operations,
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

    /// 解析记录 id 为工作区路径；记录不存在时返回 `NOT_FOUND`。
    pub fn resolve_workdir(&self, repo_id: i64) -> AppResult<std::path::PathBuf> {
        let record = self.store.find_by_id(repo_id)?.ok_or(
            AppError::new(ErrorCode::NotFound, "the repository record does not exist")
                .with_detail(format!("repo_id: {repo_id}")),
        )?;
        Ok(std::path::PathBuf::from(record.path))
    }

    // ---------------------------------------------------------------- prepare

    /// 生成一份提交计划（不写仓库）。
    ///
    /// 这里做的每一步都是"能在真正执行前发现的问题"：信息为空、索引里什么都没有、
    /// 有未解决的冲突。放到执行时才发现，用户就白填了一遍信息。
    pub fn prepare(&self, repo_id: i64, request: &PrepareRequest) -> AppResult<CommitPlan> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);
        let reader = self.engines.read();

        let message = compose_message(&request.subject, request.description.as_deref());
        let review = review_message(&message);
        if review.is_blocking() {
            // hint 只放数据（字段名），建议性文案由前端按 code 给（CODING_STYLE §2.1）
            return Err(
                AppError::new(ErrorCode::Validation, "the commit message is empty")
                    .with_hint("message"),
            );
        }

        // 冲突检查必须排在指纹之前：`git write-tree` 遇到未合并条目会**直接失败**，
        // 那样用户拿到的是一句 git 的内部报错，而不是"先解决冲突"这个真正的原因
        let index = self.index_snapshot(&repo)?;
        if index.conflicted {
            return Err(AppError::new(
                ErrorCode::GitConflict,
                "the repository has unresolved conflicts, so it cannot be committed",
            ));
        }

        let index_fingerprint = reader.index_tree(&repo)?;
        let head_tree = reader.head_tree(&repo)?;
        let head_oid = self.head_oid(&repo)?;

        let hooks = self.commit_hooks(&repo)?;
        let equivalent = equivalent_command(EquivalentCommandInput {
            message: &message,
            amend: request.amend,
            sign: request.sign,
            sign_off: request.sign_off,
            no_verify: request.no_verify,
            author: request.author.as_ref(),
            file_count: index.staged.len(),
        });

        let plan = CommitPlan {
            plan_id: self.plans.next_id(),
            repo_id,
            files: index.staged,
            message,
            description: request.description.clone(),
            author: request.author.clone(),
            sign: request.sign,
            sign_off: request.sign_off,
            no_verify: request.no_verify,
            amend: request.amend,
            hooks,
            equivalent_command: equivalent,
            head_oid,
            index_fingerprint,
            created_at_ms: self.now(),
            review,
        };

        // 空提交判定放在最后：它要用到指纹与 HEAD 的树（前面的查询已经拿到）
        if plan.stages_nothing(head_tree.as_deref()) {
            return Err(empty_commit_error(
                &plan.index_fingerprint,
                head_tree.as_deref(),
            ));
        }

        // 顺手清理过期计划：用户反复"改信息 → 重新预览"会留下多份，
        // 没人清理就一直是内存里的垃圾
        self.plans.prune(plan.created_at_ms);
        self.plans.insert(plan.clone());
        Ok(plan)
    }

    // ---------------------------------------------------------------- execute

    /// 执行一份计划。
    pub fn execute(&self, plan_id: &str) -> AppResult<CommitOutcome> {
        let now = self.now();
        let plan = self
            .plans
            .take(plan_id)
            .ok_or_else(|| stale_plan("the commit plan is not known or was already used", None))?;

        if plan.is_expired(now) {
            return Err(stale_plan(
                "the commit plan has expired",
                Some(plan.repo_id),
            ));
        }

        let workdir = self.resolve_workdir(plan.repo_id)?;
        let repo = RepoId::new(workdir.clone());

        // 界面上那份 diff 是几秒前取的：这里必须确认索引还是同一份
        let current = self.engines.read().index_tree(&repo)?;
        if current != plan.index_fingerprint {
            return Err(stale_plan(
                "the index changed after the plan was prepared",
                Some(plan.repo_id),
            )
            .with_detail(format!(
                "prepared: {}\ncurrent: {current}",
                plan.index_fingerprint
            )));
        }

        let args_json = audit_args(&plan);
        let operation_id = self.operations.begin(&NewOperation {
            repo_id: plan.repo_id,
            op_type: "commit",
            args_json: Some(&args_json),
            started_at_ms: now,
        })?;

        let snapshot_id = self.create_snapshot(&plan, &workdir);

        let spec = CommitSpec {
            message: plan.message.clone(),
            // 提交索引里的**全部内容**：T1.7 的语义是"提交我暂存的东西"，
            // 路径级提交（`--only`）等到有"只提交某几个文件"的需求时再说
            paths: Vec::new(),
            amend: plan.amend,
            // amend 时允许"索引没有变化"：用户只改信息是完全正常的用法
            allow_empty: plan.amend,
            sign: plan.sign.as_commit_flag(),
            sign_off: plan.sign_off,
            author: plan.author.clone(),
            no_verify: plan.no_verify,
        };

        match self.engines.write().commit(&repo, spec) {
            Ok(oid) => {
                self.finish_operation(operation_id, Some(0), None, snapshot_id);
                Ok(CommitOutcome {
                    repo_id: plan.repo_id,
                    oid,
                    subject: plan.review.subject.clone(),
                    snapshot_id,
                    paths: plan.files.iter().map(|file| file.path.clone()).collect(),
                })
            }
            Err(error) => {
                let error = classify_commit_failure(error, &plan.hooks, plan.no_verify);
                let summary = error
                    .detail
                    .clone()
                    .unwrap_or_else(|| error.message.clone());
                self.finish_operation(operation_id, Some(1), Some(&summary), snapshot_id);
                Err(error)
            }
        }
    }

    // ---------------------------------------------------------------- 提示

    /// 提交信息的风格提示（最近提交 + 分支名前缀）。
    pub fn message_hint(&self, repo_id: i64) -> AppResult<MessageHint> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);
        let reader = self.engines.read();

        let page = reader.log(&repo, LogQuery::new().with_limit(20))?;
        let status = reader.status(&repo, &StatusQuery::default())?;
        let branch_style = status.branch.head.as_deref().and_then(branch_prefix);

        Ok(MessageHint {
            recent_messages: page
                .items
                .iter()
                .map(|commit| commit.subject.clone())
                .filter(|subject| !subject.trim().is_empty())
                .collect(),
            template: branch_style.as_ref().map(|style| format!("{style}: ")),
            branch_style,
        })
    }

    // ---------------------------------------------------------------- 内部

    /// 索引里相对 HEAD 有变化的文件，以及是否存在未合并条目。
    fn index_snapshot(&self, repo: &RepoId) -> AppResult<IndexSnapshot> {
        let report: StatusReport = self.engines.read().status(repo, &StatusQuery::default())?;

        let conflicted = report
            .entries
            .iter()
            .any(|entry| entry.kind == EntryKind::Unmerged);

        let staged = report
            .entries
            .iter()
            .filter(|entry| {
                // 只认"索引侧有变化"的已跟踪条目：未跟踪文件不在索引里
                // （porcelain 对它们不给 `XY` 位，用 `kind` 排除更直白）
                matches!(
                    entry.kind,
                    EntryKind::Ordinary | EntryKind::RenamedOrCopied | EntryKind::Unmerged
                ) && entry.index_status != ChangeKind::Unmodified
            })
            .map(|entry| PlannedFile {
                path: entry.path.clone(),
                index_status: entry.index_status,
            })
            .collect();

        Ok(IndexSnapshot { staged, conflicted })
    }

    /// 当前 HEAD 的 oid（空仓库为 `None`）。
    ///
    /// 用 `log(limit=1)` 而不是 `rev-parse HEAD`：T1.2 已经统一了"空仓库 → 空页"
    /// 这个语义（两个引擎一致），而 `rev-parse` 在空仓库上是错误路径。
    fn head_oid(&self, repo: &RepoId) -> AppResult<Option<String>> {
        let page = self
            .engines
            .read()
            .log(repo, LogQuery::new().with_limit(1))?;
        Ok(page.items.first().map(|commit| commit.oid.clone()))
    }

    /// 本次提交会实际执行的钩子。
    fn commit_hooks(&self, repo: &RepoId) -> AppResult<Vec<String>> {
        // 钩子目录必须问 git（`core.hooksPath` 会让它不再是 `.git/hooks`）
        let hooks_dir = self.engines.read().hooks_dir(repo)?;
        Ok(executable_commit_hooks(&hooks_dir))
    }

    /// 建快照；失败不阻断提交，但绝不静默。
    fn create_snapshot(&self, plan: &CommitPlan, workdir: &Path) -> Option<i64> {
        let request = SnapshotRequest {
            repo_id: plan.repo_id,
            workdir,
            label: SnapshotKind::PreCommit.key(),
            kind: SnapshotKind::PreCommit,
        };
        match self.snapshots.create(&request) {
            Ok(id) => id,
            Err(error) => {
                // M3 之前本来就没有快照；M3 之后这里会变成"用户可见的降级提示"。
                // 现在至少保证：审计里的 reversible 会因此为 false，日志里能查到原因。
                tracing::warn!(
                    repo_id = plan.repo_id,
                    error = %error.message(),
                    "提交前未能创建快照"
                );
                None
            }
        }
    }

    /// 给审计记录收尾；失败只记日志。
    ///
    /// 为什么不把错误往上抛：审计写不进去不该改变"提交到底成没成"这个事实——
    /// 用户看到的结果必须来自 git，而不是来自本地数据库。
    fn finish_operation(
        &self,
        operation_id: i64,
        exit_code: Option<i32>,
        stderr_summary: Option<&str>,
        snapshot_id: Option<i64>,
    ) {
        let outcome = OperationOutcome {
            ended_at_ms: self.now(),
            exit_code,
            stderr_summary,
            snapshot_id,
            reversible: snapshot_id.is_some(),
        };
        if let Err(error) = self.operations.finish(operation_id, &outcome) {
            tracing::warn!(
                operation_id,
                error = %error,
                "操作记录收尾失败（提交本身的结果不受影响）"
            );
        }
    }
}

/// 索引侧的一次快照（一次 `status` 调用同时回答两个问题）。
struct IndexSnapshot {
    staged: Vec<PlannedFile>,
    conflicted: bool,
}

/// 从分支名推断提交信息的风格前缀（纯本地规则）。
///
/// `feat/login-page` → `feat`；`main` / `fix-typo`（没有斜杠）→ `None`。
/// 只认"斜杠前那一段"这一个规则：更复杂的推断（Conventional Commits 的 scope、
/// 团队自定义前缀）靠猜只会猜错，而错的提示比没有提示更烦人。
fn branch_prefix(branch: &str) -> Option<String> {
    let (prefix, _) = branch.split_once('/')?;
    let prefix = prefix.trim();
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_owned())
    }
}

/// 审计用的参数摘要（**已脱敏**，红线 R8）。
///
/// 不含提交信息全文：审计要能一眼看出"这是什么操作"，而不是把几 KB 的信息抄一遍。
fn audit_args(plan: &CommitPlan) -> String {
    let summary = serde_json::json!({
        "subject": plan.review.subject,
        "files": plan.files.len(),
        "amend": plan.amend,
        "signOff": plan.sign_off,
        "noVerify": plan.no_verify,
        "sign": plan.sign.key(),
        "hooks": plan.hooks,
        "indexFingerprint": plan.index_fingerprint,
    });
    sanitize_log(&summary.to_string())
}

/// 计划过期 / 失效。
fn stale_plan(reason: &str, repo_id: Option<i64>) -> AppError {
    let error = AppError::new(ErrorCode::PlanStale, reason);
    match repo_id {
        // 只有拿到 repo_id 时才给动作：FixAction 的语义是"点击后调用某个命令"，
        // 少了参数它会点不动，那种按钮比没有按钮更糟
        Some(id) => error.with_action(
            FixAction::new(
                "refresh-status",
                "errors.actions.refresh",
                "workspace_status",
            )
            .with_args(serde_json::json!({ "repoId": id })),
        ),
        None => error,
    }
}

/// 没有可提交的内容。
fn empty_commit_error(index_fingerprint: &str, head_tree: Option<&str>) -> AppError {
    let reason = if index_fingerprint == EMPTY_TREE_OID {
        "the index is empty"
    } else {
        "the index is identical to HEAD"
    };
    AppError::new(ErrorCode::EmptyCommit, reason).with_detail(format!(
        "index tree: {index_fingerprint}\nhead tree: {}",
        head_tree.unwrap_or("(no commit yet)")
    ))
}

/// 把提交失败分类成"钩子拒绝"或原样返回。
///
/// # 判定依据（以及为什么不能更严）
///
/// git **没有**给"钩子失败"稳定的退出码或专用字段：钩子脚本的 stdout/stderr 与 git
/// 自己的报错混在同一个管道里，措辞还随版本与钩子框架变化（husky 打印
/// `pre-commit hook exited with code 1`，裸 git 可能只把脚本输出原样透传）。
///
/// 因此这里用三个可观察到的事实一起判定：
///
/// 1. 本次提交**会**执行钩子（`no_verify` 为假且仓库里存在可执行钩子）；
/// 2. git 非零退出且 stderr 非空；
/// 3. 输出里没有"明显不是钩子"的信号（见 [`NOT_A_HOOK`]）。
///
/// 这样判的风险是"有钩子的仓库里，别的原因导致的失败被说成钩子拒绝"。它的代价
/// 是可接受的：`detail` 始终保留原始输出，展开就能看到真相（例如 `index.lock`
/// 会被第 3 条排除掉）。反过来，要求输出里必须出现 `hook` 字样会漏掉大量真实情况
/// ——钩子脚本完全可以只打印一句自己的错误信息。
fn classify_commit_failure(error: AppError, hooks: &[String], no_verify: bool) -> AppError {
    if no_verify || hooks.is_empty() || !looks_like_hook_output(&error) {
        return error;
    }

    AppError::new(
        ErrorCode::HookRejected,
        "a commit hook rejected the commit",
    )
    // detail 保留原始输出：钩子到底说了什么，只有它知道
    .with_detail(error.detail.unwrap_or_default())
    // hint 只放数据（钩子名清单），文案由前端按 code 给
    .with_hint(hooks.join(", "))
    .with_retryable(false)
}

/// 明显不是钩子输出（而是 git 自己的致命错误）的信号。
///
/// 用"排除法"而不是"包含法"：钩子的输出是不可穷举的任意文本，而 git 的致命错误
/// 有稳定的措辞。
const NOT_A_HOOK: [&str; 3] = [
    // 索引被别的进程锁住（另一个 git 命令正在跑）
    "index.lock",
    // 无法创建对象 / 锁文件（磁盘、权限）
    "unable to create",
    // 工作目录不是仓库（调用方的路径解析错了）
    "not a git repository",
];

/// 失败输出是否"像是钩子干的"。
fn looks_like_hook_output(error: &AppError) -> bool {
    let Some(detail) = error.detail.as_deref() else {
        return false;
    };
    let text = detail.trim();
    if text.is_empty() {
        return false;
    }
    let lowered = text.to_ascii_lowercase();
    !NOT_A_HOOK.iter().any(|signal| lowered.contains(signal))
}

/// 取锁并在 poisoned 时继续（一次提交写不坏内存里的计划表）。
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
