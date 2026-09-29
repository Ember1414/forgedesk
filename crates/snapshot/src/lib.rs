//! 操作快照与回滚：破坏性操作前的状态打点与一致性校验。
//!
//! # 红线 R7 的最后一环（M1 / T1.9）
//!
//! 完整链条是"计划预览 → 快照 → 执行 → 可回滚"。T1.7 之前实际的安全网只有前两步
//! （快照位由 [`NoopSnapshotManager`] 如实标注"没有"）；现在 [`RefSnapshotManager`]
//! 补上了后两步：每个快照是一组**可独立校验的 git 事实**（HEAD oid、索引树、
//! 自定义 ref 锚点），而不是一份需要解释的备份文件。
//!
//! # 为什么是"自定义 ref"而不是备份目录或 reflog
//!
//! - **备份目录**（把整个工作区拷走）是 T3.8 快照 v2 的事：v1 的破坏性操作路径
//!   （提交、amend）不会删除或覆盖未跟踪文件，记录它们的路径就够定位问题；
//! - **reflog / `HEAD@{n}`** 会被外部操作改写或清空（别的工具、`gc --prune=now`、
//!   clone），不能作为唯一依据——任务定义明确禁止。
//!   自定义 ref `refs/forgedesk/snapshots/<id>` 指向快照时刻的 HEAD 提交，
//!   git 因此**不会**把它当垃圾回收，这是"锚点"二字的含义。
//!
//! # 校验纪律
//!
//! 回滚之后必须**用 git 自己的事实**核对（HEAD oid、`write-tree` 的树 oid），
//! 对不上就返回 [`SnapshotError::RestoreVerify`]，并且**自动恢复到回滚前快照**——
//! 用户要的是"回到过去"，不是"落到一个既不是过去也不是现在的状态"。

#![forbid(unsafe_code)]

use std::path::Path;

use forgedesk_domain::ErrorCode;

/// 快照 id（`snapshots` 表主键）。
pub type SnapshotId = i64;

mod ref_manager;

pub use ref_manager::{
    RefSnapshotManager, DEFAULT_MAX_AGE_DAYS, DEFAULT_MAX_COUNT, SNAPSHOT_REF_PREFIX,
};

/// 快照锚点 ref 的前缀（完整形如 `refs/forgedesk/snapshots/<id>`）。
///
/// 挂在 `refs/forgedesk/` 下而不是 `refs/heads/`：它是**应用的管理数据**，
/// 不该出现在分支列表里，也不该被 `git push --mirror` 之类当成普通分支带走。
pub const SNAPSHOT_REF_NAMESPACE: &str = "refs/forgedesk/snapshots";

/// 创建快照的场景。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    /// 用户手动创建。
    Manual,
    /// 提交之前。
    PreCommit,
    /// 回滚之前——先给"现在"打一个点，回滚错了还能回来。
    PreRestore,
    /// 同步（拉取 / 合并 / 变基）之前。
    PreSync,
    /// 会移动 HEAD 的操作（重置、切换分支）之前。
    PreHeadMove,
    /// 只改动工作区/索引、不动 HEAD 的操作（储藏、应用储藏）之前。
    ///
    /// 与 [`Self::PreHeadMove`] 分开：回滚一份"只动了工作区"的快照不需要移动 HEAD，
    /// 界面上的说明也完全不同（"恢复我的未提交改动" vs "回到某个提交"）。
    PreWorktreeChange,
}

impl SnapshotKind {
    /// 稳定的短名（写进 `snapshots.kind` 与审计；前端据此走 i18n）。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::PreCommit => "pre-commit",
            Self::PreRestore => "pre-restore",
            Self::PreSync => "pre-sync",
            Self::PreHeadMove => "pre-head-move",
            Self::PreWorktreeChange => "pre-worktree-change",
        }
    }
}

/// 创建快照的请求。
#[derive(Debug, Clone, Copy)]
pub struct SnapshotRequest<'a> {
    /// 存储层记录 id（`repositories.id`）。
    pub repo_id: i64,
    /// 仓库工作区（快照要读 HEAD、索引与未跟踪清单）。
    pub workdir: &'a Path,
    /// 展示用的标签（例如 `pre-commit`）。
    pub label: &'a str,
    /// 场景。
    pub kind: SnapshotKind,
}

/// 快照的列表行（给界面看的摘要；恢复细节用 [`SnapshotDiff`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta {
    /// 主键。
    pub id: SnapshotId,
    /// 展示标签。
    pub label: String,
    /// 场景短名。
    pub kind: String,
    /// 快照时刻的 HEAD oid（全量；界面自行截短显示）。
    pub head_oid: String,
    /// 当时的分支名。
    pub branch: Option<String>,
    /// 是否游离 HEAD。
    pub detached: bool,
    /// 创建时间（Unix 毫秒）。
    pub created_at: i64,
}

/// 快照与当前状态的差异摘要。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotDiff {
    /// HEAD 已经不是快照时的位置。
    pub head_changed: bool,
    /// 索引已经不是快照时的树。
    pub index_changed: bool,
    /// 当前的 HEAD oid（空仓库为 `None`）。
    pub current_head_oid: Option<String>,
    /// 当前的索引树；索引里有未合并条目时 `write-tree` 会失败，此时为 `None`
    /// （并保留 `index_changed = true`：读不出树本身就说明状态已经不同）。
    pub current_index_tree_oid: Option<String>,
    /// 锚点 ref 已丢失——这个快照**不可恢复**，只能作为历史记录查看。
    pub ref_missing: bool,
}

/// 一次回滚的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// 被恢复的快照。
    pub restored_snapshot_id: SnapshotId,
    /// 恢复后的 HEAD oid（与快照记录一致，由恢复后的校验保证）。
    pub head_oid: String,
    /// 恢复后的索引树 oid。
    pub index_tree_oid: String,
    /// 回滚前自动打的"保护点"——用户对回滚结果不满意时可以再回到这里。
    pub pre_restore_snapshot_id: Option<SnapshotId>,
    /// 快照时刻的未跟踪文件路径。v1 只记录不恢复（内容备份属 T3.8），
    /// 列在这里是让用户知道"当时有这些文件"。
    pub untracked_paths: Vec<String>,
}

/// 快照的保留策略：**两个条件先到者生效**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// 每个仓库最多保留的条数。
    pub max_count: u32,
    /// 超过这个天数（按创建时间）的快照可被清理。
    pub max_age_days: u32,
}

impl Default for RetentionPolicy {
    /// 任务定义的默认值：每仓库 50 条或 30 天，先到者生效。
    fn default() -> Self {
        Self {
            max_count: DEFAULT_MAX_COUNT,
            max_age_days: DEFAULT_MAX_AGE_DAYS,
        }
    }
}

impl RetentionPolicy {
    /// 截止时间（Unix 毫秒）：早于它的快照可被清理。
    pub fn cutoff_ms(&self, now_ms: i64) -> i64 {
        now_ms - i64::from(self.max_age_days) * 24 * 60 * 60 * 1000
    }
}

/// 快照相关失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// 创建快照失败（读状态、写对象库、落库）。
    Failed(String),
    /// 快照记录不存在，或不属于指定的仓库。
    NotFound(SnapshotId),
    /// 锚点 ref 丢失——快照不可恢复。
    RefMissing {
        /// 快照 id。
        id: SnapshotId,
        /// 本应存在的 ref 名。
        name: String,
    },
    /// 恢复后的校验不通过。**发生时已自动回到回滚前快照**，仓库不会停在中间态。
    RestoreVerify(String),
    /// 恢复过程失败（git 报错等）。发生时同样先尝试回到回滚前快照。
    RestoreFailed(String),
    /// 数据库读写失败。
    Storage(String),
}

impl SnapshotError {
    /// 开发者可读的描述（英文；用户可见文案由前端按错误码走 i18n）。
    pub fn message(&self) -> String {
        match self {
            Self::Failed(detail) => format!("snapshot creation failed: {detail}"),
            Self::NotFound(id) => format!("snapshot {id} does not exist"),
            Self::RefMissing { id, name } => {
                format!("snapshot {id} is not restorable: anchor ref {name} is missing")
            }
            Self::RestoreVerify(detail) => {
                format!("restore verification failed: {detail}")
            }
            Self::RestoreFailed(detail) => format!("restore failed: {detail}"),
            Self::Storage(detail) => format!("snapshot storage failed: {detail}"),
        }
    }

    /// 稳定错误码（命令层转 `AppError` 时用；用户可见文案由前端给）。
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::NotFound(_) | Self::RefMissing { .. } => ErrorCode::NotFound,
            Self::RestoreVerify(_) => ErrorCode::RestoreVerifyFailed,
            Self::Failed(_) | Self::RestoreFailed(_) | Self::Storage(_) => ErrorCode::Internal,
        }
    }
}

/// 快照管理器。
///
/// 调用方（`services` 层）只依赖这个 trait；`create` 的调用点在 T1.7 就已就位
/// （提交执行前），换实现不需要动那条链路。
///
/// 要求 `Debug`：命令层的 `AppState` 持有 `Arc<dyn SnapshotManager>` 并派生 `Debug`
/// （启动与排查时能一眼看清状态对象的组成）。实现者多写一个 `#[derive(Debug)]`
/// 远比让状态结构丢掉可调试性划算。
pub trait SnapshotManager: Send + Sync + std::fmt::Debug {
    /// 创建一个快照，返回它的 id。
    ///
    /// 空仓库（还没有 HEAD 提交）会失败：没有提交就没有可锚定的对象，
    /// 这种状态下"快照"是空洞的——调用方应当继续原操作并自行降级提示。
    fn create(&self, request: &SnapshotRequest<'_>) -> Result<SnapshotId, SnapshotError>;

    /// 某个仓库的快照列表（新的在前）。
    fn list(&self, repo_id: i64, limit: i64) -> Result<Vec<SnapshotMeta>, SnapshotError>;

    /// 回滚到指定快照，返回恢复报告。
    ///
    /// 流程（顺序即语义）：校验锚点 → 给当前状态打"回滚前快照" → `reset --hard`
    /// → `read-tree` 恢复索引 → 用 git 的事实核对 → 对不上就回到回滚前快照。
    fn restore(
        &self,
        repo_id: i64,
        snapshot_id: SnapshotId,
    ) -> Result<RestoreReport, SnapshotError>;

    /// 快照与当前状态的差异摘要（列表页的"这个快照还有意义吗"）。
    fn diff(&self, repo_id: i64, snapshot_id: SnapshotId) -> Result<SnapshotDiff, SnapshotError>;

    /// 按保留策略清理，返回被清理的快照 id。
    fn prune(
        &self,
        repo_id: i64,
        policy: &RetentionPolicy,
    ) -> Result<Vec<SnapshotId>, SnapshotError>;
}

/// 未启用快照的实现。
///
/// 现在只出现在测试里（真实现随 T1.9 落地、由宿主注入）。它**不会**假装成功：
/// `Err(NotFound)` 与"创建了快照"在调用方是可区分的。保留它是因为
/// "需要注入一个必然失败的快照管理器来测降级路径"这件事永远存在。
#[derive(Debug, Default)]
pub struct NoopSnapshotManager;

impl SnapshotManager for NoopSnapshotManager {
    fn create(&self, _request: &SnapshotRequest<'_>) -> Result<SnapshotId, SnapshotError> {
        Err(SnapshotError::Failed(
            "snapshotting is disabled by configuration".to_owned(),
        ))
    }

    fn list(&self, _repo_id: i64, _limit: i64) -> Result<Vec<SnapshotMeta>, SnapshotError> {
        Ok(Vec::new())
    }

    fn restore(
        &self,
        _repo_id: i64,
        snapshot_id: SnapshotId,
    ) -> Result<RestoreReport, SnapshotError> {
        Err(SnapshotError::NotFound(snapshot_id))
    }

    fn diff(&self, _repo_id: i64, snapshot_id: SnapshotId) -> Result<SnapshotDiff, SnapshotError> {
        Err(SnapshotError::NotFound(snapshot_id))
    }

    fn prune(
        &self,
        _repo_id: i64,
        _policy: &RetentionPolicy,
    ) -> Result<Vec<SnapshotId>, SnapshotError> {
        Ok(Vec::new())
    }
}

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        NoopSnapshotManager, RetentionPolicy, SnapshotKind, SnapshotManager, SnapshotRequest,
    };
    use std::path::Path;

    #[test]
    fn the_noop_manager_fails_loudly_instead_of_pretending_to_succeed() {
        let manager = NoopSnapshotManager;
        let request = SnapshotRequest {
            repo_id: 1,
            workdir: Path::new("/tmp/repo"),
            label: "pre-commit",
            kind: SnapshotKind::PreCommit,
        };

        // "未启用"必须是显式的失败：调用方据此走降级提示，而不是以为有快照
        assert!(manager.create(&request).is_err());
        assert!(manager.list(1, 10).unwrap().is_empty());
        assert!(manager
            .prune(1, &RetentionPolicy::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn snapshot_kinds_have_stable_keys() {
        assert_eq!(SnapshotKind::PreCommit.key(), "pre-commit");
        assert_eq!(SnapshotKind::PreRestore.key(), "pre-restore");
        assert_eq!(SnapshotKind::Manual.key(), "manual");
        assert_eq!(SnapshotKind::PreSync.key(), "pre-sync");
        assert_eq!(SnapshotKind::PreHeadMove.key(), "pre-head-move");
    }

    #[test]
    fn the_default_retention_policy_matches_the_task_definition() {
        let policy = RetentionPolicy::default();
        assert_eq!(policy.max_count, 50);
        assert_eq!(policy.max_age_days, 30);

        // 截止时间的数学要经得起时区无关的检查：30 天前
        let cutoff = policy.cutoff_ms(1_700_000_000_000);
        assert_eq!(cutoff, 1_700_000_000_000 - 30 * 24 * 60 * 60 * 1000);
    }
}
