//! Shell 选择（T5.2）：探测本平台可用的 shell，供终端 "+" 菜单与默认会话使用。
//!
//! # 设计要点
//!
//! - **探测每次现查**（几次 stat 调用）：用户可能在应用开着的时候安装了 Git Bash，
//!   缓存会让"刚装的 shell 不出现"成为无法解释的玄学。
//! - **Windows 上绝不静默落到 `System32\bash.exe`**：那是 WSL 的 bash，
//!   在错误的文件系统里执行 git 命令比没有 Git Bash 更糟。
//! - **编码设置随 shell 一起下发**（PTY-SPIKE §3.5 的结论）：PowerShell 需要显式
//!   UTF-8 输出编码，cmd 需要 `chcp 65001`，否则中文在 ConPTY 管道里乱码。
//!   把 setup 放进启动参数（`-NoExit -Command` / `/K`）而不是启动后注入命令，
//!   用户的历史记录里就不会多出一条莫名其妙的环境命令。

use std::path::{Path, PathBuf};

/// 一个 shell 候选：程序名 + 启动参数。
///
/// `program` 交给系统按 PATH 解析（也可以是探测到的绝对路径，Git Bash 就是）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCommand {
    /// 程序名或绝对路径。
    pub program: String,
    /// 启动参数（不含程序名本身）。
    pub args: Vec<String>,
}

impl ShellCommand {
    /// 构造一个候选（tests 与 spike 需要显式指定 shell）。
    #[must_use]
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
        }
    }
}

/// "+" 菜单里的一个 shell 选项。
///
/// `id` 是稳定标识（前端 i18n 按 id 给展示名，本 crate 不产出文案）；
/// `default` 恒在首位，表示"本平台默认"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOption {
    /// 稳定 id（`default` / `pwsh` / `powershell` / `gitbash` / `cmd` / `bash` / `zsh`）。
    pub id: String,
    /// 程序（绝对路径或 PATH 可解析的名字）。
    pub program: String,
    /// 启动参数。
    pub args: Vec<String>,
}

/// 在 PATH 里探测程序（Windows 自动追加 `.exe`）。
#[must_use]
pub fn probe_in_path(program: &str) -> Option<PathBuf> {
    let with_ext: String = if cfg!(windows) && !program.ends_with(".exe") {
        format!("{program}.exe")
    } else {
        program.to_string()
    };
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(&with_ext);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 定位 Git Bash 的 `bash.exe`（仅 Windows）。
///
/// 推导顺序：PATH 里 git.exe 的安装根（`<root>\cmd\git.exe` → `<root>\bin\bash.exe`）
/// → 常见安装路径。绝不返回 `System32\bash.exe`（WSL，见模块头）。
#[cfg(windows)]
fn locate_git_bash() -> Option<PathBuf> {
    if let Some(git) = probe_in_path("git") {
        // <root>\cmd\git.exe → <root>
        if let Some(root) = git.parent().and_then(Path::parent) {
            for candidate in [
                root.join("bin").join("bash.exe"),
                root.join("usr").join("bin").join("bash.exe"),
            ] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    let roots = [
        std::env::var_os("ProgramFiles").map(|value| PathBuf::from(value).join("Git")),
        std::env::var_os("ProgramFiles(x86)").map(|value| PathBuf::from(value).join("Git")),
        std::env::var_os("LOCALAPPDATA")
            .map(|value| PathBuf::from(value).join("Programs").join("Git")),
    ];
    for root in roots.into_iter().flatten() {
        for candidate in [
            root.join("bin").join("bash.exe"),
            root.join("usr").join("bin").join("bash.exe"),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn locate_git_bash() -> Option<PathBuf> {
    None
}

/// 保证中文/emoji 正确往返的启动参数（PTY-SPIKE §3.5）。
fn utf8_setup_args(id: &str) -> Vec<String> {
    match id {
        "pwsh" | "powershell" => vec![
            "-NoLogo".into(),
            "-NoExit".into(),
            "-Command".into(),
            "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8".into(),
        ],
        "cmd" => vec!["/K".into(), "chcp 65001 >nul".into()],
        _ => Vec::new(),
    }
}

/// 本平台默认 shell（`default` 选项的实现）。
fn default_option() -> ShellOption {
    if cfg!(windows) {
        for id in ["powershell", "cmd"] {
            if let Some(program) = probe_in_path(id) {
                return ShellOption {
                    id: "default".into(),
                    program: program.to_string_lossy().into_owned(),
                    args: utf8_setup_args(id),
                };
            }
        }
        // 探测全空（PATH 异常）时给个能报错的名字：spawn 失败会带 detail。
        ShellOption {
            id: "default".into(),
            program: "cmd".into(),
            args: utf8_setup_args("cmd"),
        }
    } else {
        let program = std::env::var("SHELL")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| probe_in_path("bash").map(|path| path.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "sh".into());
        ShellOption {
            id: "default".into(),
            program,
            args: Vec::new(),
        }
    }
}

/// 本平台可用的 shell 清单（`default` 恒在首位）。
///
/// 探测不可用的选项不出现（前端不展示灰项）；一个都探不到时仍有 `default`
/// （spawn 失败时返回 `NOT_FOUND` 与 detail，比隐藏入口更好诊断）。
#[must_use]
pub fn available_shells() -> Vec<ShellOption> {
    let mut options = vec![default_option()];
    if cfg!(windows) {
        for id in ["pwsh", "cmd"] {
            if let Some(program) = probe_in_path(id) {
                options.push(ShellOption {
                    id: id.into(),
                    program: program.to_string_lossy().into_owned(),
                    args: utf8_setup_args(id),
                });
            }
        }
        // Windows 自带的 Windows PowerShell 5.1 单独列出（组织习惯上的"PowerShell"）。
        if let Some(program) = probe_in_path("powershell") {
            options.push(ShellOption {
                id: "powershell".into(),
                program: program.to_string_lossy().into_owned(),
                args: utf8_setup_args("powershell"),
            });
        }
        if let Some(bash) = locate_git_bash() {
            options.push(ShellOption {
                id: "gitbash".into(),
                program: bash.to_string_lossy().into_owned(),
                args: vec!["-i".into(), "-l".into()],
            });
        }
    } else {
        for id in ["zsh", "bash"] {
            if let Some(program) = probe_in_path(id) {
                options.push(ShellOption {
                    id: id.into(),
                    program: program.to_string_lossy().into_owned(),
                    args: Vec::new(),
                });
            }
        }
    }
    options
}

/// 把用户选择的 shell id（或 `None` = 默认）解析成可执行候选。
///
/// 未知 id 静默回落默认：终端宁可少一个选项也不能开不出来。
#[must_use]
pub fn resolve_shell(shell: Option<&str>) -> ShellCommand {
    let options = available_shells();
    let picked = shell
        .and_then(|id| options.iter().find(|option| option.id == id))
        .unwrap_or(&options[0]);
    ShellCommand {
        program: picked.program.clone(),
        args: picked.args.clone(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// `default` 必须存在且在首位；id 不得重复（前端按 id 查 i18n 文案）。
    #[test]
    fn shell_list_starts_with_default_and_has_unique_ids() {
        let options = available_shells();
        assert_eq!(options[0].id, "default");
        let mut ids: Vec<_> = options.iter().map(|option| option.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), options.len(), "shell ids must be unique");
    }

    /// Windows 默认 shell 必须带 UTF-8 设置参数（PTY-SPIKE §3.5 的结论）。
    #[test]
    fn windows_default_carries_utf8_setup() {
        let default = resolve_shell(None);
        if cfg!(windows) {
            let has_setup = default
                .args
                .iter()
                .any(|arg| arg.contains("OutputEncoding") || arg.contains("chcp"));
            assert!(has_setup, "windows 默认 shell 必须带编码设置参数");
        }
    }

    /// 未知 shell id 回落默认而不是报错（宁可少一个选项也不能开不出终端）。
    #[test]
    fn unknown_shell_id_falls_back_to_default() {
        let fallback = resolve_shell(Some("fish"));
        assert_eq!(fallback, resolve_shell(None));
    }

    /// PATH 探测的基本契约：探测一个必然存在的程序。
    #[test]
    fn probe_finds_a_program_that_must_exist() {
        let must_exist = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(probe_in_path(must_exist).is_some());
        assert!(probe_in_path("forgedesk-no-such-binary").is_none());
    }
}
