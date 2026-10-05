//! 内嵌终端命令族（T5.2）。
//!
//! 会话的生杀大权在 [`forgedesk_services::terminal::TerminalRegistry`]（AppState 持有）；
//! 本层只做四件事：参数收敛（尺寸范围、cwd 逃逸校验）、repo_id → 工作目录解析、
//! 输出字节 → base64 事件的转发、以及错误统一转换。
//!
//! # 传输契约
//!
//! - `term:output` 载荷 `{ termId, data }`，`data` 是 **base64**（16ms 合并块）；
//! - `term_write` 的 `data` 是 **`Vec<u8>`**（JSON 数字数组）：键盘输入一次几字节，
//!   胖载荷只发生在输出方向，输入方向的便利性优先（与输出不对称是故意的，
//!   理由见 `docs/PTY-SPIKE.md` §3.1）。
//!
//! # 输出事件里的 term_id
//!
//! 回调在 `registry.create` 之前构造，而 id 由 create 生成——两者之间用共享槽位
//! 衔接：create 返回后立刻回填 id。回填前的那几纳秒理论上可能有首批输出
//! （空 id 事件，前端按"未知会话"丢弃）；实际 shell 启动要几百毫秒，窗口不存在。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_services::terminal::{
    available_shells, TerminalCallbacks, TerminalSpawnSpec, TerminalSummary,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;

/// 终端输出事件：`{ termId, data }`，`data` 为 16ms 合并块的 base64。
pub const EVENT_TERM_OUTPUT: &str = "term:output";
/// 终端退出事件：`{ termId, code }`（`code` 为 `None` 表示拿不到退出码）。
pub const EVENT_TERM_EXIT: &str = "term:exit";

/// 输出事件载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TermOutputPayload {
    /// 会话 id。
    pub term_id: String,
    /// base64 编码的输出块。
    pub data: String,
}

/// 退出事件载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TermExitPayload {
    /// 会话 id。
    pub term_id: String,
    /// 退出码。
    pub code: Option<u32>,
}

/// `term_create` 的请求。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TermCreateRequest {
    /// 归属仓库（存储层记录 id）。
    pub repo_id: i64,
    /// shell id（`term_shell_list` 给出的选项；缺省 = 平台默认）。
    pub shell: Option<String>,
    /// 初始目录（**必须落在仓库根内**；缺省 = 仓库根）。
    pub cwd: Option<String>,
    /// 初始列数（2..=500）。
    pub cols: u16,
    /// 初始行数（2..=200）。
    pub rows: u16,
    /// 额外环境变量。
    pub env: Option<BTreeMap<String, String>>,
}

/// `term_create` 的返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermCreatedDto {
    /// 会话 id。
    pub term_id: String,
    /// 实际启动的 shell 程序。
    pub program: String,
}

/// `term_shell_list` 的返回元素（id 的展示名由前端 i18n 提供）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermShellDto {
    /// 稳定 id（`default` / `pwsh` / `powershell` / `gitbash` / `cmd` / `bash` / `zsh`）。
    pub id: String,
    /// 程序（绝对路径或名字；仅用于展示与诊断）。
    pub program: String,
}

/// 校验尺寸并收敛 cwd。
fn prepare_spec(state: &AppState, request: &TermCreateRequest) -> AppResult<TerminalSpawnSpec> {
    if !(2..=500).contains(&request.cols) || !(2..=200).contains(&request.rows) {
        return Err(AppError::new(
            ErrorCode::Validation,
            "cols must be 2..=500 and rows must be 2..=200",
        ));
    }
    let root = state.workspace_service().resolve_workdir(request.repo_id)?;

    // cwd 必须在仓库根内（零信任）：canonicalize 双方后做前缀比较，
    // 防止 `..` 与符号链接逃逸（软链解析由 canonicalize 一并完成）。
    let cwd: PathBuf = match &request.cwd {
        Some(raw) => {
            let requested = Path::new(raw);
            if !requested.is_dir() {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    "the requested terminal cwd is not a directory",
                )
                .with_detail(raw.clone()));
            }
            let resolved = requested.canonicalize().map_err(|error| {
                AppError::new(
                    ErrorCode::Validation,
                    "the requested terminal cwd is invalid",
                )
                .with_detail(error.to_string())
            })?;
            let root_canonical = root.canonicalize().map_err(|error| {
                AppError::new(ErrorCode::Validation, "the repository root is invalid")
                    .with_detail(error.to_string())
            })?;
            if !resolved.starts_with(&root_canonical) {
                return Err(AppError::new(
                    ErrorCode::PermissionDenied,
                    "the requested terminal cwd escapes the repository root",
                )
                .with_detail(raw.clone()));
            }
            resolved
        }
        None => root,
    };

    Ok(TerminalSpawnSpec {
        repo_id: request.repo_id,
        cwd,
        shell: request.shell.clone(),
        cols: request.cols,
        rows: request.rows,
        env: request.env.clone().unwrap_or_default(),
    })
}

/// 创建终端会话。
///
/// 能力等级：`Mutating`（创建进程与线程；不改仓库数据）。
///
/// cwd 校验失败（逃逸 / 不存在）返回 `VALIDATION` / `PERMISSION_DENIED`；
/// 仓库未打开返回 `resolve_workdir` 的错误（`NOT_FOUND`）。
#[tauri::command]
pub fn term_create(
    state: State<'_, AppState>,
    app: AppHandle,
    request: TermCreateRequest,
) -> AppResult<TermCreatedDto> {
    let spec = prepare_spec(&state, &request)?;

    // 输出与退出回调共用一个 id 槽位（见模块头说明）。
    let term_id_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let output_slot = Arc::clone(&term_id_slot);
    let output_app = app.clone();
    let exit_slot = Arc::clone(&term_id_slot);
    let exit_app = app;

    let handle = state.terminals.create(
        &spec,
        TerminalCallbacks {
            on_output: Arc::new(move |data: &[u8]| {
                // 终端回显是用户的私有输出区，不经日志落盘（R8 约束的是日志），
                // 事件载荷按原始字节 base64 原样转发。
                let payload = TermOutputPayload {
                    term_id: output_slot
                        .lock()
                        .ok()
                        .and_then(|guard| guard.clone())
                        .unwrap_or_default(),
                    data: BASE64.encode(data),
                };
                if let Err(error) = output_app.emit(EVENT_TERM_OUTPUT, payload) {
                    tracing::debug!(%error, "term output emit failed");
                }
            }),
            on_exit: Arc::new(move |code: Option<u32>| {
                let payload = TermExitPayload {
                    term_id: exit_slot
                        .lock()
                        .ok()
                        .and_then(|guard| guard.clone())
                        .unwrap_or_default(),
                    code,
                };
                if let Err(error) = exit_app.emit(EVENT_TERM_EXIT, payload) {
                    tracing::debug!(%error, "term exit emit failed");
                }
            }),
        },
    )?;
    *term_id_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle.id.clone());

    Ok(TermCreatedDto {
        term_id: handle.id.clone(),
        program: handle.program.clone(),
    })
}

/// 向会话写入输入（键盘字节，含 `\r`）。
///
/// 能力等级：`Mutating`。
#[tauri::command]
pub fn term_write(state: State<'_, AppState>, term_id: String, data: Vec<u8>) -> AppResult<()> {
    let handle = state.terminals.get_or_err(&term_id)?;
    handle.session.write(&data)
}

/// 调整会话尺寸。
///
/// 能力等级：`Mutating`。
#[tauri::command]
pub fn term_resize(
    state: State<'_, AppState>,
    term_id: String,
    cols: u16,
    rows: u16,
) -> AppResult<()> {
    if !(2..=500).contains(&cols) || !(2..=200).contains(&rows) {
        return Err(AppError::new(
            ErrorCode::Validation,
            "cols must be 2..=500 and rows must be 2..=200",
        ));
    }
    let handle = state.terminals.get_or_err(&term_id)?;
    handle.session.resize(cols, rows)
}

/// 关闭并移除会话。
///
/// 能力等级：`Mutating`。
#[tauri::command]
pub fn term_close(state: State<'_, AppState>, term_id: String) -> AppResult<()> {
    if let Some(handle) = state.terminals.remove(&term_id) {
        handle.session.close();
    }
    Ok(())
}

/// 列出全部会话（跨仓库；前端按 repoId 过滤）。
///
/// 能力等级：`ReadOnly`。
#[tauri::command]
pub fn term_list(state: State<'_, AppState>) -> AppResult<Vec<TerminalSummary>> {
    Ok(state.terminals.list())
}

/// 读取会话的尾部输出（默认 200 行，上限 1000）。
///
/// 会话退出后依然可读（回滚缓冲独立于会话存活）——这是"退出后保留
/// 最后 1000 行供查看"的后端出口；前端 xterm 自己的缓冲是主视图。
///
/// 能力等级：`ReadOnly`。
#[tauri::command]
pub fn term_output_tail(
    state: State<'_, AppState>,
    term_id: String,
    lines: Option<u16>,
) -> AppResult<Vec<String>> {
    let handle = state.terminals.get_or_err(&term_id)?;
    let max = lines.unwrap_or(200).clamp(1, 1000) as usize;
    Ok(handle.scrollback.tail(max))
}

/// 列出本平台可选的 shell（终端 "+" 菜单的数据源）。
///
/// 能力等级：`ReadOnly`。
#[tauri::command]
pub fn term_shell_list() -> AppResult<Vec<TermShellDto>> {
    Ok(available_shells()
        .into_iter()
        .map(|option| TermShellDto {
            id: option.id,
            program: option.program,
        })
        .collect())
}
