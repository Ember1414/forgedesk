//! 提交详情命令（`git_commit_detail`，T2.4）。
//!
//! # 能力等级
//!
//! `ReadOnly`：只读展示提交元数据与统计——不改仓库状态、不写数据库，
//! 无需快照/审计（与 `git_log_page` 同一等级）。
//!
//! # 单文件 diff 为什么不在这里
//!
//! 文件列表的行级展开复用 `workspace_diff`（`DiffRequest` 已支持
//! `between` / `commit` 目标，父提交 oid 由详情命令下发），「复制为 patch」
//! 复用 `workspace_diff_patch`——不新增第二对 diff 命令，避免"同一份数据
//! 两个入口"（AGENTS §6 单一写入口的读侧镜像）。

use forgedesk_domain::AppResult;
use forgedesk_services::CommitDetail;
use tauri::State;

use crate::state::AppState;

/// 读取提交详情（元数据 + 统计 + 文件清单 + 状态标记）。能力等级：`ReadOnly`。
///
/// `parent_index`：合并提交的对比父（0 = 第一父，缺省；1 = 第二父）。
/// 非合并提交传非零值、或任意提交越界时返回 `VALIDATION`。
#[tauri::command(async)]
pub fn git_commit_detail(
    state: State<'_, AppState>,
    repo_id: i64,
    oid: String,
    parent_index: Option<usize>,
) -> AppResult<CommitDetail> {
    state
        .commit_detail_service()
        .detail(repo_id, &oid, parent_index)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::git::{DiffChangeKind, Signature, SignatureStatus};
    use forgedesk_services::CommitDetail;

    /// CommitDetail 序列化为 camelCase（与 docs/API.md 契约一致）。
    #[test]
    fn commit_detail_serializes_to_camel_case() {
        let detail = CommitDetail {
            meta: forgedesk_services::CommitMeta {
                oid: "a".repeat(40),
                short_oid: "aaaaaaa".to_owned(),
                parents: vec!["b".repeat(40)],
                author: Signature::new("Ada", "ada@example.com").with_time(1_700_000_000),
                committer: Signature::new("Ada", "ada@example.com").with_time(1_700_000_000),
                subject: "subject".to_owned(),
                body: Some("body".to_owned()),
                signature: SignatureStatus::Unsigned,
            },
            refs: vec!["HEAD -> main".to_owned()],
            stats: forgedesk_services::CommitStats {
                files_changed: 2,
                insertions: 10,
                deletions: 3,
            },
            files: vec![forgedesk_services::CommitFileChange {
                path: "src/lib.rs".to_owned(),
                original_path: None,
                kind: DiffChangeKind::Modified,
                binary: false,
                additions: 10,
                deletions: 3,
                truncated: false,
            }],
            is_merge: false,
            is_head: true,
            is_pushed: false,
            web_url: None,
            parent_index: 0,
        };

        let json = serde_json::to_value(&detail).unwrap();
        // 顶层字段
        assert!(json.get("isMerge").is_some());
        assert!(json.get("isHead").is_some());
        assert!(json.get("isPushed").is_some());
        assert!(json.get("webUrl").is_some());
        assert!(json.get("parentIndex").is_some());
        assert!(json.get("is_merge").is_none());
        // stats 与 meta 的字段
        assert!(json["stats"].get("filesChanged").is_some());
        assert!(json["meta"].get("shortOid").is_some());
        assert!(json["meta"].get("signature").is_some());
        // 文件项的 kind 序列化为小驼峰字符串
        assert_eq!(json["files"][0]["kind"], "modified");
    }
}
