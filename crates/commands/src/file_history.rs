//! 文件级历史命令（T5.8）：blame / 文件历史 / 历史版本内容。
//!
//! 业务在 [`forgedesk_services::file_history`]；本层只做参数收敛
//! （路径非空、limit 夹取）与 repo_id 解析。

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_services::file_history::{BlameOptions, FileHistoryPage};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

fn prepare_root(state: &AppState, repo_id: i64) -> AppResult<std::path::PathBuf> {
    state.workspace_service().resolve_workdir(repo_id)
}

fn validate_path(path: &str) -> AppResult<()> {
    if path.is_empty() || path.contains('\0') {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the file path is empty or invalid",
        ));
    }
    Ok(())
}

/// `git_blame` 的请求。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBlameRequest {
    /// 归属仓库。
    pub repo_id: i64,
    /// 相对仓库根的文件路径。
    pub path: String,
    /// 忽略行尾空白。
    #[serde(default)]
    pub ignore_whitespace: bool,
    /// 检测移动/复制的行。
    #[serde(default)]
    pub detect_moves: bool,
    /// `-L <start>,<end>`（降级模式：只 blame 可见区间）。
    pub range: Option<String>,
}

/// blame 命令（逐行归属）。
///
/// 能力等级：`ReadOnly`。
#[tauri::command]
pub async fn git_blame(
    state: State<'_, AppState>,
    request: GitBlameRequest,
) -> AppResult<Vec<forgedesk_git_engine::parsers::blame::BlameLine>> {
    validate_path(&request.path)?;
    let root = prepare_root(&state, request.repo_id)?;
    let options = BlameOptions {
        ignore_whitespace: request.ignore_whitespace,
        detect_moves: request.detect_moves,
        range: request.range.clone(),
    };
    forgedesk_services::file_history::git_blame(&root, &request.path, &options).await
}

/// `git_file_history` 的请求。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileHistoryRequest {
    /// 归属仓库。
    pub repo_id: i64,
    /// 相对仓库根的文件路径。
    pub path: String,
    /// 跟随重命名（默认开）。
    #[serde(default = "default_true")]
    pub follow: bool,
    /// 每页条数（后端夹取 1..=200）。
    pub limit: Option<u32>,
    /// 分页游标（offset）。
    pub cursor: Option<u64>,
}

fn default_true() -> bool {
    true
}

/// 文件历史（分页；`--follow` 跟随重命名，变更类型 A/M/D/R）。
///
/// 能力等级：`ReadOnly`。
#[tauri::command]
pub async fn git_file_history(
    state: State<'_, AppState>,
    request: GitFileHistoryRequest,
) -> AppResult<FileHistoryPage> {
    validate_path(&request.path)?;
    let root = prepare_root(&state, request.repo_id)?;
    let limit = request.limit.unwrap_or(50).clamp(1, 200);
    forgedesk_services::file_history::git_file_history(
        &root,
        &request.path,
        request.follow,
        limit,
        request.cursor.unwrap_or(0),
    )
    .await
}

/// `git_file_at` 的返回（base64 内容；二进制原样编码）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAtDto {
    /// base64 编码的文件内容（二进制安全传输）。
    pub content_base64: String,
    /// 是否二进制（前 8000 字节含 NUL）。
    pub is_binary: bool,
    /// 原始字节数。
    pub size: u64,
}

/// 读取历史版本的文件内容。
///
/// 能力等级：`ReadOnly`。
#[tauri::command]
pub async fn git_file_at(
    state: State<'_, AppState>,
    repo_id: i64,
    path: String,
    rev: String,
) -> AppResult<FileAtDto> {
    validate_path(&path)?;
    let root = prepare_root(&state, repo_id)?;
    let (bytes, is_binary) =
        forgedesk_services::file_history::git_file_at(&root, &path, &rev).await?;
    use base64::Engine as _;
    Ok(FileAtDto {
        content_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
        is_binary,
        size: bytes.len() as u64,
    })
}
