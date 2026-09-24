//! 操作快照与回滚：破坏性操作前的状态打点与一致性校验。
//!
//! # 当前进度（M1 / T1.7）
//!
//! 本 crate 目前只有**接口**与一个"未启用"的占位实现：[`SnapshotManager`] 的形状
//! 由 T1.7 定下来（提交链路要调用它，并把结果写进审计），真正的实现属于 M3 / T1.9
//! —— 记录 `head_oid` / `index_tree_oid` / `reflog_ref`、`restore` 与一致性校验。
//!
//! **为什么先定接口**：提交里"执行前先打点"的**顺序**必须在第一次写出提交链路时就位
//! （先快照、后执行、失败可回滚）。等 T1.9 再插入这一步，意味着要把已经稳定下来的
//! 执行流程拆开重排——那是更容易出错、也更难验证的改动方式。
//!
//! # 与红线 R7 的关系（当前版本的诚实说明）
//!
//! R7 要求的完整链条是"计划预览 → 快照 → 执行 → 可回滚"。M3 之前，
//! 提交的实际安全网是**前两步**：用户先看到完整的计划（文件清单、等价命令、钩子），
//! 执行前再核对索引指纹是否变过，变了就拒绝执行。快照位缺的那一环由审计记录
//! 如实标注（`operation_records.snapshot_id` 为 `NULL`），不会假装有快照。

#![forbid(unsafe_code)]

use std::path::Path;

/// 快照 id（`snapshots` 表主键）。
pub type SnapshotId = i64;

/// 创建快照的场景。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    /// 用户手动创建。
    Manual,
    /// 提交之前。
    PreCommit,
    /// 同步（拉取 / 合并 / 变基）之前。
    PreSync,
    /// 会移动 HEAD 的操作（重置、切换分支）之前。
    PreHeadMove,
}

impl SnapshotKind {
    /// 稳定的短名（写进 `snapshots.kind` 与审计；前端据此走 i18n）。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::PreCommit => "pre-commit",
            Self::PreSync => "pre-sync",
            Self::PreHeadMove => "pre-head-move",
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

/// 建快照失败的原因。
///
/// 现在只有一个变体：M3 之前不存在"部分失败"的路径。T1.9 会按真实失败原因细分
/// （磁盘满 / 索引被锁 / 对象库不可写），那时再拆——现在拆只是猜。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// 建快照时出错。
    Failed(String),
}

impl SnapshotError {
    /// 开发者可读的描述（英文；用户可见文案由前端按错误码走 i18n）。
    pub fn message(&self) -> String {
        match self {
            Self::Failed(detail) => format!("snapshot creation failed: {detail}"),
        }
    }
}

/// 快照管理器。
///
/// 调用方（`services` 层）只依赖这个 trait，因此 T1.9 换成真实实现时，
/// 上层代码不需要改——这正是先定接口的意义。
///
/// 要求 `Debug`：命令层的 `AppState` 持有 `Arc<dyn SnapshotManager>` 并派生 `Debug`
/// （启动与排查时能一眼看清状态对象的组成）。实现者多写一个 `#[derive(Debug)]`
/// 远比让状态结构丢掉可调试性划算。
pub trait SnapshotManager: Send + Sync + std::fmt::Debug {
    /// 创建一个快照。
    ///
    /// 返回 `Ok(None)` 表示**这次没有创建快照**（当前是"未启用"），调用方应当
    /// 继续执行、并在审计里如实记为"无快照"；返回 `Err` 才是真的失败了。
    fn create(&self, request: &SnapshotRequest<'_>) -> Result<Option<SnapshotId>, SnapshotError>;
}

/// 未启用快照的实现（M3 / T1.9 之前的默认注入）。
///
/// 它**不会**假装成功：`Ok(None)` 明确表示"没有快照"，与"创建了快照"在审计里
/// 是可区分的两种记录。
#[derive(Debug, Default)]
pub struct NoopSnapshotManager;

impl SnapshotManager for NoopSnapshotManager {
    fn create(&self, _request: &SnapshotRequest<'_>) -> Result<Option<SnapshotId>, SnapshotError> {
        Ok(None)
    }
}

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{NoopSnapshotManager, SnapshotKind, SnapshotManager, SnapshotRequest};
    use std::path::Path;

    #[test]
    fn the_noop_manager_reports_no_snapshot_instead_of_pretending_to_succeed() {
        let manager = NoopSnapshotManager;
        let request = SnapshotRequest {
            repo_id: 1,
            workdir: Path::new("/tmp/repo"),
            label: "pre-commit",
            kind: SnapshotKind::PreCommit,
        };

        // `None` 是"没有快照"，与 "Some(id)" 在审计里必须可区分
        assert_eq!(manager.create(&request).unwrap(), None);
    }

    #[test]
    fn snapshot_kinds_have_stable_keys() {
        assert_eq!(SnapshotKind::PreCommit.key(), "pre-commit");
        assert_eq!(SnapshotKind::Manual.key(), "manual");
        assert_eq!(SnapshotKind::PreSync.key(), "pre-sync");
        assert_eq!(SnapshotKind::PreHeadMove.key(), "pre-head-move");
    }
}
