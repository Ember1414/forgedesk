//! 演示 / 调试命令。
//!
//! 存在的理由：错误链路（后端分类 → 脱敏 → IPC → 前端 i18n → Toast → 动作按钮）
//! 是**基础设施**，它坏掉时不会有任何业务功能报错，只会在真正出错的那天集体失效。
//! 因此需要一个可以随时触发受控失败的入口，让端到端验证与将来的 E2E 测试有抓手。
//!
//! 稳定性约定：本模块的命令**只在开发构建中注册**（见 `src-tauri` 的注册处），
//! 不允许被业务代码依赖。

use forgedesk_diagnostics::sanitize_log;
use forgedesk_domain::{AppError, AppResult, ErrorCode, FixAction};

/// 演示用的假凭据：故意在 detail 里放一段真实形状的令牌，
/// 用来验证"详情经过脱敏"这条链路确实生效（在界面上应显示为 `ghp_«redacted»`）。
const DEMO_LEAKY_DETAIL: &str =
    "remote: https://alice:ghp_DEMO0000000000000000000000000000@example.com \
rejected the push\nhint: run 'git fetch' then retry";

/// 触发一个受控失败的演示错误。
///
/// 能力等级：`ReadOnly`（不读写仓库、不访问网络；仅构造并返回一个错误）。
///
/// 参数 `code` 为错误码字符串（见 [`ErrorCode::as_str`]），大小写不敏感；
/// 传入未知值时返回 `VALIDATION` 错误——**外部输入必须收敛到已知枚举**，
/// 不能把用户传进来的字符串原样当作错误码回显。
///
/// 返回的错误自带：
/// - `detail`：含假凭据的原始输出（用于验证脱敏）；
/// - `actions`：一个可点击的修复动作（调用 `app_version`，验证动作按钮的链路）。
#[tauri::command]
pub fn debug_throw_error(code: String) -> AppResult<()> {
    let Some(error_code) = ErrorCode::parse(&code) else {
        // 外部输入只进 detail（只读、折叠展示、经脱敏），不进 message。
        // 原因：message 会被前端当作"标题级"文本处理（可能进日志、通知、崩溃报告），
        // 把用户输入拼进去等于给日志注入开了口子；而 detail 的定位本就是"原始上下文"。
        return Err(AppError::new(ErrorCode::Validation, "unknown error code")
            .with_detail(sanitize_log(&code)));
    };

    Err(AppError::from_code(error_code)
        .with_detail(DEMO_LEAKY_DETAIL)
        .with_action(FixAction::new(
            "debug.refresh",
            "errors.actions.refresh",
            "app_version",
        )))
}

/// 触发一次真实的 panic（用于 T0.8 的手工验收与将来的崩溃恢复测试）。
///
/// 能力等级：`ReadOnly`（不改任何数据；它只是让一个后台线程崩掉）。
///
/// **为什么在独立线程里 panic**：同步命令默认在 Tauri 的主线程上执行，
/// 在主线程 panic 会连带界面一起消失，无法验证"仅该操作失败"。
/// 在独立线程里 panic 更接近真实场景（耗时操作都在后台线程），
/// 也让我们能确认 panic hook 与主流程的隔离确实生效。
///
/// release 构建使用 `panic = "abort"`（见根 Cargo.toml），因此这个命令只在开发构建注册；
/// 崩溃留在 release 下的表现由 M7/T7.5 的崩溃恢复处理——那正是会话标记与 panic 文件的用途。
#[tauri::command]
// 这里的 panic 是**功能本身**，不是疏忽：整个命令的存在意义就是制造一次真实崩溃，
// 以便验证 panic 留档、会话标记与"界面不受影响"。因此显式豁免该 lint，
// 并把它限制在这一个函数里（workspace 其余代码仍然禁止 panic）。
#[allow(clippy::panic)]
pub fn debug_panic() -> AppResult<()> {
    std::thread::spawn(|| {
        panic!("ForgeDesk debug panic: 人为触发的崩溃，用于验证日志与崩溃恢复链路（T0.8）");
    });

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::ErrorCode;

    use super::debug_throw_error;

    #[test]
    fn returns_the_requested_error_code() {
        let error = debug_throw_error("GIT_CONFLICT".to_owned()).expect_err("演示命令必须失败");

        assert_eq!(error.code, ErrorCode::GitConflict);
        assert!(!error.message.is_empty(), "必须带兜底描述");
        assert!(error.actions.len() == 1, "应带一个可点击的修复动作");
        assert_eq!(error.actions[0].command, "app_version");
    }

    #[test]
    fn accepts_lower_case_and_whitespace() {
        let error = debug_throw_error(" network ".to_owned()).expect_err("演示命令必须失败");
        assert_eq!(error.code, ErrorCode::Network);
        assert!(error.retryable, "网络错误应可重试");
    }

    #[test]
    fn rejects_unknown_codes_with_validation() {
        let error = debug_throw_error("<script>alert(1)</script>".to_owned())
            .expect_err("演示命令必须失败");

        assert_eq!(error.code, ErrorCode::Validation);
        assert!(
            !error.message.contains("<script>"),
            "外部输入不应进入 message（标题级文本，会进日志与通知）"
        );
        assert_eq!(
            error.detail.as_deref(),
            Some("<script>alert(1)</script>"),
            "原始输入应保留在 detail 里以便定位参数问题"
        );
    }
}
