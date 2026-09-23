//! 系统与运行时相关命令。
//!
//! 为什么命令定义在**子模块**而不是 crate 根 `lib.rs`：
//! `#[tauri::command]` 会为每个命令生成一个 `#[macro_export]` 的宏
//! （形如 `__tauri_command_name_<fn>`），它落在 **crate 根**的宏命名空间；
//! 若命令函数本身也定义在 crate 根，宏生成的 `pub use __tauri_command_name_<fn>;`
//! 会与前者同名，触发 E0255（"must be defined only once in the macro namespace"）。
//! 因此本项目约定：**所有命令一律定义在子模块中，由 `lib.rs` 重导出**。

use forgedesk_domain::AppResult;
use serde::{Deserialize, Serialize};

/// 应用版本与构建信息。
///
/// 序列化为 camelCase，与前端 TypeScript DTO 一一对应
/// （见 `src/lib/ipc/index.ts` 的 `AppVersion`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppVersion {
    /// 语义化版本号。
    pub version: String,
    /// 构建时的 git 提交短哈希；不在 git 仓库中构建时为 `"unknown"`。
    pub git_sha: String,
    /// 目标平台标识，形如 `x86_64-windows`。
    pub target: String,
    /// 构建类型：`"debug"` 或 `"release"`。
    pub profile: String,
}

/// 返回应用版本与构建信息。
///
/// 能力等级：`ReadOnly`（无参数、无副作用）。
///
/// 用途：验证前后端 IPC 通路是否打通；同时让用户在提 Issue 时能一键提供
/// 版本 + 提交号 + 平台信息。
#[tauri::command]
pub fn app_version() -> AppResult<AppVersion> {
    Ok(AppVersion {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        git_sha: option_env!("FORGEDESK_GIT_SHA")
            .unwrap_or("unknown")
            .to_owned(),
        target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        profile: if cfg!(debug_assertions) {
            "debug".to_owned()
        } else {
            "release".to_owned()
        },
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::app_version;

    #[test]
    fn app_version_reports_expected_fields() {
        let info = app_version().expect("app_version 不应失败");

        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(!info.target.is_empty(), "target 不应为空");
        assert!(
            info.target.contains('-'),
            "target 应为 <arch>-<os> 形式，实际：{}",
            info.target
        );
        assert!(
            info.profile == "debug" || info.profile == "release",
            "profile 只能是 debug 或 release，实际：{}",
            info.profile
        );
    }

    #[test]
    fn app_version_serializes_to_camel_case() {
        let info = app_version().expect("app_version 不应失败");
        let json = serde_json::to_string(&info).unwrap();

        assert!(json.contains("\"gitSha\""), "应输出 camelCase 字段：{json}");
        assert!(
            !json.contains("git_sha"),
            "不应出现 snake_case 字段：{json}"
        );
    }
}
