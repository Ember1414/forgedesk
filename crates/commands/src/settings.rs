//! 设置相关命令（`settings_*`）。
//!
//! # 为什么命令这么薄
//!
//! 参数解析与数据存取都在 `forgedesk-storage`，这里只做三件事：
//! ① 把 IPC 参数收敛到已知取值（外部输入一律不可信）；
//! ② 校验值的形状（必须是合法 JSON 字符串）；
//! ③ 把结果转成前端 DTO。
//!
//! 命令层带 `State`，很难在没有 Tauri 运行时的情况下单测，
//! 因此凡是能被纯函数表达的逻辑都下沉（见 `forgedesk_storage::Scope::parse`）。

use std::collections::BTreeMap;

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_storage::{Scope, SettingsRepository};
use tauri::State;

use crate::state::AppState;
use crate::watch::{self, WatchSettings};

/// 校验设置值是否为合法 JSON。
///
/// 契约是"值一律为 JSON 字符串"（见 `docs/API.md`）。为什么在命令层校验：
/// 前端可能因为 bug 传进来一段裸文本，如果放行，将来读取时 `JSON.parse` 会抛错，
/// 而那时已经无从知道是谁写坏的。在入口拦住，错误信息能直接指向调用点。
pub fn validate_json_value(value: &str) -> AppResult<()> {
    serde_json::from_str::<serde_json::Value>(value)
        .map(|_| ())
        .map_err(|error| {
            AppError::new(ErrorCode::Validation, "settings value must be valid JSON")
                .with_detail(error.to_string())
        })
}

/// 读取一个设置项；不存在时返回 `null`（而不是报错）。
///
/// 能力等级：`ReadOnly`。
///
/// 参数 `scope` 取 `"global"` 或 `"repo"`；`repo` 时必须提供 `repoId`。
#[tauri::command]
pub fn settings_get(
    state: State<'_, AppState>,
    scope: String,
    repo_id: Option<i64>,
    key: String,
) -> AppResult<Option<String>> {
    let scope = Scope::parse(&scope, repo_id)?;
    SettingsRepository::new(&state.database).get(&scope, &key)
}

/// 写入（或覆盖）一个设置项。
///
/// 能力等级：`Mutating`（写本地配置；不涉及仓库状态，因此不需要快照）。
///
/// `value` 必须是合法 JSON 字符串——调用方负责序列化，本命令只校验形状。
#[tauri::command]
pub fn settings_set(
    state: State<'_, AppState>,
    scope: String,
    repo_id: Option<i64>,
    key: String,
    value: String,
) -> AppResult<()> {
    let scope = Scope::parse(&scope, repo_id)?;
    validate_json_value(&value)?;
    SettingsRepository::new(&state.database).set(&scope, &key, &value)?;

    // 监听类设置写完之后必须**立刻生效**：用户改了去抖动时长却要重启应用
    // 才起作用，等同于这个设置不存在。
    if WatchSettings::is_watch_key(&key) {
        watch::apply_settings(&state.watchers, &state.database, &state.open_repos.ids())?;
    }
    Ok(())
}

/// 读取某个范围下的全部设置（返回 `key → JSON 字符串`）。
///
/// 能力等级：`ReadOnly`。
///
/// 用途：应用启动时一次性拉取，避免逐个 key 往返（启动阶段每次 IPC 都是可见延迟）。
#[tauri::command]
pub fn settings_all(
    state: State<'_, AppState>,
    scope: String,
    repo_id: Option<i64>,
) -> AppResult<BTreeMap<String, String>> {
    let scope = Scope::parse(&scope, repo_id)?;
    SettingsRepository::new(&state.database).all(&scope)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::ErrorCode;

    use super::validate_json_value;

    #[test]
    fn accepts_valid_json_values() {
        for value in [
            "\"compact\"",
            "42",
            "true",
            "null",
            "{\"mode\":\"compact\",\"width\":280}",
            "[1,2,3]",
        ] {
            validate_json_value(value).unwrap_or_else(|error| panic!("{value} 应通过：{error:?}"));
        }
    }

    #[test]
    fn rejects_plain_text_with_validation_error() {
        let error = validate_json_value("compact").unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert!(
            error.detail.is_some(),
            "应带上解析失败的原因，便于定位是哪个调用点写坏的"
        );
    }

    #[test]
    fn rejects_empty_and_partial_json() {
        assert!(validate_json_value("").is_err());
        assert!(validate_json_value("{").is_err());
    }
}
