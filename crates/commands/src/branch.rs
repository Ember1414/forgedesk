//! 分支与标签管理命令（T2.5）。
//!
//! # 能力等级
//!
//! 写操作一律：审计（`record`）→（危险时）快照 → 执行 → 失效。快照在
//! services 层做（`BranchService` 持有 `SnapshotManager`），本层只负责审计与
//! 参数形状；`branch_list` / `tag_list` / `branch_compare` 是只读。
//!
//! # 二次确认的语义
//!
//! `force` 类操作的"确认"是**显式参数**（`confirmForce` / `confirmUnmerged`），
//! 不是前端口头承诺：services 层在没有确认参数时拒绝执行——API 直调绕不过 UI。

use forgedesk_domain::AppResult;
use forgedesk_services::{AuditArgs, AuditEntry, BranchComparison, BranchDeleteOutcome};
use tauri::State;

use crate::audit;
use crate::state::AppState;

// 注意：git_branch_list 已在 history.rs（T2.3 的筛选下拉）——同一个命令、
// 同一个名字，不再重复定义（tauri::command 的宏名冲突会被编译器抓住）。

/// 标签列表。
#[tauri::command(async)]
pub fn git_tag_list(
    state: State<'_, AppState>,
    repo_id: i64,
) -> AppResult<Vec<forgedesk_domain::git::Tag>> {
    state.branch_service().tag_list(repo_id)
}

/// 比较两个分支：ahead/behind 与 a 独有的提交（删除确认清单的数据源）。
#[tauri::command(async)]
pub fn git_branch_compare(
    state: State<'_, AppState>,
    repo_id: i64,
    a: String,
    b: String,
) -> AppResult<BranchComparison> {
    state.branch_service().branch_compare(repo_id, &a, &b)
}

/// 新建分支（可同时切换）。写操作：审计。
#[tauri::command]
pub fn git_branch_create(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: forgedesk_domain::git::BranchCreateSpec,
) -> AppResult<()> {
    let args = AuditArgs::new()
        .text("name", &spec.name)
        .flag("checkout", spec.checkout);
    audit::record(
        &state,
        AuditEntry::new(repo_id, "branch.create").with_args(args),
        || state.branch_service().branch_create(repo_id, &spec),
    )
}

/// 切换分支（三策略）。Force 必须带 `confirmForce`；Stash 由服务层编排。
#[tauri::command]
pub fn git_branch_switch(
    state: State<'_, AppState>,
    repo_id: i64,
    target: String,
    strategy: SwitchStrategyDto,
    confirm_force: Option<bool>,
) -> AppResult<Option<i64>> {
    let strategy = strategy.into_domain();
    let args = AuditArgs::new()
        .text("target", &target)
        .text("strategy", strategy_label(strategy))
        .flag("confirmForce", confirm_force.unwrap_or(false));
    audit::record(
        &state,
        AuditEntry::new(repo_id, "branch.switch").with_args(args),
        || {
            state.branch_service().switch_by_id(
                repo_id,
                &target,
                strategy,
                confirm_force.unwrap_or(false),
            )
        },
    )
}

/// 重命名分支。写操作：审计。
#[tauri::command]
pub fn git_branch_rename(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: forgedesk_domain::git::BranchRenameSpec,
) -> AppResult<()> {
    let args = AuditArgs::new()
        .text("old", &spec.old)
        .text("new", &spec.new);
    audit::record(
        &state,
        AuditEntry::new(repo_id, "branch.rename").with_args(args),
        || state.branch_service().branch_rename(repo_id, &spec),
    )
}

/// 删除一批分支。未合并强删必须带 `confirmUnmerged`。
#[tauri::command]
pub fn git_branch_delete(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: forgedesk_domain::git::BranchDeleteSpec,
    confirm_unmerged: Option<bool>,
) -> AppResult<BranchDeleteOutcome> {
    let args = AuditArgs::new()
        .text("names", &spec.names.join(","))
        .flag("force", spec.force);
    audit::record(
        &state,
        AuditEntry::new(repo_id, "branch.delete").with_args(args),
        || {
            state
                .branch_service()
                .branch_delete(repo_id, &spec, confirm_unmerged.unwrap_or(false))
        },
    )
}

/// 设置 / 取消上游。写操作：审计。
#[tauri::command]
pub fn git_branch_set_upstream(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: forgedesk_domain::git::BranchSetUpstreamSpec,
) -> AppResult<()> {
    let args = AuditArgs::new()
        .text("branch", &spec.branch)
        .text("upstream", spec.upstream.as_deref().unwrap_or("<unset>"));
    audit::record(
        &state,
        AuditEntry::new(repo_id, "branch.setUpstream").with_args(args),
        || state.branch_service().branch_set_upstream(repo_id, &spec),
    )
}

/// 创建标签。写操作：审计。
#[tauri::command]
pub fn git_tag_create(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: forgedesk_domain::git::TagCreateSpec,
) -> AppResult<()> {
    let args = AuditArgs::new()
        .text("name", &spec.name)
        .flag("annotated", spec.message.is_some())
        .flag("force", spec.force);
    audit::record(
        &state,
        AuditEntry::new(repo_id, "tag.create").with_args(args),
        || state.branch_service().tag_create(repo_id, &spec),
    )
}

/// 删除一批标签。写操作：审计。
#[tauri::command]
pub fn git_tag_delete(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: forgedesk_domain::git::TagDeleteSpec,
) -> AppResult<()> {
    let args = AuditArgs::new().text("names", &spec.names.join(","));
    audit::record(
        &state,
        AuditEntry::new(repo_id, "tag.delete").with_args(args),
        || state.branch_service().tag_delete(repo_id, &spec),
    )
}

/// 切换策略的 wire 形状（serde 小驼峰；与 docs/API.md 契约一致）。
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SwitchStrategyDto {
    /// 先储藏本地改动，切换后恢复（默认策略）。
    Stash,
    /// 强制切换：丢弃本地改动（危险，需确认 + 快照）。
    Force,
    /// 仅在工作区干净时切换，否则拒绝。
    Clean,
}

impl SwitchStrategyDto {
    fn into_domain(self) -> forgedesk_domain::git::SwitchStrategy {
        match self {
            Self::Stash => forgedesk_domain::git::SwitchStrategy::Stash,
            Self::Force => forgedesk_domain::git::SwitchStrategy::Force,
            Self::Clean => forgedesk_domain::git::SwitchStrategy::Clean,
        }
    }
}

fn strategy_label(strategy: forgedesk_domain::git::SwitchStrategy) -> &'static str {
    match strategy {
        forgedesk_domain::git::SwitchStrategy::Stash => "stash",
        forgedesk_domain::git::SwitchStrategy::Force => "force",
        forgedesk_domain::git::SwitchStrategy::Clean => "clean",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::SwitchStrategyDto;

    /// 切换策略的 wire 形状（camelCase 反序列化）。
    #[test]
    fn switch_strategy_deserializes_from_camel_case() {
        assert!(matches!(
            serde_json::from_str::<SwitchStrategyDto>("\"stash\"").unwrap(),
            SwitchStrategyDto::Stash
        ));
        assert!(matches!(
            serde_json::from_str::<SwitchStrategyDto>("\"force\"").unwrap(),
            SwitchStrategyDto::Force
        ));
        assert!(matches!(
            serde_json::from_str::<SwitchStrategyDto>("\"clean\"").unwrap(),
            SwitchStrategyDto::Clean
        ));
        assert!(serde_json::from_str::<SwitchStrategyDto>("\"Force\"").is_err());
    }
}
