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
