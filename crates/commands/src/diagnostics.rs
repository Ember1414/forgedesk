//! 诊断引擎的 IPC 桥接（T5.5/T5.6）。
//!
//! 诊断本身是纯函数（[`forgedesk_diagnostics::rules::diagnose`]）；本层只做
//! 参数收敛与覆盖目录的装配。stderr 是**不可信输入**：截断后传入，
//! 且本命令不写日志（用户粘贴的 stderr 可能含敏感内容，落盘交给脱敏层）。

use forgedesk_diagnostics::rules::DiagContext;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use forgedesk_domain::AppResult;

/// 诊断上下文（缺省字段 = 不关心）。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiagContextDto {
    /// 操作类型（push / pull / commit / checkout …）。
    pub op_type: Option<String>,
    /// 当前分支是否有上游。
    pub upstream: bool,
    /// 是否处于分离 HEAD。
    pub detached: bool,
    /// 是否浅克隆。
    pub shallow: bool,
}

/// stderr 上限：诊断只需要报错的关键片段，超大输入截断（8KB 足够）。
pub const DIAGNOSTIC_STDERR_LIMIT: usize = 8 * 1024;

/// 诊断一段 stderr。
///
/// 能力等级：`ReadOnly`（纯计算 + 读取可选的覆盖目录）。
#[tauri::command]
pub fn system_diagnose_error(
    app: AppHandle,
    stderr: String,
    context: Option<DiagContextDto>,
) -> AppResult<forgedesk_diagnostics::rules::DiagnosticReport> {
    let truncated: String = stderr.chars().take(DIAGNOSTIC_STDERR_LIMIT).collect();
    let dto = context.unwrap_or_default();
    let ctx = DiagContext {
        op_type: dto.op_type.filter(|op| !op.is_empty() && op.len() <= 64),
        upstream: dto.upstream,
        detached: dto.detached,
        shallow: dto.shallow,
    };

    // 运行时覆盖目录（任务书"不发版修规则"）：app_config_dir()/diagnostics/，
    // 目录不存在 = 纯内嵌规则（绝大多数用户路径）。
    let override_dir = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("diagnostics"));

    Ok(forgedesk_diagnostics::diagnose(
        &truncated,
        &ctx,
        override_dir.as_deref(),
    ))
}

/// 诊断报告的 i18n key 清单（设置页"诊断历史"用；T5.6 消费）。
///
/// 为什么要显式收集：报告里的 key 分散在 primary / alternatives / fixes，
/// 前端做"是否已诊断过"或导出时需要一份扁平清单。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagKeysSummary {
    /// 命中的最高置信度规则 id（无命中为 null）。
    pub primary: Option<String>,
    /// 其余命中的规则 id。
    pub alternatives: Vec<String>,
}

/// 诊断命中的规则 id 清单（"诊断历史"的轻量指纹，T5.6 消费）。
///
/// 能力等级：`ReadOnly`（纯计算，不走覆盖目录——历史指纹要求与在线展示
/// 使用同一份内嵌规则，否则不可比）。
#[tauri::command]
pub fn system_diagnose_keys(
    stderr: String,
    context: Option<DiagContextDto>,
) -> AppResult<DiagKeysSummary> {
    let truncated: String = stderr.chars().take(DIAGNOSTIC_STDERR_LIMIT).collect();
    let dto = context.unwrap_or_default();
    let ctx = DiagContext {
        op_type: dto.op_type.filter(|op| !op.is_empty() && op.len() <= 64),
        upstream: dto.upstream,
        detached: dto.detached,
        shallow: dto.shallow,
    };
    let report = forgedesk_diagnostics::diagnose(&truncated, &ctx, None);
    Ok(DiagKeysSummary {
        primary: report.primary.map(|d| d.id),
        alternatives: report.alternatives.into_iter().map(|d| d.id).collect(),
    })
}
