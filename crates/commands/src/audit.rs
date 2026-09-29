//! 审计命令与写操作的统一拦截（M1 / T1.11）。
//!
//! | 命令 | 能力等级 | 说明 |
//! | --- | --- | --- |
//! | [`audit_list`] | `ReadOnly` | 分页查询操作历史（可按仓库 / 类型 / 时间筛） |
//! | [`audit_export`] | `ReadOnly`（写临时文件） | 导出 CSV / JSON，返回文件路径 |
//! | [`audit_prune`] | `Mutating` | 按保留策略清理旧记录 |
//!
//! # 为什么拦截放在命令层
//!
//! 红线 R7 要求"每一次写操作都留痕"。写操作的**入口**在这一层：每个
//! `#[tauri::command]` 就是一次"用户按下按钮"，它知道自己的操作类型、仓库 id
//! 与参数形状（只有这一层同时看得见 IPC 参数与用例服务）。
//!
//! 放在服务层要改十几个方法签名，而且**服务的方法可以被别的服务调用**
//! （`StagingService::stage` 的文件粒度会转调 `WorkspaceService::stage`），
//! 那样每一层都要决定"这一跳要不要记"，最后必然是重复记录或者漏记。
//!
//! 唯一的例外是 `commit_execute`：它的参数摘要（提交信息首行、文件数、钩子清单）
//! 只存在于服务内部的计划里，因此那条记录仍由 `CommitService` 自己写
//! （同一个 [`AuditLog`] 类型，不是两套机制）。

use forgedesk_domain::AppResult;
use forgedesk_services::audit::op_type;
use forgedesk_services::{AuditArgs, AuditEntry, AuditExportFormat, AuditRetention};
use forgedesk_storage::{OperationQuery, OperationRecord};
use serde::Serialize;
use tauri::State;

use crate::state::AppState;

/// 一条审计记录的 DTO。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntryDto {
    /// 主键。
    pub id: i64,
    /// 仓库 id（全局操作为 0）。
    pub repo_id: i64,
    /// 操作类型（稳定短名，见 `services::audit::op_type`）。
    pub op_type: String,
    /// 参数摘要（已脱敏的 JSON 字符串；界面展示为一行）。
    pub args_json: Option<String>,
    /// 开始时间（Unix 毫秒）。
    pub started_at_ms: Option<i64>,
    /// 结束时间（Unix 毫秒）；`null` = 没有正常收尾（崩溃特征）。
    pub ended_at_ms: Option<i64>,
    /// 耗时（毫秒）；未收尾时为 `null`。
    pub duration_ms: Option<i64>,
    /// git 的退出码。
    pub exit_code: Option<i32>,
    /// 结果短名：`ok` / `failed` / `running`（界面据此选 i18n 文案与颜色）。
    pub result: String,
    /// 失败摘要（已脱敏）。
    pub stderr_summary: Option<String>,
    /// 关联的快照 id。
    pub snapshot_id: Option<i64>,
    /// 是否可回滚（有快照才为真）。
    pub reversible: bool,
}

/// 一页审计记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditPageDto {
    /// 满足条件的总数（不受分页影响）。
    pub total: i64,
    /// 当前页。
    pub entries: Vec<AuditEntryDto>,
}

/// 导出结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditExportDto {
    /// 写好的文件路径（本任务只写临时目录；用户选目录要等 M7 的文件对话框）。
    pub path: String,
    /// 导出条数。
    pub rows: usize,
    /// 格式短名。
    pub format: String,
}

/// 清理结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditPruneDto {
    /// 删除条数。
    pub removed: usize,
    /// 本次使用的保留天数。
    pub retention_days: i64,
    /// 本次使用的保留条数上限。
    pub retention_rows: i64,
}

/// 分页查询操作历史。能力等级：`ReadOnly`。
#[tauri::command]
pub fn audit_list(
    state: State<'_, AppState>,
    repo_id: Option<i64>,
    op_type: Option<String>,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> AppResult<AuditPageDto> {
    let query = OperationQuery {
        repo_id,
        op_type: normalize_op_type(op_type)?,
        from_ms,
        to_ms,
        limit: limit.unwrap_or(OperationQuery::DEFAULT_LIMIT),
        offset: offset.unwrap_or(0),
    };

    let page = state.audit_service().query(&query)?;
    Ok(AuditPageDto {
        total: page.total,
        entries: page.records.into_iter().map(to_dto).collect(),
    })
}

/// 导出操作历史到临时文件。能力等级：`ReadOnly`（只读库里已有的记录）。
///
/// 导出本身也会被记录（见 [`op_type::AUDIT_EXPORT`]）：**谁把历史倒出去过**
/// 是审计的一部分，否则"谁能看到历史"这件事在系统里没有证据。
#[tauri::command]
pub fn audit_export(
    state: State<'_, AppState>,
    repo_id: Option<i64>,
    op_type: Option<String>,
    format: String,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
) -> AppResult<AuditExportDto> {
    let format = AuditExportFormat::parse(&format)?;
    let query = OperationQuery {
        repo_id,
        op_type: normalize_op_type(op_type.clone())?,
        from_ms,
        to_ms,
        offset: 0,
        limit: OperationQuery::MAX_LIMIT,
    };

    let audit = state.audit_service();
    let mut args = AuditArgs::new()
        .text("format", format.key())
        .number("fromMs", from_ms.unwrap_or(0))
        .number("toMs", to_ms.unwrap_or(0));
    if let Some(repo_id) = repo_id {
        args = args.number("repoId", repo_id);
    }

    let result = audit.export(&forgedesk_services::AuditExportRequest {
        query,
        format,
        now_ms: forgedesk_services::audit::now_ms(),
    });

    // 导出（成功或失败）都要留痕：失败同样是"有人试图导出"
    audit.note(
        &AuditEntry::new(
            repo_id.unwrap_or(forgedesk_services::GLOBAL_REPO_ID),
            op_type::AUDIT_EXPORT,
        )
        .with_args(args),
        &result
            .as_ref()
            .map(|export| export.path.display().to_string())
            .map_err(|error| error.clone()),
    );

    result.map(|export| AuditExportDto {
        path: export.path.display().to_string(),
        rows: export.rows,
        format: format.key().to_owned(),
    })
}

/// 按保留策略清理旧记录。能力等级：`Mutating`（删除本地记录）。
///
/// 清理动作本身要留痕：删除历史的人不该是匿名的。
#[tauri::command]
pub fn audit_prune(state: State<'_, AppState>) -> AppResult<AuditPruneDto> {
    let retention = AuditRetention::load(&state.database);
    let audit = state.audit_service();

    let result = audit.prune(&retention.policy(forgedesk_services::audit::now_ms()));
    audit.note(
        &AuditEntry::new(forgedesk_services::GLOBAL_REPO_ID, op_type::AUDIT_PRUNE).with_args(
            AuditArgs::new()
                .number(
                    "removed",
                    result.as_ref().map_or(0, |removed| *removed as i64),
                )
                .number("keepDays", retention.days)
                .number("keepRows", retention.rows),
        ),
        &result
            .as_ref()
            .map(|removed| removed.to_string())
            .map_err(|error| error.clone()),
    );

    Ok(AuditPruneDto {
        removed: result?,
        retention_days: retention.days,
        retention_rows: retention.rows,
    })
}

/// 包住一次写操作：开始记录 → 执行 → 收尾。
///
/// 审计写不进去时照常执行（见 [`forgedesk_services::AuditLog::begin`] 的说明）：
/// 它是安全网，不是闸门。
pub(crate) fn record<T>(
    state: &AppState,
    entry: AuditEntry<'_>,
    run: impl FnOnce() -> AppResult<T>,
) -> AppResult<T>
where
    T: serde::Serialize,
{
    record_with(&state.audit_service(), entry, run)
}

/// [`record`] 的实现体：只依赖审计服务，因此可以脱离 `AppState` 测试。
///
/// 拦截逻辑本身（"成功与失败都要收尾"）是最值得钉住的部分：它错了会表现为
/// "审计表里一堆没有结束时间的记录"，而那正是"应用崩了"的信号——假警报
/// 会让真正的崩溃淹没在噪音里。
///
/// # `snapshot_id` 从结果里提取（T2.10 修复）
///
/// 结果的 serde 形状是 IPC 契约（camelCase）：凡是带 `snapshotId` 字段的结果
/// （reset / cherry-pick / revert / stash save / apply / pop…），这个 id 会被
/// 写进 `operation_records.snapshot_id`，`reversible` 据此为真——这是"遍历
/// operation_records 与 snapshots 的关联性"能成立的前提。此前这里硬编码
/// `None`：服务层明明打了快照、前端也拿得到 id，唯独审计表断链，
/// "这个操作能不能回滚"在操作历史里永远是"否"。
///
/// 提取走**契约形状**而不是给每个 DTO 加 trait：没有 `snapshotId` 字段的
/// 结果（`()`、清单、只读查询）自然提取为 `None`；哪个操作该带 id 而没带，
/// 由 `write_ops_safety_net` 集成测试的关联断言兜底。
pub fn record_with<T>(
    audit: &forgedesk_services::AuditLog<'_>,
    entry: AuditEntry<'_>,
    run: impl FnOnce() -> AppResult<T>,
) -> AppResult<T>
where
    T: serde::Serialize,
{
    let operation = audit.begin(&entry);
    let result = run();
    if let Some(operation) = operation {
        let snapshot_id = match &result {
            Ok(value) => serde_json::to_value(value)
                .ok()
                .and_then(|json| json.get("snapshotId").and_then(serde_json::Value::as_i64)),
            Err(_) => None,
        };
        operation.finish(&result, snapshot_id);
    }
    result
}

/// 启动时按保留策略清理一次（尽力而为）。
///
/// 为什么不放在 `audit_list` 里：查询路径上做删除会让"看一眼历史"变成写操作，
/// 而且第一次打开设置页就要等一次删除。启动时清理是成本最低的时机。
pub fn prune_on_startup(state: &AppState) -> usize {
    let retention = AuditRetention::load(&state.database);
    match state
        .audit_service()
        .prune(&retention.policy(forgedesk_services::audit::now_ms()))
    {
        Ok(removed) => {
            if removed > 0 {
                tracing::info!(removed, days = retention.days, "按保留策略清理了操作记录");
            }
            removed
        }
        Err(error) => {
            tracing::warn!(error = %error.message, "启动时的操作记录清理失败");
            0
        }
    }
}

// ---------------------------------------------------------------- 内部

/// 空字符串/空白按"不筛选"处理：界面上的下拉框是"全部"时不该发一个空字符串过来。
fn normalize_op_type(op_type: Option<String>) -> AppResult<Option<String>> {
    match op_type {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Ok(None),
        Some(value) => Ok(Some(value)),
    }
}

fn to_dto(record: OperationRecord) -> AuditEntryDto {
    let duration_ms = match (record.started_at_ms, record.ended_at_ms) {
        (Some(started), Some(ended)) => Some(ended - started),
        _ => None,
    };
    let result = match (record.ended_at_ms, record.exit_code) {
        (None, _) => "running",
        (Some(_), Some(0)) => "ok",
        (Some(_), _) => "failed",
    };

    AuditEntryDto {
        id: record.id,
        repo_id: record.repo_id,
        op_type: record.op_type,
        args_json: record.args_json,
        started_at_ms: record.started_at_ms,
        ended_at_ms: record.ended_at_ms,
        duration_ms,
        exit_code: record.exit_code,
        result: result.to_owned(),
        stderr_summary: record.stderr_summary,
        snapshot_id: record.snapshot_id,
        reversible: record.reversible,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{normalize_op_type, to_dto};
    use forgedesk_storage::OperationRecord;

    #[test]
    fn the_result_and_duration_are_derived_from_the_stored_columns() {
        let record = OperationRecord {
            id: 1,
            repo_id: 2,
            op_type: "commit".to_owned(),
            args_json: Some(r#"{"subject":"x"}"#.to_owned()),
            started_at_ms: Some(1_000),
            ended_at_ms: Some(1_250),
            exit_code: Some(0),
            stderr_summary: None,
            snapshot_id: Some(9),
            reversible: true,
        };

        let dto = to_dto(record);
        assert_eq!(dto.duration_ms, Some(250));
        assert_eq!(dto.result, "ok");
        assert!(dto.reversible);
    }

    #[test]
    fn an_unfinished_record_reads_as_running() {
        let record = OperationRecord {
            id: 1,
            repo_id: 2,
            op_type: "commit".to_owned(),
            args_json: None,
            started_at_ms: Some(1_000),
            ended_at_ms: None,
            exit_code: None,
            stderr_summary: None,
            snapshot_id: None,
            reversible: false,
        };

        let dto = to_dto(record);
        // "只有开始没有结束"是崩溃的可观测特征，界面必须能一眼看出来
        assert_eq!(dto.result, "running");
        assert_eq!(dto.duration_ms, None);
    }

    #[test]
    fn a_failed_operation_is_distinguishable_from_a_running_one() {
        let record = OperationRecord {
            id: 1,
            repo_id: 2,
            op_type: "commit".to_owned(),
            args_json: None,
            started_at_ms: Some(1_000),
            ended_at_ms: Some(1_010),
            exit_code: Some(1),
            stderr_summary: Some("hook failed".to_owned()),
            snapshot_id: None,
            reversible: false,
        };

        assert_eq!(to_dto(record).result, "failed");
    }

    #[test]
    fn a_blank_type_filter_means_no_filter() {
        assert_eq!(normalize_op_type(None).unwrap(), None);
        assert_eq!(normalize_op_type(Some("  ".to_owned())).unwrap(), None);
        assert_eq!(
            normalize_op_type(Some("commit".to_owned())).unwrap(),
            Some("commit".to_owned())
        );
    }

    #[test]
    fn a_wrapped_operation_leaves_one_finished_record_with_the_arguments() {
        let database = forgedesk_storage::Database::open_in_memory().unwrap();
        forgedesk_storage::migrate(&database).unwrap();
        let audit =
            forgedesk_services::AuditLog::new(forgedesk_storage::OperationStore::new(&database));

        let outcome = super::record_with(
            &audit,
            super::AuditEntry::new(3, super::op_type::STAGE).with_args(
                super::AuditArgs::new()
                    .text("kind", "files")
                    .number("paths", 2),
            ),
            || Ok::<_, forgedesk_domain::AppError>(42),
        );

        assert_eq!(outcome.unwrap(), 42);
        let page = audit
            .query(&forgedesk_storage::OperationQuery::default())
            .unwrap();
        assert_eq!(page.total, 1, "一次写操作只留一条记录");
        let record = &page.records[0];
        assert_eq!(record.repo_id, 3);
        assert_eq!(record.exit_code, Some(0));
        assert!(record.ended_at_ms.is_some(), "必须收尾（否则会被读成崩溃）");
        assert!(record.args_json.as_ref().unwrap().contains("\"paths\":2"));
    }

    #[test]
    fn a_failed_operation_is_still_closed_with_the_error_message() {
        let database = forgedesk_storage::Database::open_in_memory().unwrap();
        forgedesk_storage::migrate(&database).unwrap();
        let audit =
            forgedesk_services::AuditLog::new(forgedesk_storage::OperationStore::new(&database));

        let error = forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "the selection is out of range",
        );
        let outcome = super::record_with(
            &audit,
            super::AuditEntry::new(1, super::op_type::UNSTAGE),
            || Err::<(), _>(error.clone()),
        );

        assert_eq!(
            outcome.unwrap_err().code,
            forgedesk_domain::ErrorCode::Validation
        );
        let record = audit
            .query(&forgedesk_storage::OperationQuery::default())
            .unwrap()
            .records
            .remove(0);
        assert_eq!(record.exit_code, Some(1));
        assert!(record.ended_at_ms.is_some(), "失败也必须收尾");
        assert!(record
            .stderr_summary
            .unwrap()
            .contains("the selection is out of range"));
    }
}
