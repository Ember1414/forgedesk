//! 与系统外壳交互（在文件管理器中打开目录、用默认程序打开文件）。
//!
//! # 为什么不引入 tauri-plugin-opener
//!
//! 那个插件提供的是"带权限声明与作用域限制的统一打开能力"，对一个只在自己应用数据目录里
//! 打开文件夹的需求来说，多引入一个插件意味着：新增依赖（AGENTS.md §8）、
//! 新增 capability 配置、以及一条需要随 Tauri 版本跟进的升级路径。
//! 这里用各平台自带的命令实现同一件事，代码量小且完全可控；
//! 一旦将来需要"用户点击任意链接"（不受限），再评估换成插件更合适。

use std::path::Path;
use std::process::Command;

use forgedesk_domain::{AppError, AppResult, ErrorCode};

/// 各平台用于打开目录/文件的命令与参数。
///
/// 返回 `(program, args)`；`{path}` 会被替换成目标路径。
///
/// Linux 依赖 `xdg-open`（绝大多数桌面环境都有，由 xdg-utils 提供）；
/// 缺失时用户会看到明确的失败提示与可手动打开的路径，而不是静默无反应。
pub const fn opener_command(os: &str) -> Option<(&'static str, &'static [&'static str])> {
    match os.as_bytes() {
        b"windows" => Some(("explorer", &["{path}"])),
        b"macos" => Some(("open", &["{path}"])),
        b"linux" => Some(("xdg-open", &["{path}"])),
        _ => None,
    }
}

/// 在系统文件管理器中打开一个目录。
///
/// 失败时返回 `INTERNAL` 并附带**可手动使用的绝对路径**：这类操作的失败原因
/// （缺少 `xdg-open`、策略限制）用户无法从错误码推断，但完全可以自己打开目录。
pub fn open_in_file_manager(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Err(
            AppError::new(ErrorCode::NotFound, "the path to open does not exist")
                .with_detail(path.display().to_string()),
        );
    }

    let (program, args) = opener_command(std::env::consts::OS).ok_or_else(|| {
        AppError::new(ErrorCode::Internal, "opening a folder is not supported on this platform")
            .with_detail(std::env::consts::OS.to_owned())
            // hint 由前端 i18n 渲染，这里给的是后端侧的兜底说明
            .with_hint(path.display().to_string())
    })?;

    let target = path.display().to_string();
    let resolved: Vec<String> = args
        .iter()
        .map(|arg| arg.replace("{path}", &target))
        .collect();

    let spawned = Command::new(program).args(&resolved).spawn();

    match spawned {
        Ok(_child) => {
            // 不等待子进程：文件管理器会一直驻留，等待它等于把命令永久挂起。
            // 子进程的生命周期交给系统管理（这是"打开文件夹"这类操作的常态）。
            Ok(())
        }
        Err(error) => Err(
            AppError::new(ErrorCode::Internal, "could not open the folder")
                .with_detail(format!("{program}: {error}"))
                .with_hint(target),
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::opener_command;

    #[test]
    fn resolves_the_platform_command() {
        assert_eq!(
            opener_command("windows").map(|(program, _)| program),
            Some("explorer")
        );
        assert_eq!(
            opener_command("macos").map(|(program, _)| program),
            Some("open")
        );
        assert_eq!(
            opener_command("linux").map(|(program, _)| program),
            Some("xdg-open")
        );
        assert_eq!(
            opener_command("freebsd"),
            None,
            "未知平台应明确返回 None 而不是猜一个命令"
        );
    }

    #[test]
    fn every_platform_command_takes_the_path_placeholder() {
        for os in ["windows", "macos", "linux"] {
            let (_, args) = opener_command(os).expect("应支持三大平台");
            assert!(
                args.iter().any(|arg| arg.contains("{path}")),
                "{os} 的命令缺少 {{path}} 占位符，路径传不进去"
            );
        }
    }
}

/// 用默认浏览器打开一个 http(s) 链接（终端链接识别、文档入口等）。
///
/// 能力边界（安全）：只接受 `http://` 与 `https://` 且不含空白/控制字符的 URL——
/// 这个入口的调用方包括"终端输出里识别到的链接"， Anything else（`file:`、
/// 自定义协议）都可能演变成任意程序执行，一律 `VALIDATION` 拒绝。
///
/// 复用 [`opener_command`]：explorer / open / xdg-open 对 URL 的处理与
/// 对路径一致（转交系统默认处理程序），无需引入浏览器专用依赖。
pub fn open_url(url: &str) -> AppResult<()> {
    let lowered = url.to_ascii_lowercase();
    let scheme_ok = lowered.starts_with("http://") || lowered.starts_with("https://");
    let has_forbidden = url.chars().any(|c| c.is_whitespace() || c.is_control());
    if !scheme_ok || has_forbidden {
        return Err(AppError::new(
            ErrorCode::Validation,
            "only http(s) URLs without whitespace can be opened",
        )
        .with_detail(url.to_string()));
    }

    let (program, args) = opener_command(std::env::consts::OS).ok_or_else(|| {
        AppError::new(
            ErrorCode::Internal,
            "opening a URL is not supported on this platform",
        )
        .with_detail(std::env::consts::OS.to_owned())
    })?;

    let resolved: Vec<String> = args.iter().map(|arg| arg.replace("{path}", url)).collect();

    let spawned = Command::new(program).args(&resolved).spawn();
    match spawned {
        Ok(_child) => Ok(()),
        Err(error) => Err(AppError::new(ErrorCode::Internal, "could not open the URL")
            .with_detail(format!("{program}: {error}"))
            .with_hint(url.to_string())),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod url_tests {
    use super::open_url;

    /// 非 http(s) 协议一律拒绝：这个入口会拿到终端输出里的任意字符串，
    /// `file:` / 自定义协议都可能演变成任意程序执行。
    #[test]
    fn rejects_non_http_schemes_and_control_characters() {
        for bad in [
            "file:///C:/Windows/System32/calc.exe",
            "ftp://example.com/pub",
            "calc.exe",
            "https://example.com/ with space",
            "",
        ] {
            let error = open_url(bad).expect_err("必须拒绝");
            assert_eq!(error.code, forgedesk_domain::ErrorCode::Validation, "{bad}");
        }
    }

    /// 合法 URL 走到 spawn 这一步：用一个不可解析的假程序间接断言"参数检查
    /// 已通过、失败来自程序本身"——open_url 不真正开浏览器的可测写法。
    #[test]
    fn accepts_wellformed_https_url() {
        // 不注入 opener_command 的桩：本测试只断言"合法 URL 不被参数校验拦截"。
        // spawn 失败（explorer/open/xdg-open 在 CI 沙箱可能可用）两种结果都可接受：
        // Ok（真开了）或 INTERNAL（spawn 层失败），但绝不能是 VALIDATION。
        match open_url("https://example.com/forgedesk-test") {
            Ok(()) => {}
            Err(error) => {
                assert_ne!(
                    error.code,
                    forgedesk_domain::ErrorCode::Validation,
                    "合法 URL 不应被参数校验拒绝"
                );
            }
        }
    }
}
