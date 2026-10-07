//! 审计命令与写操作的统一拦截（M1 / T1.11）。
//!
//! | 命令 | 能力等级 | 说明 |
//! | --- | --- | --- |
//! | [`audit_list`] | `ReadOnly` | 分页查询操作历史（可按仓库 / 类型 / 时间筛） |
//! | [`audit_export`] | `ReadOnly`（写文件） | 导出 CSV / JSON 到用户选定的路径（未指定则写临时目录） |
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
    /// 写好的文件路径（用户选定的，或未指定时临时目录里的）。
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
        // 原始审计查询不做"危险/可回滚/关键词"这三项收窄：那些是操作历史页的筛选
        ..OperationQuery::default()
    };

    let page = state.audit_service().query(&query)?;
    Ok(AuditPageDto {
        total: page.total,
        entries: page.records.into_iter().map(to_dto).collect(),
    })
}

/// 操作历史的筛选条件（T3.10）。
///
/// 每个字段都是"可选收窄"，与 `OperationQuery` 同一套语义。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OperationFiltersDto {
    /// 只查某一类操作（稳定短名）。
    pub op_type: Option<String>,
    /// 只看"危险操作"（清单在 `services::audit::DANGEROUS_OP_TYPES`）。
    pub only_dangerous: bool,
    /// 只看当时留下了回滚点的记录。
    pub only_reversible: bool,
    /// 关键词（匹配参数摘要与 stderr 摘要）。
    pub keyword: Option<String>,
}

/// 一条操作记录 + 它**此刻**是否仍可回滚。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationHistoryEntryDto {
    /// 记录本体（字段平铺：前端把一条记录当普通审计行用）。
    #[serde(flatten)]
    pub operation: AuditEntryDto,
    /// 现在还能不能回滚：记录标记为可回滚 **且** 快照的锚点此刻仍在。
    pub can_rollback: bool,
}

/// 一页操作历史。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationHistoryDto {
    /// 满足条件的总数（不受分页影响）。
    pub total: i64,
    /// 当前页。
    pub entries: Vec<OperationHistoryEntryDto>,
}

/// 操作历史（T3.10）。能力等级：`ReadOnly`。
///
/// 与 [`audit_list`] 的分工：那是**原始审计查询**（设置页的审计面板，面向排查），
/// 本命令面向**操作历史页**——它多回答一个问题："那条记录的回滚点，现在还作数吗？"
/// 锚点会消失（外部 clone、`git gc`、手工删 ref），而记录还在；不核对就给出一排
/// "回滚"按钮，用户点下去只会收到一个到不了的目标。
#[tauri::command]
pub fn operation_history(
    state: State<'_, AppState>,
    repo_id: i64,
    filters: Option<OperationFiltersDto>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> AppResult<OperationHistoryDto> {
    if repo_id <= 0 {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "repoId must be a positive record id",
        ));
    }
    collect_history(
        &state.audit_service(),
        state.snapshots.as_ref(),
        repo_id,
        filters.unwrap_or_default(),
        limit,
        offset,
    )
}

/// 操作历史的查询主逻辑（命令壳只负责参数校验）。
///
/// 为什么要抽出来：命令函数带 `State<'_, AppState>`，测试里构造它要把整个宿主
/// 状态搬出来；而这段逻辑只依赖两个能力——审计查询与快照管理。抽出来之后，
/// "记录 + 回滚点是否仍作数"的判据可以在真实仓库上直接测。
pub fn collect_history(
    audit: &forgedesk_services::AuditLog<'_>,
    snapshots: &dyn forgedesk_snapshot::SnapshotManager,
    repo_id: i64,
    filters: OperationFiltersDto,
    limit: Option<usize>,
    offset: Option<usize>,
) -> AppResult<OperationHistoryDto> {
    let query = OperationQuery {
        repo_id: Some(repo_id),
        op_type: normalize_op_type(filters.op_type)?,
        op_types: if filters.only_dangerous {
            Some(
                forgedesk_services::audit::DANGEROUS_OP_TYPES
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            )
        } else {
            None
        },
        keyword: filters.keyword.filter(|value| !value.trim().is_empty()),
        from_ms: None,
        to_ms: None,
        reversible_only: filters.only_reversible,
        limit: limit
            .unwrap_or(OperationQuery::DEFAULT_LIMIT)
            .clamp(1, OperationQuery::MAX_LIMIT),
        offset: offset.unwrap_or(0),
    };

    let page = audit.query(&query)?;

    // 一次问清这一页引用到的所有快照（去重）：逐条问会把 50 条记录变成 50 次 git 调用
    let mut ids: Vec<forgedesk_snapshot::SnapshotId> = page
        .records
        .iter()
        .filter_map(|record| record.snapshot_id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let restorable: std::collections::HashSet<forgedesk_snapshot::SnapshotId> = if ids.is_empty() {
        std::collections::HashSet::new()
    } else {
        snapshots
            .restorable(repo_id, &ids)
            .map_err(|error| forgedesk_domain::AppError::new(error.code(), error.message()))?
            .into_iter()
            .collect()
    };

    Ok(OperationHistoryDto {
        total: page.total,
        entries: page
            .records
            .into_iter()
            .map(|record| {
                let can_rollback = record.reversible
                    && record
                        .snapshot_id
                        .is_some_and(|id| restorable.contains(&id));
                OperationHistoryEntryDto {
                    operation: to_dto(record),
                    can_rollback,
                }
            })
            .collect(),
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
    target_path: Option<String>,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
) -> AppResult<AuditExportDto> {
    let format = AuditExportFormat::parse(&format)?;
    let target_path = validate_export_target(target_path.as_deref(), format)?;
    let query = OperationQuery {
        repo_id,
        op_type: normalize_op_type(op_type.clone())?,
        from_ms,
        to_ms,
        offset: 0,
        limit: OperationQuery::MAX_LIMIT,
        // 导出是"把所有匹配的记录倒出去"，不套用操作历史页的收窄项
        ..OperationQuery::default()
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
        target_path,
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

/// 校验用户选定的导出目标路径（T7.6）。
///
/// 两条规则，都是"早失败好过晚失败"：
///
/// - **必须是绝对路径**：相对路径的基准是进程当前目录——那是用户看不见的东西，
///   写出来的文件会落在没人预期的地方；
/// - **扩展名必须与格式一致**：保存对话框里用户可能把 `.csv` 改成 `.json`，
///   内容按所选格式生成而扩展名相反，双击打开时就是"文件损坏"。
///
/// **刻意不做**的事：不检查父目录是否存在（真的写失败会返回带路径的 `STORAGE`
/// 错误，那条信息比这里猜的准）；不检查目标是否落在仓库工作树内——保存对话框
/// 已经明确显示了位置，替用户否决一个他/她明确选择的位置是越权。
fn validate_export_target(
    target: Option<&str>,
    format: AuditExportFormat,
) -> AppResult<Option<std::path::PathBuf>> {
    let Some(raw) = target else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        // 未指定 = 退回临时目录（旧行为），因此空串按"没给"处理
        return Ok(None);
    }

    let path = std::path::PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "the export target must be an absolute path",
        )
        .with_hint(trimmed.to_owned()));
    }

    let actual = path
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if actual != format.extension() {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "the export file extension does not match the chosen format",
        )
        .with_hint(format.extension().to_owned()));
    }

    Ok(Some(path))
}

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
    use super::{normalize_op_type, to_dto, validate_export_target};
    use forgedesk_services::AuditExportFormat;
    use forgedesk_storage::OperationRecord;

    /// 相对路径的基准是进程当前目录（用户看不见它），因此一律拒绝。
    #[test]
    fn a_relative_export_target_is_refused() {
        let error = validate_export_target(Some("reports/audit.csv"), AuditExportFormat::Csv)
            .expect_err("相对路径必须被拒绝");
        assert_eq!(error.code, forgedesk_domain::ErrorCode::Validation);
    }

    /// 扩展名与格式不符会被双击打开时报"文件损坏"，因此在写之前就拦下。
    #[test]
    fn the_extension_must_match_the_format() {
        let temp = std::env::temp_dir();
        let csv = temp.join("audit.csv");
        let json = temp.join("audit.json");

        assert!(validate_export_target(csv.to_str(), AuditExportFormat::Csv).is_ok());
        assert!(validate_export_target(json.to_str(), AuditExportFormat::Json).is_ok());
        assert!(validate_export_target(json.to_str(), AuditExportFormat::Csv).is_err());
    }

    /// 大小写不敏感：Windows 上用户很容易保存成 `报表.CSV`。
    #[test]
    fn the_extension_check_ignores_case() {
        let upper = std::env::temp_dir().join("audit.CSV");
        assert!(validate_export_target(upper.to_str(), AuditExportFormat::Csv).is_ok());
    }

    /// 没给（或只给了空白）= 退回临时目录，是合法用法而不是错误。
    #[test]
    fn a_blank_target_falls_back_to_the_temp_directory() {
        assert!(validate_export_target(None, AuditExportFormat::Csv)
            .expect("缺省不是错误")
            .is_none());
        assert!(validate_export_target(Some("   "), AuditExportFormat::Csv)
            .expect("空白不是错误")
            .is_none());
    }

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
