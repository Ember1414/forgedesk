//! Dashboard 聚合命令（T4.11）：多仓库状态、待审 PR、CI 失败一屏可见。
//!
//! 聚合的请求预算在 services 层控制（每仓库 2 个请求、目标 ≤10，见
//! [`forgedesk_services::host_repos::HostRepoService::dashboard`]）；
//! 本层只做参数校验与转发。单仓库失败在 services 层降级（`errors` +
//! 摘要 `None`），不打断其他仓库——命令层拿到的一定是 `Ok`。

use serde::{Deserialize, Serialize};
use tauri::State;

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_services::host_repos::RepoDashboard;

use crate::state::AppState;

/// 聚合目标上限（与 services 层的截断一致；前端先截，这里兜底校验）。
pub const MAX_DASHBOARD_TARGETS: usize = 10;

/// `repo_dashboard` 的单个目标。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardTargetDto {
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub repo: String,
}

/// `repo_dashboard` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardRequest {
    /// 站点。
    pub host: String,
    /// 聚合目标（1..=10 个）。
    pub targets: Vec<DashboardTargetDto>,
    /// 本地仓库 id（绑定账号解析来源；可省）。
    #[serde(default)]
    pub repo_id: Option<i64>,
}

/// 聚合结果（`RepoDashboard` 原样透出，provider/services 已是 camelCase）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardReportDto {
    /// 每仓库一行。
    pub repos: Vec<RepoDashboard>,
}

/// 多仓库聚合视图。能力等级：`Network`。
#[tauri::command]
pub async fn repo_dashboard(
    state: State<'_, AppState>,
    request: DashboardRequest,
) -> AppResult<DashboardReportDto> {
    let host = request.host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "provider host must not be empty",
        ));
    }
    if request.targets.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "dashboard needs at least one target repository",
        ));
    }
    if request.targets.len() > MAX_DASHBOARD_TARGETS {
        return Err(AppError::new(
            ErrorCode::Validation,
            format!("dashboard accepts at most {MAX_DASHBOARD_TARGETS} targets"),
        ));
    }
    let targets = request
        .targets
        .iter()
        .map(|target| {
            if target.owner.trim().is_empty() || target.repo.trim().is_empty() {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    "dashboard target owner and repo must not be empty",
                ));
            }
            Ok(forgedesk_services::host_repos::DashboardTarget {
                owner: target.owner.trim().to_owned(),
                repo: target.repo.trim().to_owned(),
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    let repos = state
        .host_repos
        .dashboard(&host, request.repo_id, &targets)
        .await?;
    Ok(DashboardReportDto { repos })
}
