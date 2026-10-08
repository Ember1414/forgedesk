//! 托管平台账号命令（T4.3/T4.4）：登录、列表、删除。
//!
//! # 能力等级与审计
//!
//! | 命令 | 能力等级 | 说明 |
//! | --- | --- | --- |
//! | `account_login_with_pat` | Network | 一次 `/user` 校验；写入凭据库与账号表 |
//! | `account_device_flow_start` | Network | 一次 `/login/device/code`；秘密留在后端会话表 |
//! | `account_device_flow_wait` | Network | **长任务**（走 `JobRunner`）：轮询直至授权/过期/取消 |
//! | `account_list` | ReadOnly | 读 `accounts` 表 |
//! | `account_remove` | Mutating | 删凭据库条目 + 账号行 |
//!
//! 账号操作不触碰任何仓库状态，因此**不走** SnapshotManager（审计也只在
//! 涉及仓库的写操作时要求）；凭据本体去向由 `credentials` 命令组的状态接口如实呈现。
//!
//! # 令牌与 IPC 的边界（红线 R8）
//!
//! PAT 明文随参数进 IPC 是不可避免的（用户在哪里粘贴，令牌就从哪里来），
//! 但它**只进服务层**：不回显、不进日志、不进任何 DTO。
//! Device Flow 更进一步：`device_code` 与轮询令牌从不离开后端——
//! 前端拿到的是 user_code 与验证链接，等待结果经 `job:done` 只带回账号信息。

use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, State};

use forgedesk_domain::AppResult;
use forgedesk_services::accounts::{Account, StartedDeviceFlow};
use secrecy::SecretString;
use serde_json::json;

use crate::jobs::reporter_for;
use crate::state::AppState;

/// 账号信息（不含任何令牌材料）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDto {
    /// 稳定 id。
    pub id: String,
    /// 平台标识（`github`）。
    pub provider: String,
    /// 主机。
    pub host: String,
    /// 登录名。
    pub login: String,
    /// 头像地址。
    pub avatar_url: Option<String>,
    /// 令牌作用域。
    pub scopes: Vec<String>,
    /// 首次登录时间（Unix 毫秒）。
    pub created_at: Option<i64>,
}

impl From<Account> for AccountDto {
    fn from(account: Account) -> Self {
        Self {
            id: account.id,
            provider: account.provider,
            host: account.host,
            login: account.login,
            avatar_url: account.avatar_url,
            scopes: account.scopes,
            created_at: account.created_at,
        }
    }
}

/// Device Flow 的 UI 引导数据（不含 device_code——它是秘密，留在后端）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFlowSessionDto {
    /// 后端会话 id，`account_device_flow_wait` 凭它取会话。
    pub flow_id: String,
    /// 用户要输入的码（大字展示 + 一键复制）。
    pub user_code: String,
    /// 输码页面。
    pub verification_uri: String,
    /// 携带 user_code 的直链（存在时"打开即填"）。
    pub verification_uri_complete: Option<String>,
    /// 流程过期秒数。
    pub expires_in_secs: u64,
    /// 轮询间隔秒数（后端负责节奏，这里供 UI 显示"大约多久"）。
    pub interval_secs: u64,
}

fn validate_host(host: &str) -> AppResult<String> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "provider host must not be empty",
        ));
    }
    Ok(host)
}

/// PAT 登录：校验令牌并保存账号。能力等级：`Network`。
///
/// 令牌只在此处进入后端：`SecretString` 包装后传给服务层，绝不回显。
#[tauri::command]
pub async fn account_login_with_pat(
    state: State<'_, AppState>,
    host: String,
    token: String,
) -> AppResult<AccountDto> {
    let host = validate_host(&host)?;
    if token.trim().is_empty() {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "token must not be empty",
        ));
    }
    let account = state
        .accounts
        .login_with_pat(&host, SecretString::from(token))
        .await?;
    Ok(AccountDto::from(account))
}

/// 启动 Device Flow。能力等级：`Network`。
#[tauri::command]
pub async fn account_device_flow_start(
    state: State<'_, AppState>,
    host: String,
    scopes: Option<Vec<String>>,
) -> AppResult<DeviceFlowSessionDto> {
    let host = validate_host(&host)?;
    let StartedDeviceFlow { flow_id, start } =
        state.accounts.start_device_flow(&host, scopes).await?;
    Ok(DeviceFlowSessionDto {
        flow_id,
        user_code: start.user_code,
        verification_uri: start.verification_uri,
        verification_uri_complete: start.verification_uri_complete,
        expires_in_secs: start.expires_in_secs,
        interval_secs: start.interval_secs,
    })
}

/// 等待 Device Flow 完成（长任务）。能力等级：`Network`。
///
/// 返回 `jobId`：轮询结果（账号或错误）经 `job:done` / `job:failed` 事件送达；
/// 取消走通用的 `job_cancel`。任务线程里自建 current-thread 运行时执行
/// 异步轮询——`JobRunner` 的任务体是阻塞线程（理由见 `forgedesk-jobs`），
/// 而轮询循环是 async；两边各用所长。
#[tauri::command]
pub fn account_device_flow_wait(
    state: State<'_, AppState>,
    app: AppHandle,
    flow_id: String,
) -> AppResult<crate::repository::JobIdDto> {
    if flow_id.trim().is_empty() {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "flow id must not be empty",
        ));
    }
    let accounts = Arc::clone(&state.accounts);
    let job_id = state.jobs.spawn(reporter_for(app), move |context| {
        context.progress("waiting", None, None, None);
        // 轮询是 async 而任务体是阻塞线程：给这次等待一个专属运行时
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                forgedesk_domain::AppError::new(
                    forgedesk_domain::ErrorCode::Internal,
                    "failed to create async runtime for device flow polling",
                )
                .with_detail(error.to_string())
            })?;
        runtime.block_on(accounts.wait_device_flow(&flow_id, &context.cancellation()))
    });

    Ok(crate::repository::JobIdDto {
        job_id: job_id.as_str().to_owned(),
    })
}

/// 列出已登录账号。能力等级：`ReadOnly`。
#[tauri::command(async)]
pub fn account_list(state: State<'_, AppState>) -> AppResult<Vec<AccountDto>> {
    Ok(state
        .accounts
        .list()?
        .into_iter()
        .map(AccountDto::from)
        .collect())
}

/// 删除账号（凭据库条目 + 账号行）。能力等级：`Mutating`。
#[tauri::command]
pub fn account_remove(state: State<'_, AppState>, account_id: String) -> AppResult<()> {
    if account_id.trim().is_empty() {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "account id must not be empty",
        ));
    }
    state.accounts.remove(account_id.trim())
}

/// `account_device_flow_wait` 完成事件的载荷构造辅助（测试用）。
///
/// 公开原因：`job:done` 的 result 是 `serde_json::Value`，前端契约测试
/// 需要与后端相同的形状构造器，避免两处手拼 JSON 漂移。
pub fn device_flow_done_payload(account: &AccountDto) -> serde_json::Value {
    json!({ "account": account })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{device_flow_done_payload, AccountDto};

    /// `job:done` 载荷形状是前后端契约：字段名 camelCase，无令牌材料。
    #[test]
    fn done_payload_shape_is_camel_case_and_token_free() {
        let dto = AccountDto {
            id: "a1".to_owned(),
            provider: "github".to_owned(),
            host: "github.com".to_owned(),
            login: "octocat".to_owned(),
            avatar_url: None,
            scopes: vec!["repo".to_owned()],
            created_at: Some(123),
        };
        let payload = device_flow_done_payload(&dto);
        let rendered = payload.to_string();

        assert!(rendered.contains("\"account\""));
        assert!(rendered.contains("\"login\":\"octocat\""));
        assert!(rendered.contains("\"createdAt\":123"), "{rendered}");
        assert!(!rendered.to_lowercase().contains("token"));
    }
}
