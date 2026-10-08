//! 工作区文件系统命令族（T5.7）：文件树 / 读写 / 创建 / 重命名 / 删除。
//!
//! 业务逻辑在 [`forgedesk_services::workspace_fs`]；本层只做参数收敛与
//! repo_id → 工作目录解析。安全核心（canonicalize + 仓库根前缀校验 +
//! 软链逃逸拒绝）见该模块的 `resolve_within`——所有命令都经过它。

use forgedesk_domain::AppResult;
use forgedesk_services::workspace_fs::{self, FsEol, FsFileContent, FsNode, FS_READ_LIMIT};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

/// 把超界的工作目录访问统一成 `NOT_FOUND` / `VALIDATION`（调用方负责展示）。
fn prepare_root(state: &AppState, repo_id: i64) -> AppResult<std::path::PathBuf> {
    state.workspace_service().resolve_workdir(repo_id)
}

/// `fs_tree` 的请求。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsTreeRequest {
    /// 归属仓库（存储层记录 id）。
    pub repo_id: i64,
    /// 相对仓库根的目录路径；空串 = 仓库根。
    /// 相对仓库根的目录路径；空串 = 仓库根。
    #[serde(default)]
    pub path: String,
    /// 显示点开头的隐藏项（默认不显示）。
    #[serde(default)]
    pub show_hidden: bool,
    /// 显示被 .gitignore 忽略的项（默认不显示）。
    #[serde(default)]
    pub show_ignored: bool,
}

/// 列出目录下的一层节点（懒加载）。
///
/// 能力等级：`ReadOnly`。
#[tauri::command]
pub async fn fs_tree(state: State<'_, AppState>, request: FsTreeRequest) -> AppResult<Vec<FsNode>> {
    let root = prepare_root(&state, request.repo_id)?;
    workspace_fs::fs_tree(
        &root,
        &request.path,
        request.show_hidden,
        request.show_ignored,
    )
    .await
}

/// `fs_read` 的请求。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsReadRequest {
    /// 归属仓库。
    pub repo_id: i64,
    /// 相对仓库根的文件路径。
    pub path: String,
}

/// 读取文件（≤ 5MB；二进制只给元信息；EOL/BOM 如实上报）。
///
/// 能力等级：`ReadOnly`。
#[tauri::command(async)]
pub fn fs_read(state: State<'_, AppState>, request: FsReadRequest) -> AppResult<FsFileContent> {
    let root = prepare_root(&state, request.repo_id)?;
    workspace_fs::fs_read(&root, &request.path)
}

/// `fs_write` 的请求。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsWriteRequest {
    /// 归属仓库。
    pub repo_id: i64,
    /// 相对仓库根的文件路径。
    pub path: String,
    /// 新内容（UTF-8 文本）。
    pub content: String,
    /// 保留原换行符（缺省 = LF）。
    #[serde(default)]
    pub eol: Option<FsEol>,
    /// 保留原 BOM（缺省 = 无）。
    #[serde(default)]
    pub has_bom: bool,
}

/// `fs_write` 的返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsWriteOutcome {
    /// 实际写入的字节数（含 EOL 规范化与 BOM 的差异）。
    pub written_bytes: u64,
}

/// 写入文件（EOL/BOM 按调用方声明的原文件形态恢复；成功后前端刷新 Git 状态）。
///
/// 能力等级：`Mutating`。
#[tauri::command(async)]
pub fn fs_write(state: State<'_, AppState>, request: FsWriteRequest) -> AppResult<FsWriteOutcome> {
    let root = prepare_root(&state, request.repo_id)?;
    if request.content.len() as u64 > FS_READ_LIMIT {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "the content exceeds the 5MB inline-edit limit",
        ));
    }
    let written = workspace_fs::fs_write(
        &root,
        &request.path,
        &request.content,
        request.eol.unwrap_or(FsEol::Lf),
        request.has_bom,
    )?;
    Ok(FsWriteOutcome {
        written_bytes: written,
    })
}

/// `fs_create` 的请求。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsCreateRequest {
    /// 归属仓库。
    pub repo_id: i64,
    /// 目标路径。
    pub path: String,
    /// true = 目录；false = 空文件。
    pub is_dir: bool,
}

/// 创建文件 / 目录。
///
/// 能力等级：`Mutating`。
#[tauri::command(async)]
pub fn fs_create(state: State<'_, AppState>, request: FsCreateRequest) -> AppResult<()> {
    let root = prepare_root(&state, request.repo_id)?;
    workspace_fs::fs_create(&root, &request.path, request.is_dir)
}

/// `fs_rename` 的请求。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsRenameRequest {
    /// 归属仓库。
    pub repo_id: i64,
    /// 源路径。
    pub path: String,
    /// 目标路径（不得逃逸仓库根）。
    pub new_path: String,
}

/// 重命名 / 移动（目标同样不得逃逸仓库根）。
///
/// 能力等级：`Mutating`。
#[tauri::command(async)]
pub fn fs_rename(state: State<'_, AppState>, request: FsRenameRequest) -> AppResult<()> {
    let root = prepare_root(&state, request.repo_id)?;
    workspace_fs::fs_rename(&root, &request.path, &request.new_path)
}

/// 删除（移入回收站——不是永久删除）。
///
/// 能力等级：`Mutating`。
#[tauri::command(async)]
pub fn fs_delete(state: State<'_, AppState>, path: String, repo_id: i64) -> AppResult<()> {
    let root = prepare_root(&state, repo_id)?;
    workspace_fs::fs_delete(&root, &path)
}
