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

    let update = updater.check().await.map_err(|error| {
        // 检查失败此前完全静默：后端不记日志、前端把错误态画成"没有提示"——
        // 用户只会觉得"没有更新提示"，而原因（网络/清单/签名）无从知晓
        tracing::warn!(error = %error, "更新检查失败");
        updater_error(ErrorCode::Network, "the update check failed", &error)
    })?;

    match &update {
        Some(next) => tracing::info!(version = %next.version, "发现新版本"),
        None => tracing::debug!("当前已是最新版本"),
    }

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

    /// # 篡改包被拒绝（PLAN M7 验收标准第 2 条）
    ///
    /// 更新包在安装前必须通过 minisign 验签。`tauri-plugin-updater` 的内部实现是
    /// **先 base64 解码、再 `PublicKey::decode` / `Signature::decode`、最后
    /// `verify(data, &signature, true)`**——这条测试用同一个库、同一套调用，
    /// 把"篡改即拒绝"钉成自动化断言（此前该验收项一直标注"未执行"，因为它
    /// 看起来需要真实发布密钥；实际上**一次性夹具密钥**就能覆盖同一段验证逻辑）。
    ///
    /// 夹具材料全部是常量：公钥、签名、载荷内联在下面，私钥从未进仓库。
    /// 因此本测试离线、确定、不触碰任何真实发布密钥。
    #[test]
    fn a_tampered_update_payload_is_refused_by_signature_verification() {
        use base64::Engine as _;
        use minisign_verify::{PublicKey, Signature};

        /// 夹具公钥（tauri 的 pubkey 格式：minisign 文本再做一层 base64）。
        const PUBKEY_BASE64: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IERGOUM5MjRFMTFEMUUzQjMKUldTejQ5RVJUcEtjMzIxUWpIc2lyNmJweUliNXlhRUx1Zms0WW52MHRZVFdwZTNpRmNtdXkzMWkK";
        /// 另一对密钥的公钥：拿错钥匙必须验不过（防止"随便什么钥匙都放行"）。
        const OTHER_PUBKEY_BASE64: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE3MjlGRkEyOTUyMDk3MjYKUldRbWx5Q1ZvdjhwRjBpVHFpN3ZCQ0tMajVNRTVZZU56dnp6S2NoVTkyZFpnSnhGYTFpdFpkZDAK";
        /// 对 `PAYLOAD` 的签名（同样两层 base64）。
        const SIGNATURE_BASE64: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTejQ5RVJUcEtjMzRNeVFlaEdCalVRMk5xZHFBYk1ld1h1ZzRlNzZ1bnBYNDJ6M2VyTDVlQ2FTUWlQNlpDdWswL0EvWlNWV0RoZnRlc1pSdVBmWWFTcVE4RGppRE5QNWdRPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkxNDMwNTU5CWZpbGU6cGF5bG9hZC50eHQKa1owcDZRcVJ2Z1ZmVVYzOVQwM0R2VVNKa3c2dkRWMG5rcllmSG5lZU9XbkRITTVsWU1lOFRrMGpmblRNTXoxQVg4Ny8ydFBhamVjVGcvSVdROE4rQ3c9PQo=";
        /// 被签名的载荷（36 字节，无换行）。
        const PAYLOAD: &[u8] = b"ForgeDesk updater fixture payload v1";

        fn decode_base64(value: &str) -> String {
            String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .unwrap(),
            )
            .unwrap()
        }

        let public_key = PublicKey::decode(&decode_base64(PUBKEY_BASE64)).unwrap();
        let signature = Signature::decode(&decode_base64(SIGNATURE_BASE64)).unwrap();

        // 正面：原样必须通过——否则下面几条"拒绝"什么也证明不了
        public_key
            .verify(PAYLOAD, &signature, true)
            .expect("原样载荷必须验签通过");

        // 篡改一个字节：拒绝（中间人换包的最小形态）
        let mut tampered = PAYLOAD.to_vec();
        tampered[0] ^= 0x01;
        assert!(
            public_key.verify(&tampered, &signature, true).is_err(),
            "改动一个字节必须拒绝"
        );

        // 截短：拒绝（换包者也可能只改长度）
        assert!(
            public_key
                .verify(&PAYLOAD[..PAYLOAD.len() - 1], &signature, true)
                .is_err(),
            "长度变化必须拒绝"
        );

        // 拿错公钥：拒绝（key id 不匹配，连"签名看起来对"的机会都不给）
        let other_key = PublicKey::decode(&decode_base64(OTHER_PUBKEY_BASE64)).unwrap();
        assert!(
            other_key.verify(PAYLOAD, &signature, true).is_err(),
            "非签发者的公钥必须拒绝"
        );

        // 签名里记录着被签的文件名（`requireSignedVersion` 读的 trusted comment
        // 就在同一段里）——这是"签名绑定了哪个产物"的可核对凭据
        assert!(
            signature.trusted_comment().contains("file:payload.txt"),
            "trusted comment 必须记录被签文件名，实际：{}",
            signature.trusted_comment()
        );
    }
}
