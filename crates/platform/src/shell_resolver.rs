//! Shell 解析（T6.9）：列出可用 shell、给出默认建议。
//!
//! # 为什么自己探测而不是调用 `where`/`which`
//!
//! 子进程探测慢且输出随语言环境变化（AGENTS §7 对机器可解析输出的要求
//! 会把 `where` 的本地化输出变成雷区）。shell 无非是 PATH 上的几个固定名字
//! 加上 Git Bash 的固定安装布局，文件系统探测是纯逻辑：可注入、可测试、零进程。
//!
//! # WSL bash 的排除
//!
//! Windows 上 `C:\Windows\System32\bash.exe` 存在时它多半是 WSL——用户期望的
//! "Git Bash" 是 Git for Windows 自带的。区分标记：Git 安装根目录必然有
//! `cmd\git.exe`（安装器固定生成），从 bash.exe 所在目录向上两层验证。

use std::path::{Path, PathBuf};

use forgedesk_domain::{AppError, AppResult, ErrorCode};

/// 支持识别的 shell 种类（T6.9 清单：pwsh/powershell/cmd/Git Bash/bash/zsh）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    /// PowerShell 7+（跨平台）。
    PowerShellCore,
    /// Windows 自带的 Windows PowerShell 5.x。
    WindowsPowerShell,
    /// cmd.exe。
    Cmd,
    /// Git for Windows 自带的 bash。
    GitBash,
    /// zsh（macOS 默认）。
    Zsh,
    /// bash（Linux）。
    Bash,
}

impl ShellKind {
    /// 展示名（稳定标识，前端按此 i18n 或原样展示）。
    pub fn display_name(self) -> &'static str {
        match self {
            ShellKind::PowerShellCore => "PowerShell",
            ShellKind::WindowsPowerShell => "Windows PowerShell",
            ShellKind::Cmd => "Command Prompt",
            ShellKind::GitBash => "Git Bash",
            ShellKind::Zsh => "zsh",
            ShellKind::Bash => "bash",
        }
    }
}

/// 一个可用的 shell。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellInfo {
    /// 可执行文件绝对路径。
    pub path: PathBuf,
    /// 种类。
    pub kind: ShellKind,
}

/// 默认 shell 的偏好顺序（spec 无从得知"用户默认"时按此取第一个可用项）。
const PREFERENCE: [ShellKind; 6] = [
    ShellKind::PowerShellCore,
    ShellKind::WindowsPowerShell,
    ShellKind::GitBash,
    ShellKind::Zsh,
    ShellKind::Bash,
    ShellKind::Cmd,
];

/// 探测可用 shell。`search_dirs` 依次为 PATH 各目录（由调用方解析，
/// 测试可注入临时目录）；`is_windows` 决定探测哪些可执行名与 Git Bash 标记逻辑。
///
/// 返回去重后的列表（同一可执行只出现一次，先到先得——PATH 顺序即优先级）。
pub fn probe_shells(search_dirs: &[PathBuf], is_windows: bool) -> Vec<ShellInfo> {
    let mut found: Vec<ShellInfo> = Vec::new();
    let mut push = |path: PathBuf, kind: ShellKind| {
        if !found.iter().any(|shell| shell.path == path) {
            found.push(ShellInfo { path, kind });
        }
    };

    for dir in search_dirs {
        if is_windows {
            for (file, kind) in [
                ("pwsh.exe", ShellKind::PowerShellCore),
                ("powershell.exe", ShellKind::WindowsPowerShell),
                ("cmd.exe", ShellKind::Cmd),
            ] {
                let candidate = dir.join(file);
                if candidate.is_file() {
                    push(candidate, kind);
                }
            }
            let bash = dir.join("bash.exe");
            if bash.is_file() {
                if let Some(kind) = classify_windows_bash(dir) {
                    push(bash, kind);
                }
            }
        } else {
            for (file, kind) in [
                ("pwsh", ShellKind::PowerShellCore),
                ("zsh", ShellKind::Zsh),
                ("bash", ShellKind::Bash),
            ] {
                let candidate = dir.join(file);
                if candidate.is_file() {
                    push(candidate, kind);
                }
            }
        }
    }
    found
}

/// 判断 Windows 上某个 `bash.exe` 所在目录是否属于 Git for Windows。
/// 不是（例如 WSL 的 System32\bash.exe）则返回 None。
fn classify_windows_bash(dir: &Path) -> Option<ShellKind> {
    // Git for Windows 的两种布局：<Git>\bin\bash.exe 与 <Git>\usr\bin\bash.exe。
    // 安装根的特征是 <Git>\cmd\git.exe（安装器固定生成），从 bash 目录逐层上探。
    let mut candidate = dir.parent();
    for _ in 0..2 {
        match candidate {
            Some(root) => {
                if root.join("cmd").join("git.exe").is_file() {
                    return Some(ShellKind::GitBash);
                }
                candidate = root.parent();
            }
            None => break,
        }
    }
    None
}

/// 从候选中挑默认 shell（纯函数）。
///
/// `shell_env`（unix 的 `$SHELL`）显式指定时优先匹配（按文件名，忽略大小写）；
/// 否则按 [`PREFERENCE`] 取第一个可用项。
pub fn pick_default(shells: &[ShellInfo], shell_env: Option<&str>) -> Option<ShellInfo> {
    if let Some(wanted) = shell_env {
        let wanted = wanted.replace('\\', "/");
        let name = wanted.rsplit('/').next().unwrap_or_default().to_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name).to_owned();
        if let Some(hit) = shells.iter().find(|shell| {
            shell
                .path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().to_lowercase() == name)
        }) {
            return Some(hit.clone());
        }
    }
    for kind in PREFERENCE {
        if let Some(hit) = shells.iter().find(|shell| shell.kind == kind) {
            return Some(hit.clone());
        }
    }
    None
}

/// 当前 PATH 的搜索目录（分隔符按平台）。
pub fn path_search_dirs() -> Vec<PathBuf> {
    let raw = std::env::var("PATH").unwrap_or_default();
    std::env::split_paths(&raw)
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect()
}

/// ShellResolver trait：真实实现探测文件系统；测试注入固定列表。
pub trait ShellResolver: Send + Sync {
    /// 当前可用的 shell 列表。
    fn available(&self) -> AppResult<Vec<ShellInfo>>;
    /// 建议的默认 shell。
    fn default_shell(&self) -> AppResult<ShellInfo>;
}

/// 真实实现：探测 PATH + （unix）读 `$SHELL`。
#[derive(Debug, Default)]
pub struct SystemShellResolver;

impl ShellResolver for SystemShellResolver {
    fn available(&self) -> AppResult<Vec<ShellInfo>> {
        let shells = probe_shells(&path_search_dirs(), cfg!(windows));
        if shells.is_empty() {
            // 一个 shell 都没有时终端页无内容可列；给出明确错误而不是空 UI
            return Err(
                AppError::new(ErrorCode::NotFound, "no known shell found on PATH")
                    .with_detail(std::env::var("PATH").unwrap_or_default()),
            );
        }
        Ok(shells)
    }

    fn default_shell(&self) -> AppResult<ShellInfo> {
        let shells = self.available()?;
        let shell_env = if cfg!(windows) {
            None
        } else {
            std::env::var("SHELL").ok()
        };
        pick_default(&shells, shell_env.as_deref()).ok_or_else(|| {
            AppError::new(
                ErrorCode::Internal,
                "no shell candidate matches the default preference",
            )
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("forgedesk-shell-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn windows_probing_finds_the_standard_shells() {
        let root = temp_root("win");
        let system32 = root.join("System32");
        let pwsh_dir = root.join("PowerShell");
        fs::create_dir_all(&system32).unwrap();
        fs::create_dir_all(&pwsh_dir).unwrap();
        fs::write(system32.join("cmd.exe"), b"").unwrap();
        fs::write(system32.join("powershell.exe"), b"").unwrap();
        fs::write(pwsh_dir.join("pwsh.exe"), b"").unwrap();

        let shells = probe_shells(&[pwsh_dir, system32], true);
        let kinds: Vec<_> = shells.iter().map(|shell| shell.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ShellKind::PowerShellCore,
                ShellKind::WindowsPowerShell,
                ShellKind::Cmd
            ]
        );
    }

    #[test]
    fn git_bash_is_recognized_by_the_git_install_layout() {
        let root = temp_root("gitbash");
        let git = root.join("Git");
        for sub in ["Git/bin", "Git/cmd"] {
            fs::create_dir_all(root.join(sub)).unwrap();
        }
        fs::write(git.join("cmd").join("git.exe"), b"").unwrap();
        fs::write(git.join("bin").join("bash.exe"), b"").unwrap();

        let shells = probe_shells(&[git.join("bin")], true);
        assert_eq!(shells.len(), 1);
        assert_eq!(shells[0].kind, ShellKind::GitBash);
    }

    #[test]
    fn wsl_bash_is_excluded_because_it_has_no_git_install_marker() {
        let root = temp_root("wsl");
        let system32 = root.join("System32");
        fs::create_dir_all(&system32).unwrap();
        fs::write(system32.join("bash.exe"), b"").unwrap();

        let shells = probe_shells(&[system32], true);
        assert!(
            shells.is_empty(),
            "System32 下的 bash.exe 是 WSL，绝不能当成 Git Bash"
        );
    }

    #[test]
    fn unix_probing_finds_zsh_and_bash_without_windows_markers() {
        let root = temp_root("unix");
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("zsh"), b"").unwrap();
        fs::write(bin.join("bash"), b"").unwrap();

        let shells = probe_shells(&[bin], false);
        let kinds: Vec<_> = shells.iter().map(|shell| shell.kind).collect();
        assert_eq!(kinds, vec![ShellKind::Zsh, ShellKind::Bash]);
    }

    #[test]
    fn default_pick_prefers_the_explicit_env_then_the_preference_order() {
        let pwsh = ShellInfo {
            path: PathBuf::from(r"C:\Tools\pwsh.exe"),
            kind: ShellKind::PowerShellCore,
        };
        let cmd = ShellInfo {
            path: PathBuf::from(r"C:\Windows\System32\cmd.exe"),
            kind: ShellKind::Cmd,
        };
        let bash = ShellInfo {
            path: PathBuf::from("/bin/bash"),
            kind: ShellKind::Bash,
        };

        // $SHELL 显式指定优先
        let picked = pick_default(&[pwsh.clone(), bash.clone()], Some("/bin/bash")).unwrap();
        assert_eq!(picked.kind, ShellKind::Bash);

        // Windows 上的 .exe 大小写与后缀不影响匹配
        let picked =
            pick_default(&[pwsh.clone(), cmd.clone()], Some(r"C:\Tools\PWSH.EXE")).unwrap();
        assert_eq!(picked.kind, ShellKind::PowerShellCore);

        // 未指定时按偏好顺序：PowerShell 在 bash 之前
        let picked = pick_default(&[bash.clone(), pwsh.clone()], None).unwrap();
        assert_eq!(picked.kind, ShellKind::PowerShellCore);
    }

    #[test]
    fn duplicate_entries_across_path_dirs_are_deduplicated() {
        let root = temp_root("dup");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("zsh"), b"").unwrap();

        let shells = probe_shells(&[root.clone(), root], false);
        assert_eq!(shells.len(), 1, "同一可执行不应出现两次");
    }
}
