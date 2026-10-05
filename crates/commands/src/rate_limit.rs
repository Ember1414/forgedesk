//! GitHub 限流状态命令（T4.10）：快照读取与主动刷新。
//!
//! # 数据从哪来
//!
//! [`forgedesk_provider::RateLimitTracker`] 在 HTTP 底座的**每一个**响应上
//! 捕获 `x-ratelimit-*` 头（错误响应也带），快照随 `GitHubHttp` 在
//! provider 工厂克隆间共享——命令只读快照，零网络成本，页面加载时可以
//! 随意轮询。主动刷新走 `GET /rate_limit`（不耗配额、值权威，
//! [`GitHubProvider::refresh_rate_limit`]）。
//!
//! 降级语义（M4 验收"触发限流时 UI 显示剩余额度与重置时间，并自动降级
//! 为缓存数据"）分两半：缓存回退发生在 HTTP 底座（`etag_cache` 模块，
//! 对服务层透明）；本模块的快照负责"显示"——`remaining == 0` 即横幅
//! 出现的判定条件。

use tauri::State;

use forgedesk_domain::AppResult;
use forgedesk_provider::RateLimitState;

use crate::state::AppState;

/// 最近一次限流快照。能力等级：`ReadOnly`。
///
/// 从未见过带限流头的响应（本次会话还没发过请求）为 `None`。
#[tauri::command]
pub fn repo_rate_limit_state(state: State<'_, AppState>) -> AppResult<Option<RateLimitState>> {
    Ok(state.host_repos.rate_limit_snapshot())
}

/// 主动刷新限流额度（`GET /rate_limit`）。能力等级：`Network`。
#[tauri::command]
pub async fn repo_rate_limit_refresh(
    state: State<'_, AppState>,
    host: String,
    repo_id: Option<i64>,
) -> AppResult<RateLimitState> {
    if host.trim().is_empty() {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "provider host must not be empty",
        ));
    }
    state
        .host_repos
        .refresh_rate_limit(host.trim().to_ascii_lowercase().as_str(), repo_id)
        .await
}
