//! 系统集成（T6.9）：在文件管理器中定位文件、用默认应用打开文件、开机自启。
//!
//! # 与 [`crate::shell`] 的分工
//!
//! `shell::open_in_file_manager` 是"打开一个目录"（浏览）；本模块是
//! "定位某个文件"（reveal，在管理器中选中它）与"用默认应用打开某个文件"。
//! 三者共同点：一律参数数组调用，不解析输出、不等待子进程退出。
//!
//! # 开机自启的用户可见影响（T6.9 要求明确列出）
//!
//! - Windows：在 `HKCU\...\Run` 写入一个 `ForgeDesk` 值（仅当前用户，不弹 UAC）；
//! - Linux：写 `~/.config/autostart/forgedesk.desktop`（XDG 规范）；
//! - macOS：写 `~/Library/LaunchAgents/com.forgedesk.ForgeDesk.plist`。
//!
//! 三种都只影响当前用户，卸载/关闭开关即完全撤销，不写系统目录。

use std::path::{Path, PathBuf};
use std::process::Command;

use forgedesk_domain::{AppError, AppResult, ErrorCode};

/// 各平台"在文件管理器中定位"的命令与参数（`{path}` 占位）。
/// Linux 没有统一的 select 协议，退而打开所在目录。
pub const fn reveal_command(os: &str) -> Option<(&'static str, &'static [&'static str])> {
    match os.as_bytes() {
        b"windows" => Some(("explorer", &["/select,{path}"])),
        b"macos" => Some(("open", &["-R", "{path}"])),
        b"linux" => Some(("xdg-open", &["{parent}"])),
        _ => None,
    }
}

/// 各平台"用默认应用打开"的命令与参数。
///
/// Windows 走 `cmd /c start "" <path>`：explorer 直开文件的行为不可靠，
/// `start` 才走 ShellExecute 关联。仍然全是参数数组（§7 禁止的是拼 shell 字符串）；
/// `""` 是 start 的窗口标题占位——路径含空格时缺了它会被当成标题。
pub const fn default_app_command(os: &str) -> Option<(&'static str, &'static [&'static str])> {
    match os.as_bytes() {
        b"windows" => Some(("cmd", &["/c", "start", "", "{path}"])),
        b"macos" => Some(("open", &["{path}"])),
        b"linux" => Some(("xdg-open", &["{path}"])),
        _ => None,
    }
}

fn spawn_opener(
    template: (&'static str, &'static [&'static str]),
    replacements: &[(&str, String)],
    fallback_hint: String,
) -> AppResult<()> {
    let (program, args) = template;
    let resolved: Vec<String> = args
        .iter()
        .map(|arg| {
            let mut text = arg.to_string();
            for (key, value) in replacements {
                text = text.replace(key, value);
            }
            text
        })
        .collect();

    match Command::new(program).args(&resolved).spawn() {
        Ok(_child) => Ok(()),
        Err(error) => Err(AppError::new(
            ErrorCode::Internal,
            "could not launch the system handler",
        )
        .with_detail(format!("{program}: {error}"))
        .with_hint(fallback_hint)),
    }
}

fn require_existing_file(path: &Path) -> AppResult<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(
            AppError::new(ErrorCode::NotFound, "the path does not exist")
                .with_detail(path.display().to_string()),
        )
    }
}

/// 在系统文件管理器中显示并选中一个文件（目录则进入）。
pub fn reveal_in_file_manager(path: &Path) -> AppResult<()> {
    require_existing_file(path)?;
    let template = reveal_command(std::env::consts::OS).ok_or_else(|| not_supported(path))?;
    let target = path.display().to_string();
    let parent = path
        .parent()
        .map(|parent| parent.display().to_string())
        .unwrap_or_else(|| target.clone());
    spawn_opener(
        template,
        &[("{path}", target.clone()), ("{parent}", parent)],
        target,
    )
}

/// 用系统默认应用打开一个文件。
pub fn open_file_with_default(path: &Path) -> AppResult<()> {
    require_existing_file(path)?;
    let template = default_app_command(std::env::consts::OS).ok_or_else(|| not_supported(path))?;
    let target = path.display().to_string();
    spawn_opener(template, &[("{path}", target.clone())], target)
}

fn not_supported(path: &Path) -> AppError {
    AppError::new(ErrorCode::Internal, "not supported on this platform")
        .with_detail(std::env::consts::OS.to_owned())
        .with_hint(path.display().to_string())
}

/// 开机自启能力（trait：真实实现 + 测试实现，T6.9 要求）。
pub trait Autostart: Send + Sync {
    /// 当前是否已启用。
    fn status(&self) -> AppResult<bool>;
    /// 启用（重复调用应幂等）。
    fn enable(&self) -> AppResult<()>;
    /// 禁用（未启用时调用也返回 Ok）。
    fn disable(&self) -> AppResult<()>;
}

/// XDG 自启（Linux）：`<autostart_dir>/forgedesk.desktop`。
#[derive(Debug)]
pub struct XdgAutostart {
    exe: PathBuf,
    autostart_dir: PathBuf,
}

impl XdgAutostart {
    /// `exe` 是当前可执行文件；`autostart_dir` 生产环境传 `$XDG_CONFIG_HOME/autostart`
    /// （缺省 `~/.config/autostart`），测试注入临时目录。
    pub fn new(exe: PathBuf, autostart_dir: PathBuf) -> Self {
        Self { exe, autostart_dir }
    }
}

/// desktop 条目内容（纯函数，测试直接断言）。
fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=ForgeDesk\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    )
}

const DESKTOP_FILE: &str = "forgedesk.desktop";

impl Autostart for XdgAutostart {
    fn status(&self) -> AppResult<bool> {
        Ok(self.autostart_dir.join(DESKTOP_FILE).is_file())
    }

    fn enable(&self) -> AppResult<()> {
        std::fs::create_dir_all(&self.autostart_dir).map_err(|error| {
            AppError::new(
                ErrorCode::Internal,
                "could not create the autostart directory",
            )
            .with_detail(format!("{}: {error}", self.autostart_dir.display()))
        })?;
        std::fs::write(
            self.autostart_dir.join(DESKTOP_FILE),
            desktop_entry(&self.exe),
        )
        .map_err(|error| {
            AppError::new(ErrorCode::Internal, "could not write the autostart entry")
                .with_detail(error.to_string())
        })
    }

    fn disable(&self) -> AppResult<()> {
        let file = self.autostart_dir.join(DESKTOP_FILE);
        match std::fs::remove_file(&file) {
            Ok(()) => Ok(()),
            // 文件本就不存在：视为已禁用（幂等）
            Err(_) if !file.exists() => Ok(()),
            Err(error) => Err(AppError::new(
                ErrorCode::Internal,
                "could not remove the autostart entry",
            )
            .with_detail(error.to_string())),
        }
    }
}

/// macOS LaunchAgent：`<agents_dir>/com.forgedesk.ForgeDesk.plist`。
#[derive(Debug)]
pub struct LaunchAgentAutostart {
    exe: PathBuf,
    agents_dir: PathBuf,
}

impl LaunchAgentAutostart {
    /// `exe` 是当前可执行文件；`agents_dir` 生产环境传 `~/Library/LaunchAgents`，
    /// 测试注入临时目录。
    pub fn new(exe: PathBuf, agents_dir: PathBuf) -> Self {
        Self { exe, agents_dir }
    }
}

const PLIST_FILE: &str = "com.forgedesk.ForgeDesk.plist";

/// LaunchAgent plist 内容（纯函数）。
fn launch_agent_plist(exe: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         <key>Label</key><string>com.forgedesk.ForgeDesk</string>\n\
         <key>ProgramArguments</key><array><string>{}</string></array>\n\
         <key>RunAtLoad</key><true/>\n\
         </dict>\n</plist>\n",
        exe.display()
    )
}

impl Autostart for LaunchAgentAutostart {
    fn status(&self) -> AppResult<bool> {
        Ok(self.agents_dir.join(PLIST_FILE).is_file())
    }

    fn enable(&self) -> AppResult<()> {
        std::fs::create_dir_all(&self.agents_dir).map_err(|error| {
            AppError::new(
                ErrorCode::Internal,
                "could not create the LaunchAgents directory",
            )
            .with_detail(error.to_string())
        })?;
        std::fs::write(
            self.agents_dir.join(PLIST_FILE),
            launch_agent_plist(&self.exe),
        )
        .map_err(|error| {
            AppError::new(ErrorCode::Internal, "could not write the LaunchAgent")
                .with_detail(error.to_string())
        })
    }

    fn disable(&self) -> AppResult<()> {
        let file = self.agents_dir.join(PLIST_FILE);
        match std::fs::remove_file(&file) {
            Ok(()) => Ok(()),
            Err(_) if !file.exists() => Ok(()),
            Err(error) => Err(AppError::new(
                ErrorCode::Internal,
                "could not remove the LaunchAgent",
            )
            .with_detail(error.to_string())),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_reveal_command_carries_a_path_placeholder() {
        for os in ["windows", "macos", "linux"] {
            let (_, args) = reveal_command(os).expect("三大平台都要支持");
            assert!(
                args.iter()
                    .any(|arg| arg.contains("{path}") || arg.contains("{parent}")),
                "{os} 的 reveal 命令缺少路径占位符"
            );
        }
    }

    #[test]
    fn the_windows_default_app_command_uses_start_with_an_empty_title() {
        let (program, args) = default_app_command("windows").unwrap();
        assert_eq!(program, "cmd");
        assert_eq!(
            args,
            &["/c", "start", "", "{path}"],
            "缺空标题会让含空格路径被当成标题"
        );
    }

    #[test]
    fn the_desktop_entry_quotes_the_exec_path() {
        let content = desktop_entry(Path::new("/opt/My Apps/forgedesk"));
        assert!(content.contains("Exec=\"/opt/My Apps/forgedesk\""));
        assert!(content.starts_with("[Desktop Entry]"));
    }

    #[test]
    fn the_launch_agent_plist_declares_label_and_program() {
        let content = launch_agent_plist(Path::new(
            "/Applications/ForgeDesk.app/Contents/MacOS/forgedesk",
        ));
        assert!(content.contains("<key>Label</key><string>com.forgedesk.ForgeDesk</string>"));
        assert!(content.contains("<key>RunAtLoad</key><true/>"));
    }

    #[test]
    fn xdg_autostart_round_trips_enable_status_disable() {
        let root = std::env::temp_dir().join(format!("forgedesk-autostart-{}", std::process::id()));
        let dir = root.join("autostart");
        let _ = std::fs::remove_dir_all(&dir);
        let autostart = XdgAutostart::new(PathBuf::from("/opt/forgedesk"), dir.clone());

        assert!(!autostart.status().unwrap());
        autostart.enable().unwrap();
        assert!(autostart.status().unwrap());
        assert!(dir.join("forgedesk.desktop").is_file());
        // 幂等
        autostart.enable().unwrap();
        assert!(autostart.status().unwrap());
        autostart.disable().unwrap();
        assert!(!autostart.status().unwrap());
        // 未启用时再禁用也是 Ok
        autostart.disable().unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(windows)]
mod windows_autostart {
    //! Windows 注册表自启（HKCU Run 键）。仅 Windows 编译；测试只测纯函数，
    //! 真实注册表读写不能在单测里碰（会污染开发机配置）。

    use std::path::Path;

    use forgedesk_domain::{AppError, AppResult, ErrorCode};

    use super::Autostart;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "ForgeDesk";

    /// Run 键的值内容：带引号的 exe 路径（含空格路径必需）。
    fn run_value(exe: &Path) -> String {
        format!("\"{}\"", exe.display())
    }

    /// Windows 注册表实现。
    #[derive(Debug)]
    pub struct WindowsAutostart {
        exe: std::path::PathBuf,
    }

    impl WindowsAutostart {
        pub fn new(exe: std::path::PathBuf) -> Self {
            Self { exe }
        }

        fn open_key(&self, writable: bool) -> Result<winreg::RegKey, AppError> {
            let hive = winreg::enums::HKEY_CURRENT_USER;
            winreg::RegKey::predef(hive)
                .open_subkey_with_flags(
                    RUN_KEY,
                    if writable {
                        winreg::enums::KEY_SET_VALUE
                    } else {
                        winreg::enums::KEY_QUERY_VALUE
                    },
                )
                .map_err(|error| {
                    AppError::new(ErrorCode::Internal, "could not open the registry Run key")
                        .with_detail(error.to_string())
                })
        }
    }

    impl Autostart for WindowsAutostart {
        fn status(&self) -> AppResult<bool> {
            let key = self.open_key(false)?;
            Ok(key.get_value::<String, _>(VALUE_NAME).is_ok())
        }

        fn enable(&self) -> AppResult<()> {
            let key = self.open_key(true)?;
            key.set_value(VALUE_NAME, &run_value(&self.exe))
                .map_err(|error| {
                    AppError::new(ErrorCode::Internal, "could not write the Run value")
                        .with_detail(error.to_string())
                })
        }

        fn disable(&self) -> AppResult<()> {
            let key = self.open_key(true)?;
            // 不存在的值删除报错：先查后删，保持"未启用时禁用也返回 Ok"
            if key.get_value::<String, _>(VALUE_NAME).is_ok() {
                key.delete_value(VALUE_NAME).map_err(|error| {
                    AppError::new(ErrorCode::Internal, "could not delete the Run value")
                        .with_detail(error.to_string())
                })?;
            }
            Ok(())
        }
    }

    #[cfg(test)]
    #[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    mod tests {
        use super::*;

        #[test]
        fn the_run_value_quotes_the_executable_path() {
            assert_eq!(
                run_value(Path::new(r"C:\Program Files\ForgeDesk\forgedesk.exe")),
                r#""C:\Program Files\ForgeDesk\forgedesk.exe""#
            );
        }
    }
}

/// 当前平台的自启实现（不支持的平台返回"永远未启用"的占位，而不是报错崩溃）。
///
/// 每个平台一个 cfg 门控的函数体，互斥编译：运行时字符串匹配 + 编译期 cfg
/// 混用会让"不可能"分支用 unreachable 兜底，这里让它根本不存在。
#[cfg(windows)]
pub fn autostart_for_current_platform() -> Box<dyn Autostart> {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("forgedesk.exe"));
    Box::new(windows_autostart::WindowsAutostart::new(exe))
}

/// macOS：登录项走 `~/Library/LaunchAgents` 的 LaunchAgent plist。
///
/// 与 Windows 变体同一契约：exe 路径取不到时回退到打包后的安装位置。
/// （三个平台变体必须**各自**带文档：`missing_docs` 按 cfg 分别检查，
/// Windows 上编译不到这段并不代表它合规——2026-10-07 的真实教训。）
#[cfg(target_os = "macos")]
pub fn autostart_for_current_platform() -> Box<dyn Autostart> {
    let exe = std::env::current_exe().unwrap_or_else(|_| {
        std::path::PathBuf::from("/Applications/ForgeDesk.app/Contents/MacOS/forgedesk")
    });
    let dir = std::env::var("HOME")
        .map(|home| std::path::PathBuf::from(home).join("Library/LaunchAgents"))
        .unwrap_or_else(|_| std::path::PathBuf::from("Library/LaunchAgents"));
    Box::new(LaunchAgentAutostart::new(exe, dir))
}

/// Linux：遵循 XDG 自启规范（`$XDG_CONFIG_HOME/autostart`，缺省 `~/.config/autostart`）的 .desktop 文件。
///
/// 文档缺失只会在 Linux/macOS 的编译里被 `missing_docs` 拒绝——
/// 这是 cfg 门控代码必须逐变体自查的原因（见 macOS 变体的说明）。
#[cfg(target_os = "linux")]
pub fn autostart_for_current_platform() -> Box<dyn Autostart> {
    let exe =
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("/usr/bin/forgedesk"));
    let dir = std::env::var("XDG_CONFIG_HOME")
        .map(|config| std::path::PathBuf::from(config).join("autostart"))
        .or_else(|_| {
            std::env::var("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".config/autostart"))
        })
        .unwrap_or_else(|_| std::path::PathBuf::from(".config/autostart"));
    Box::new(XdgAutostart::new(exe, dir))
}

/// 其余平台：占位实现（状态恒为未启用，enable 报明确错误）。
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn autostart_for_current_platform() -> Box<dyn Autostart> {
    Box::new(UnsupportedAutostart)
}

/// 不支持平台的占位实现：状态恒为"未启用"，enable 报明确错误。
#[derive(Debug)]
pub struct UnsupportedAutostart;

impl Autostart for UnsupportedAutostart {
    fn status(&self) -> AppResult<bool> {
        Ok(false)
    }

    fn enable(&self) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::Internal,
            "launch at startup is not supported on this platform",
        )
        .with_detail(std::env::consts::OS.to_owned()))
    }

    fn disable(&self) -> AppResult<()> {
        Ok(())
    }
}
