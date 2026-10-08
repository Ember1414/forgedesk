//! 子进程创建的平台适配：让控制台类程序（git / ssh / gpg / cmd）在 GUI 应用里**不弹窗**。
//!
//! # 为什么必须有这一层
//!
//! Windows 上，一个**没有控制台**的进程（发布构建的 ForgeDesk 就是：
//! `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`）
//! 去启动一个控制台程序（`git.exe`、`ssh.exe`、`gpg.exe`、`cmd.exe`）时，
//! 系统会为子进程**新建一个控制台窗口**，并把它显示在屏幕上闪一下。
//!
//! 于是"打开仓库"这种一次要跑十几条 git 命令的操作会连闪十几次黑框；
//! `CREATE_NO_WINDOW` 让子进程拿到一个不可见的控制台——它仍然有 stdin/stdout
//! （我们的管道读写完全正常），只是没有窗口。
//!
//! # 为什么集中在这里而不是各处就地写
//!
//! 调用点分散在 `git-engine`（git 执行器）、`commands`（ssh / gpg）、
//! `platform`（文件管理器 / 系统打开方式）四处，任何一处漏掉就"只剩那一个操作会闪"。
//! 而且这个标志在非 Windows 上是无效的——就地写 `#[cfg(windows)]` 会让
//! 每个调用点都多一段条件编译噪声。统一成 trait，调用点只写一行
//! `.no_console_window()`，语义在两种平台下都成立。
//!
//! # 明确不适用本层的场景
//!
//! 终端页（`crates/services` 的 PTY）**故意**要一个真实控制台——
//! 它就是给用户用的终端，弹窗即功能。

/// `CreateProcess` 的 `CREATE_NO_WINDOW` 标志值。
///
/// 子进程照常拿到标准句柄（我们的管道不受影响），只是不创建可见控制台窗口。
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 让子进程不弹出控制台窗口。
///
/// 非 Windows 上这些方法是无操作（空实现）——调用点因此不需要条件编译。
pub trait NoConsoleWindow {
    /// 抑制控制台窗口，返回 `&mut Self` 以便链式调用。
    fn no_console_window(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl NoConsoleWindow for std::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(not(windows))]
impl NoConsoleWindow for std::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        self
    }
}

#[cfg(windows)]
impl NoConsoleWindow for tokio::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(not(windows))]
impl NoConsoleWindow for tokio::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        self
    }
}
