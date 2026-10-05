//! PTY Spike 的调试通道（T5.1）。
//!
//! 稳定性约定：与 `debug.rs` 相同，本模块的命令**只在开发构建注册**
//! （见 `src-tauri` 的注册处），不允许被业务代码依赖。
//!
//! 存在的理由：spike 示例（`crates/services/examples/pty_spike.rs`）验证的是
//! "PTY → 读取 → 合并 → 回调"这段后端管线；本模块把同一条管线接到
//! **真实 Tauri 事件通道**上，让开发者能在 `__dev__/pty-spike` 页面里：
//!
//! 1. 验证二进制数据经 IPC 的传输方式（base64 字符串载荷，理由见 PTY-SPIKE.md）；
//! 2. 手工量测"后端 emit → 前端收到"这一跳的吞吐（页面自带计时）；
//! 3. 在写正式终端 UI 之前，用最简 UI（`<pre>` + 输入框）摸清 shell 行为。
//!
//! # 与正式终端（T5.2）的有意差异
//!
//! - `auto_reply_dsr: true`：本页面的输出区是 `<pre>` 而不是 xterm.js，
//!   没有应答 DSR（`ESC[6n`）的能力，PSReadLine 会卡在启动等待（spike 实测）；
//!   正式终端由 xterm.js 原生应答，届时 `term_create` 必须关掉它。
//! - 会话表用模块级 `OnceLock` 而不是 `AppState`：spike 工具不该挤占
//!   正式状态结构的字段（T5.2 的终端注册表才是AppState 的正式住户）。

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_services::terminal::{PtyConfig, PtySession, TerminalCallbacks};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// 终端输出事件：`{ id, data }`，`data` 为 16ms 合并块的 base64。
pub const EVENT_PTY_SPIKE_OUTPUT: &str = "pty-spike:output";
/// 会话退出事件：`{ id, code }`（`code` 为 `None` 时表示拿不到退出码）。
pub const EVENT_PTY_SPIKE_EXIT: &str = "pty-spike:exit";

/// 输出事件载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeOutputPayload {
    /// 会话 id。
    pub id: String,
    /// base64 编码的输出块。
    pub data: String,
}

/// 退出事件载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeExitPayload {
    /// 会话 id。
    pub id: String,
    /// 退出码（ConPTY 偶尔不报告）。
    pub code: Option<u32>,
}

/// `pty_spike_create` 的返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeCreateDto {
    /// 新会话 id（后续命令用它寻址）。
    pub id: String,
    /// 实际启动的 shell 程序名（候选列表里哪个赢了）。
    pub program: String,
}

/// `pty_spike_throughput` 的返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpikeThroughputDto {
    /// 收到的总行数（含回显）。
    pub lines: u64,
    /// 收到的总字节数（含回显与 VT 序列）。
    pub bytes: u64,
    /// 从写入命令到收满 10 万行的耗时（毫秒）。
    pub elapsed_ms: u64,
}

/// 会话的统计计数（吞吐量与页面状态行共用）。
#[derive(Debug, Default)]
struct SpikeStats {
    bytes: AtomicU64,
    newlines: AtomicU64,
}

struct SpikeEntry {
    session: Arc<PtySession>,
    stats: Arc<SpikeStats>,
}

/// spike 会话表（模块级：见文件头"与正式终端的有意差异"）。
fn sessions() -> &'static Mutex<HashMap<String, SpikeEntry>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, SpikeEntry>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取会话；id 不存在返回 `NOT_FOUND`（外部输入收敛，不静默）。
fn entry(id: &str) -> AppResult<SpikeEntry> {
    let guard = sessions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .get(id)
        .map(|entry| SpikeEntry {
            session: Arc::clone(&entry.session),
            stats: Arc::clone(&entry.stats),
        })
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "no such pty spike session"))
}

/// 创建一个 spike 会话。
///
/// 能力等级：`Mutating`（创建进程与线程；不触碰仓库数据）。
///
/// `cols`/`rows` 在命令层收敛到 2..=500 / 2..=200（与 `PtySession` 的夹取一致），
/// 越界返回 `VALIDATION` 而不是静默夹取——参数校验失败应当被看到。
#[tauri::command]
pub fn pty_spike_create(app: AppHandle, cols: u16, rows: u16) -> AppResult<SpikeCreateDto> {
    if !(2..=500).contains(&cols) || !(2..=200).contains(&rows) {
        return Err(AppError::new(
            ErrorCode::Validation,
            "cols must be 2..=500 and rows must be 2..=200",
        ));
    }

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let id = format!("pty-spike-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));

    let stats = Arc::new(SpikeStats::default());
    let output_stats = Arc::clone(&stats);
    let output_app = app.clone();
    let output_id = id.clone();
    let exit_id = id.clone();

    let config = PtyConfig {
        cwd: std::env::temp_dir(),
        cols,
        rows,
        shell: None,
        env: std::collections::BTreeMap::new(),
        // spike 前端是 <pre>，应答不了 DSR；正式终端（xterm.js）必须关掉它。
        auto_reply_dsr: true,
    };
    let callbacks = TerminalCallbacks {
        on_output: Arc::new(move |data| {
            output_stats
                .bytes
                .fetch_add(data.len() as u64, Ordering::Relaxed);
            output_stats.newlines.fetch_add(
                data.iter().filter(|b| **b == b'\n').count() as u64,
                Ordering::Relaxed,
            );
            let payload = SpikeOutputPayload {
                id: output_id.clone(),
                data: BASE64.encode(data),
            };
            if let Err(error) = output_app.emit(EVENT_PTY_SPIKE_OUTPUT, payload) {
                tracing::debug!(%error, "pty-spike output emit failed");
            }
        }),
        on_exit: Arc::new(move |code| {
            let payload = SpikeExitPayload {
                id: exit_id.clone(),
                code,
            };
            if let Err(error) = app.emit(EVENT_PTY_SPIKE_EXIT, payload) {
                tracing::debug!(%error, "pty-spike exit emit failed");
            }
        }),
    };

    let session = Arc::new(PtySession::spawn(&config, callbacks)?);
    let program = session.program().to_string();
    sessions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(id.clone(), SpikeEntry { session, stats });
    tracing::info!(%id, %program, "pty spike session created");
    Ok(SpikeCreateDto { id, program })
}

/// 向会话写入输入。
///
/// 能力等级：`Mutating`（驱动会话内进程）。
///
/// `data` 是 **base64 编码**的原始字节：键盘输入是二进制安全的字节流，
/// 与输出走同一条 base64 通道，前后端就不必为"这端是文本还是字节"各写一套。
#[tauri::command]
pub fn pty_spike_write(id: String, data: String) -> AppResult<()> {
    let entry = entry(&id)?;
    let bytes = BASE64.decode(data.as_bytes()).map_err(|error| {
        AppError::new(ErrorCode::Validation, "data is not valid base64")
            .with_detail(error.to_string())
    })?;
    entry.session.write(&bytes)
}

/// 调整会话尺寸。
///
/// 能力等级：`Mutating`。
#[tauri::command]
pub fn pty_spike_resize(id: String, cols: u16, rows: u16) -> AppResult<()> {
    let entry = entry(&id)?;
    entry.session.resize(cols, rows)
}

/// 关闭会话（杀子进程并从会话表移除；exit 事件由服务线程补发）。
///
/// 能力等级：`Mutating`。
#[tauri::command]
pub fn pty_spike_close(id: String) -> AppResult<()> {
    let entry = entry(&id)?;
    entry.session.close();
    sessions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&id);
    Ok(())
}

/// 吞吐量测试：向会话灌 10 万行输出，返回端到端统计。
///
/// 能力等级：`ReadOnly`（只创建临时夹具文件并读取既有会话；不改仓库）。
///
/// 实现与 spike 示例一致：预生成 10 万行文件，用 shell 的原生输出命令
/// （ReadAllText / type / cat）灌过 PTY——测的是输出流的能力而不是 shell 循环。
/// 本命令会等到收满 10 万行或 90 秒超时（async 轮询，不占住调用线程）。
#[tauri::command]
pub async fn pty_spike_throughput(id: String) -> AppResult<SpikeThroughputDto> {
    const TARGET_LINES: u64 = 100_000;
    const TIMEOUT: Duration = Duration::from_secs(90);

    let entry = entry(&id)?;
    let file = std::env::temp_dir().join("forgedesk-pty-spike-100k.txt");
    {
        let mut handle = std::fs::File::create(&file).map_err(|error| {
            AppError::new(ErrorCode::Storage, "could not create throughput fixture")
                .with_detail(error.to_string())
        })?;
        for i in 0..TARGET_LINES {
            let _ = writeln!(handle, "line-{i:06} 0123456789abcdef");
        }
    }

    let program = entry.session.program().to_ascii_lowercase();
    let path = file.to_string_lossy().replace('\\', "/");
    let command = if program.starts_with("pwsh") || program.starts_with("powershell") {
        format!(
            "[Console]::OutputEncoding=[Text.Encoding]::UTF8; [IO.File]::ReadAllText('{path}')\r"
        )
    } else if program == "cmd" {
        format!("type \"{path}\"\r\n")
    } else {
        format!("cat '{path}'\r")
    };

    let baseline = entry.stats.newlines.load(Ordering::Relaxed);
    let started = Instant::now();
    entry.session.write(command.as_bytes())?;

    let result = loop {
        let lines = entry.stats.newlines.load(Ordering::Relaxed) - baseline;
        if lines >= TARGET_LINES {
            break Ok(SpikeThroughputDto {
                lines,
                bytes: entry.stats.bytes.load(Ordering::Relaxed),
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }
        if started.elapsed() > TIMEOUT {
            break Err(
                AppError::new(ErrorCode::Network, "throughput fixture timed out")
                    .with_detail(format!("only {lines} lines in 90s")),
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    let _ = std::fs::remove_file(&file);
    result
}
