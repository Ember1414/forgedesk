//! 自动更新命令（M7 / T7.1）。
//!
//! | 命令 | 能力等级 | 说明 |
//! | --- | --- | --- |
//! | [`update_check`] | `Network` | 查询是否有新版本（未配置更新源时返回 `configured: false`） |
//! | [`update_install`] | `Network` | 下载并安装指定版本（校验签名后），随后重启 |
//!
//! # 为什么"未配置"不算错误
//!
//! 更新源（endpoints 与公钥）属于**发布配置**：源码自编译与开发构建本来就没有它。
//! 把"没有配置"当成错误，界面上就会出现一条永远修不好的红条；这里改为结构化的
//! `configured: false`，界面据此静默（设置页可以说明"本构建未配置更新源"）。
//!
//! # 安全
//!
//! 更新包必须通过**签名校验**——公钥硬编码在应用内（见 `docs/RELEASE.md` §3.2），
//! 私钥只在 CI。校验失败时插件直接报错，我们如实转达，**绝不**降级为"跳过校验"。
//!
//! # 与前端的分工
//!
//! 命令只做"检查 / 安装"两件事；"什么时候检查（启动 60s + 每 24h）、跳过哪个版本、
//! 用什么渠道"属于界面策略（设置项 + 状态栏横幅），不在这里写死。

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// 更新进度事件名（后端 → 前端）。
pub const EVENT_UPDATE_PROGRESS: &str = "update:progress";

/// 可用更新（IPC 返回形状）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfoDto {
    /// 新版本号。
    pub version: String,
    /// 当前版本号。
    pub current_version: String,
    /// 发布说明（清单里的 `notes`）。
    pub notes: Option<String>,
    /// 发布日期（清单里的 `pub_date`）。
    pub date: Option<String>,
}

/// 更新检查结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckDto {
    /// 本构建是否配置了更新源（endpoints + 公钥）。
    pub configured: bool,
    /// 有可用更新时为详情，否则为 `None`。
    pub update: Option<UpdateInfoDto>,
}

/// 进度事件载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgressPayload {
    /// 阶段：`downloading` / `installing`。
    phase: &'static str,
    /// 已接收字节。
    received: usize,
    /// 总字节（服务端未给 `Content-Length` 时为 `None`）。
    total: Option<u64>,
}

fn updater_error(code: ErrorCode, message: &str, error: &tauri_plugin_updater::Error) -> AppError {
    AppError::new(code, message.to_owned()).with_detail(error.to_string())
}

fn to_dto(update: tauri_plugin_updater::Update) -> UpdateInfoDto {
    UpdateInfoDto {
        version: update.version,
        current_version: update.current_version,
        notes: update.body,
        date: update.date.map(|date| date.to_string()),
    }
}

/// 检查是否有新版本。能力等级：`Network`（发起一次 HTTPS 请求）。
///
/// 未配置更新源时返回 `{ configured: false, update: null }` 而不是报错
/// （原因见模块说明）。网络失败才返回错误——那是用户需要知道的事。
#[tauri::command]
pub async fn update_check(app: AppHandle) -> AppResult<UpdateCheckDto> {
    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(error) => {
            // 未配置 endpoints / 公钥：开发构建与源码自编译产物的常态
            tracing::debug!(%error, "更新源未配置，跳过更新检查");
            return Ok(UpdateCheckDto {
                configured: false,
                update: None,
            });
        }
    };

    let update = updater
        .check()
        .await
        .map_err(|error| updater_error(ErrorCode::Network, "the update check failed", &error))?;

    Ok(UpdateCheckDto {
        configured: true,
        update: update.map(to_dto),
    })
}

/// 下载并安装指定版本。能力等级：`Network`（下载）→ 安装后重启进程。
///
/// `version` 必须与**本次**检查到的版本一致：界面上的信息可能已经过期（期间又发了新版），
/// 不一致时要求界面重新确认，避免"用户点了 A、装上的却是 B"。
///
/// 进度经 [`EVENT_UPDATE_PROGRESS`] 推送；安装成功后由插件重启应用（默认行为）。
#[tauri::command]
pub async fn update_install(app: AppHandle, version: String) -> AppResult<()> {
    let updater = app.updater().map_err(|error| {
        updater_error(
            ErrorCode::Validation,
            "the updater is not configured for this build",
            &error,
        )
    })?;

    let update = updater
        .check()
        .await
        .map_err(|error| updater_error(ErrorCode::Network, "the update check failed", &error))?
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "no update is available"))?;

    if update.version != version {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the available update changed; check again",
        )
        .with_detail(format!("requested {version}, available {}", update.version)));
    }

    let download_emitter = app.clone();
    let install_emitter = app.clone();
    update
        .download_and_install(
            move |received, total| {
                let _ = download_emitter.emit(
                    EVENT_UPDATE_PROGRESS,
                    UpdateProgressPayload {
                        phase: "downloading",
                        received,
                        total,
                    },
                );
            },
            move || {
                let _ = install_emitter.emit(
                    EVENT_UPDATE_PROGRESS,
                    UpdateProgressPayload {
                        phase: "installing",
                        received: 0,
                        total: None,
                    },
                );
            },
        )
        .await
        .map_err(|error| {
            updater_error(
                ErrorCode::Network,
                "the update could not be downloaded or installed",
                &error,
            )
        })?;

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    /// 未配置时不该构造出任何"看起来有更新"的结果（钉住契约形状）。
    #[test]
    fn an_unconfigured_check_reports_no_update() {
        let report = super::UpdateCheckDto {
            configured: false,
            update: None,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["configured"], serde_json::Value::Bool(false));
        assert!(json["update"].is_null());
    }

    /// IPC 形状是 camelCase，前端 DTO 与之逐字段对应。
    #[test]
    fn update_info_serializes_to_camel_case() {
        let info = super::UpdateInfoDto {
            version: "1.0.0".to_owned(),
            current_version: "0.7.0".to_owned(),
            notes: Some("notes".to_owned()),
            date: None,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["currentVersion"], "0.7.0");
        assert!(json.get("current_version").is_none());
    }
}
