//! 终端会话核心（T5.1 Spike → T5.2 在其上补全会话管理）。
//!
//! # 职责与边界
//!
//! 把 `portable-pty` 的「创建 → 读写 → resize → 退出」包成一个与 Tauri 无关的
//! [`PtySession`]；PTY 输出经 **16ms 合并**后回调给调用方（命令层负责 base64 编码
//! 与事件发送，spike 的实测数据见 `docs/PTY-SPIKE.md`）。
//!
//! 为什么放在 `services` 而不是 `platform`：任务书（AGENT-PROMPTS T5.2）把终端
//! 会话管理定在 `crates/services/terminal`；本模块不感知 Tauri，因此能在纯 Rust
//! 集成测试里驱动**真实 PTY**（`tests/terminal.rs`，Windows 实测 ConPTY）。
//!
//! # 线程模型
//!
//! 每个会话三条线程，互不阻塞 IPC 线程：
//!
//! ```text
//! reader  ：阻塞读 PTY → 写入合并缓冲（PTY 关闭时自然 EOF 退出）
//! flusher ：每 16ms 取走缓冲 → on_output 回调（stop 标记后做最后一次排水再退出）
//! waiter  ：等待子进程退出 → join 前两条线程 → 保证"最后一批输出先于 exit 事件"
//! ```
//!
//! 为什么不用 Condvar 唤醒 flusher：合并的目的就是"攒满一个时间窗再发"，
//! 逐字节唤醒只会把窗拆碎；62 次/秒的定时唤醒对 CPU 的贡献 < 0.1%，换来的是
//! 实现简单与每事件载荷更大（IPC 次数更少）。

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use forgedesk_domain::{AppError, ErrorCode};

/// 输出合并窗口：每次回调之间至少间隔这么久（T5.2 任务书指定 16ms）。
pub const OUTPUT_COALESCE_INTERVAL: Duration = Duration::from_millis(16);

/// PTY 读取块大小：ConPTY/openpty 的典型管道粒度，8KB 足以吃满吞吐又不占内存。
const READ_CHUNK_SIZE: usize = 8 * 1024;

/// 一个 shell 候选项：程序名 + 启动参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCommand {
    /// 程序名（交给系统按 PATH 解析，如 `powershell` / `bash`）。
    pub program: String,
    /// 启动参数（不含程序名本身）。
    pub args: Vec<String>,
}

impl ShellCommand {
    fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
        }
    }
}

/// 按优先级返回本平台的 shell 候选列表。
///
/// `preferred`（大小写不敏感的前缀匹配，如 `"pwsh"`、`"bash"`）会被提到最前；
/// 匹配不到任何候选时原样返回默认列表——**宁可回落默认也不报错**：
/// 终端拿不到首选 shell 时给一个能用的 shell 比一个错误对话框有价值。
///
/// Windows 刻意不把 `bash` 放进默认列表：`System32\bash.exe` 是 WSL 而不是
/// Git Bash，静默落进 WSL 会让用户在错误的文件系统里执行 git 命令。
/// Git Bash 的显式探测（安装路径定位）是 T5.2 shell 选择器的一部分。
#[must_use]
pub fn shell_candidates(preferred: Option<&str>) -> Vec<ShellCommand> {
    let mut candidates = if cfg!(windows) {
        vec![
            ShellCommand::new("pwsh", &["-NoLogo"]),
            ShellCommand::new("powershell", &["-NoLogo"]),
            ShellCommand::new("cmd", &[]),
        ]
    } else {
        let login_shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty());
        let mut list = Vec::new();
        if let Some(shell) = login_shell {
            list.push(ShellCommand::new(&shell, &[]));
        }
        list.push(ShellCommand::new("bash", &[]));
        list.push(ShellCommand::new("zsh", &[]));
        list.push(ShellCommand::new("sh", &[]));
        list
    };

    if let Some(want) = preferred {
        let want = want.to_ascii_lowercase();
        if let Some(index) = candidates
            .iter()
            .position(|c| c.program.to_ascii_lowercase().starts_with(&want))
        {
            let picked = candidates.remove(index);
            candidates.insert(0, picked);
        }
    }
    candidates
}

/// 会话创建参数。
#[derive(Debug, Clone)]
pub struct PtyConfig {
    /// shell 的初始工作目录（终端默认是仓库根，由命令层解析后传入）。
    pub cwd: PathBuf,
    /// 初始列数（< 2 会被夹到 2：ConPTY 对 0 列的行为未定义）。
    pub cols: u16,
    /// 初始行数（< 2 会被夹到 2）。
    pub rows: u16,
    /// 优先使用的 shell（`None` = 本平台默认候选顺序）。
    pub shell: Option<String>,
    /// 输出中出现 DSR 光标位置请求（`ESC[6n`）时自动以 `ESC[1;1R` 应答。
    ///
    /// 为什么需要：PSReadLine 等交互 shell 启动时会向"终端"查询光标位置并**阻塞等待**；
    /// 无头读取端（spike、测试）不应答，shell 就永远停在启动阶段（T5.1 实测复现）。
    /// 前端 xterm.js 会原生应答 DSR，因此正式终端保持 `false`（避免重复应答
    /// 混进输入流）；spike 与集成测试必须开 `true`。
    pub auto_reply_dsr: bool,
}

/// 会话的事件回调：输出块与退出通知。
///
/// 为什么用回调而不是通道：命令层要在回调里做 base64 + Tauri emit，
/// spike 示例与测试则只是攒字节——两边各自组装 [`TerminalCallbacks`]，
/// 会话核心不关心下游是谁。
#[derive(Clone)]
pub struct TerminalCallbacks {
    /// 一次合并窗口内的输出（≤ 16ms 的量 + 读取块边界）。
    pub on_output: Arc<dyn Fn(&[u8]) + Send + Sync>,
    /// 子进程退出；`None` 表示拿不到退出码（被杀 / ConPTY 未报告）。
    pub on_exit: Arc<dyn Fn(Option<u32>) + Send + Sync>,
}

/// 16ms 合并缓冲：reader 线程写入，flusher 线程定时取走。
struct PendingOutput {
    buf: Mutex<Vec<u8>>,
}

impl PendingOutput {
    fn new() -> Self {
        Self {
            buf: Mutex::new(Vec::new()),
        }
    }

    fn push(&self, bytes: &[u8]) {
        self.buf.lock().extend_from_slice(bytes);
    }

    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.buf.lock())
    }
}

/// 一个活的终端会话（PTY master + 子进程 + 三条服务线程）。
///
/// 用完必须显式调用 [`PtySession::close`]：杀掉子进程后 reader 会因 EOF
/// 自然退出，flusher 做最后一次排水，waiter 补发 exit 事件。
pub struct PtySession {
    program: String,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    /// master 句柄槽：reader/writer 都是它的克隆，会话期间必须活着；
    /// `None` = waiter 已在子进程退出后释放它（ConPTY 输出管道随之 EOF）。
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    exited: Arc<AtomicBool>,
}

impl PtySession {
    /// 创建会话并启动服务线程。
    ///
    /// shell 按候选顺序尝试 spawn：第一个启动失败的候选（不存在 / 权限不足）
    /// 静默换下一个，全部失败才报错——报错码是 [`ErrorCode::NotFound`]（找不到
    /// 可用 shell），PTY 本身创建失败才是 [`ErrorCode::PtyUnsupported`]。
    pub fn spawn(config: &PtyConfig, callbacks: TerminalCallbacks) -> Result<Self, AppError> {
        let cols = config.cols.clamp(2, 500);
        let rows = config.rows.clamp(2, 200);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| {
                AppError::new(
                    ErrorCode::PtyUnsupported,
                    "the platform pseudo-terminal could not be created",
                )
                .with_detail(error.to_string())
            })?;

        let candidates = shell_candidates(config.shell.as_deref());
        let mut spawned = None;
        let mut last_error = String::new();
        for candidate in &candidates {
            let mut command = CommandBuilder::new(&candidate.program);
            command.args(&candidate.args);
            command.cwd(&config.cwd);
            let result = pair.slave.spawn_command(command);
            match result {
                Ok(child) => {
                    spawned = Some((candidate.program.clone(), child));
                    break;
                }
                Err(error) => {
                    last_error = format!("{}: {}", candidate.program, error);
                }
            }
        }
        let (program, child) = spawned.ok_or_else(|| {
            AppError::new(ErrorCode::NotFound, "no usable shell could be spawned")
                .with_detail(last_error)
        })?;

        // writer / reader 必须在 slave 释放前拿好：Unix 上 slave 关闭后
        // 子进程写入才会变成 EOF 驱动的读端关闭（ConPTY 上这个 handle 无意义）。
        let writer = pair.master.take_writer().map_err(|error| {
            AppError::from_code(ErrorCode::Internal).with_detail(error.to_string())
        })?;
        let reader = pair.master.try_clone_reader().map_err(|error| {
            AppError::from_code(ErrorCode::Internal).with_detail(error.to_string())
        })?;
        let master = pair.master;
        drop(pair.slave);
        let master = Arc::new(Mutex::new(Some(master)));

        let writer = Arc::new(Mutex::new(writer));
        let exited = Arc::new(AtomicBool::new(false));
        let pending = Arc::new(PendingOutput::new());
        let stop = Arc::new(AtomicBool::new(false));

        // reader：阻塞读，直到子进程退出导致 EOF。
        // auto_reply_dsr：跨块拼接最近 4 字节再找 `ESC[6n`（请求序列可能被块边界切开）。
        let reader_pending = Arc::clone(&pending);
        let reader_writer = Arc::clone(&writer);
        let auto_reply_dsr = config.auto_reply_dsr;
        let reader_thread = std::thread::Builder::new()
            .name("term-reader".into())
            .spawn(move || {
                let mut reader = reader;
                let mut chunk = vec![0u8; READ_CHUNK_SIZE];
                let mut tail: Vec<u8> = Vec::new();
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            let data = &chunk[..n];
                            if auto_reply_dsr {
                                let mut joined = std::mem::take(&mut tail);
                                joined.extend_from_slice(data);
                                if contains_dsr_request(&joined) {
                                    let _ = reader_writer.lock().write_all(DSR_REPLY).inspect_err(
                                        |error| {
                                            tracing::debug!(%error, "dsr reply failed");
                                        },
                                    );
                                }
                                tail = joined
                                    .len()
                                    .checked_sub(DSR_REQUEST.len().saturating_sub(1))
                                    .map(|start| joined[start..].to_vec())
                                    .unwrap_or_default();
                            }
                            reader_pending.push(data);
                        }
                        Err(error) => {
                            tracing::debug!(%error, "terminal reader stopped");
                            break;
                        }
                    }
                }
            })
            .map_err(|error| {
                AppError::from_code(ErrorCode::Internal).with_detail(error.to_string())
            })?;

        // flusher：定时排水。stop 置位后再排一次水，保证尾部输出不丢。
        let flusher_pending = Arc::clone(&pending);
        let flusher_stop = Arc::clone(&stop);
        let flusher_output = Arc::clone(&callbacks.on_output);
        let flusher_thread = std::thread::Builder::new()
            .name("term-flusher".into())
            .spawn(move || loop {
                std::thread::sleep(OUTPUT_COALESCE_INTERVAL);
                let chunk = flusher_pending.take();
                if !chunk.is_empty() {
                    (flusher_output)(&chunk);
                }
                if flusher_stop.load(Ordering::Acquire) {
                    break;
                }
            })
            .map_err(|error| {
                AppError::from_code(ErrorCode::Internal).with_detail(error.to_string())
            })?;

        // waiter：退出码 → 释放 master → join 读/排线程 → exit 回调。join 保证了
        // "on_exit 之前所有输出都已回调"，否则前端会看到 exit 先于输出到达。
        //
        // 为什么必须先释放 master：ConPTY 的输出管道只在 pseudoconsole 关闭
        // （master drop）时 EOF，子进程退出并不会——不先关 master，reader 就
        // 永远等在 read 上，退出通知会被无限推迟（T5.1 实测复现）。
        // 释放前留 100ms 给 reader 排干管道尾部，减少被 ClosePseudoConsole 丢弃的尾巴。
        let exit_flag = Arc::clone(&exited);
        let exit_callback = Arc::clone(&callbacks.on_exit);
        let waiter_master = Arc::clone(&master);
        let mut child = child;
        let killer = child.clone_killer();
        std::thread::Builder::new()
            .name("term-waiter".into())
            .spawn(move || {
                let code = child.wait().ok().map(|status| status.exit_code());
                std::thread::sleep(Duration::from_millis(100));
                let _ = waiter_master.lock().take();
                let _ = reader_thread.join();
                stop.store(true, Ordering::Release);
                let _ = flusher_thread.join();
                exit_flag.store(true, Ordering::Release);
                (exit_callback)(code);
            })
            .map_err(|error| {
                AppError::from_code(ErrorCode::Internal).with_detail(error.to_string())
            })?;

        Ok(Self {
            program,
            writer,
            killer: Mutex::new(killer),
            master,
            exited,
        })
    }

    /// 实际启动的 shell 程序名（候选列表里哪个赢了）。
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// 会话是否已退出。
    #[must_use]
    pub fn is_exited(&self) -> bool {
        self.exited.load(Ordering::Acquire)
    }

    /// 向会话写入输入（用户键入的字节，含 `\r`）。
    pub fn write(&self, data: &[u8]) -> Result<(), AppError> {
        self.writer.lock().write_all(data).map_err(|error| {
            AppError::from_code(ErrorCode::Internal).with_detail(error.to_string())
        })
    }

    /// 调整 PTY 尺寸（前端 xterm 视图变化时调用）。
    ///
    /// 会话已退出（master 已释放）时返回错误：调用方应把它当作"视图已死"处理。
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), AppError> {
        let guard = self.master.lock();
        let master = guard.as_ref().ok_or_else(|| {
            AppError::new(
                ErrorCode::Internal,
                "the terminal session has already exited",
            )
        })?;
        master
            .resize(PtySize {
                rows: rows.clamp(2, 200),
                cols: cols.clamp(2, 500),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| {
                AppError::from_code(ErrorCode::Internal).with_detail(error.to_string())
            })
    }

    /// 关闭会话：杀掉子进程，剩余输出与 exit 事件由服务线程自然补发。
    ///
    /// 重复调用是安全的（kill 对已退出的进程是幂等错误，被忽略）。
    pub fn close(&self) {
        let _ = self.killer.lock().kill();
    }
}

/// DSR 光标位置请求：`ESC [ 6 n`。
const DSR_REQUEST: &[u8] = b"[6n";
/// 无头读取端的 DSR 应答：行 1 列 1（交互 shell 只关心"有应答"，不关心坐标）。
const DSR_REPLY: &[u8] = b"[1;1R";

fn contains_dsr_request(haystack: &[u8]) -> bool {
    haystack.len() >= DSR_REQUEST.len()
        && haystack
            .windows(DSR_REQUEST.len())
            .any(|w| w == DSR_REQUEST)
}

/// 等待会话退出（测试与 spike 用轮询而不是暴露 Condvar：调用频率极低）。
///
/// 返回是否在时限内等到退出。
#[must_use]
pub fn wait_for_exit(session: &PtySession, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if session.is_exited() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    session.is_exited()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Windows 默认候选必须是 PowerShell 系在前、cmd 兜底，且不出现 bash
    /// （System32 的 bash.exe 是 WSL，见 shell_candidates 的注释）。
    #[test]
    fn windows_defaults_prefer_powershell_and_exclude_bash() {
        let candidates = shell_candidates(None);
        assert!(!candidates.is_empty());
        if cfg!(windows) {
            assert!(
                candidates[0].program.starts_with("pwsh")
                    || candidates[0].program.starts_with("powershell")
            );
            assert!(candidates
                .iter()
                .all(|c| !c.program.to_ascii_lowercase().starts_with("bash")));
            assert!(candidates
                .iter()
                .any(|c| c.program.to_ascii_lowercase() == "cmd"));
        } else {
            // Unix：$SHELL（若有）在最前，且一定有 sh 兜底。
            assert_eq!(
                candidates.last().expect("non-empty").program,
                "sh",
                "sh must be the last-resort candidate"
            );
        }
    }

    /// 首选项按大小写不敏感前缀匹配并提到最前；匹配不到时保持默认顺序。
    #[test]
    fn preferred_shell_moves_to_front_case_insensitively() {
        let candidates = shell_candidates(Some("PWSH"));
        assert!(candidates[0].program.starts_with("pwsh"));

        let untouched = shell_candidates(Some("fish"));
        let default = shell_candidates(None);
        assert_eq!(untouched, default, "unknown preference must not reorder");
    }

    /// 合并缓冲：多次 push 累积，take 清空并返回全部字节（含跨块的多字节字符）。
    #[test]
    fn pending_output_accumulates_then_empties() {
        let pending = PendingOutput::new();
        pending.push("中文".as_bytes());
        pending.push(&[0xE6, 0x96]); // "文" 的前 2 字节
        pending.push(&[0x87]); // "文" 的第 3 字节：一个字符被拆进三次 push
        let drained = pending.take();
        assert_eq!(drained, "中文文".as_bytes());
        assert!(pending.take().is_empty(), "second take must be empty");
    }
}

#[cfg(test)]
mod dsr_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{contains_dsr_request, DSR_REQUEST};

    /// DSR 请求可能被读取块边界切开：reader 用"上一块的尾部 + 本块"拼接后再找。
    /// 这里锁定拼接窗口的宽度（DSR_REQUEST.len() - 1）足以覆盖跨块场景。
    #[test]
    fn dsr_request_is_detected_even_when_split_across_chunks() {
        assert!(contains_dsr_request(DSR_REQUEST));
        assert!(!contains_dsr_request(b"\x1b[6"));

        let tail_width = DSR_REQUEST.len() - 1;
        let previous_tail = &b"\x1b[6"[..DSR_REQUEST.len().saturating_sub(1).min(tail_width)];
        let next_chunk = b"n more output";
        let mut joined = previous_tail.to_vec();
        joined.extend_from_slice(next_chunk);
        assert!(contains_dsr_request(&joined), "split DSR must be detected");
    }
}
