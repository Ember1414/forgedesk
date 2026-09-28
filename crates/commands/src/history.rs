//! 历史查询命令（`git_log_page`，T2.2）。
//!
//! # 能力等级
//!
//! `ReadOnly`：只读遍历提交图——不改仓库状态、不写数据库，无需快照/审计。
//!
//! # 参数形状
//!
//! `query.paths` 的 JSON 形状是 `string[]`（与 `DiffRequest.paths` 的先例一致）；
//! 前端传相对仓库根的路径字符串，后端经 `RepoPath::from(String)` 转换。

use forgedesk_domain::AppResult;
use forgedesk_services::{HistoryPage, HistoryQuery};
use tauri::State;

use crate::state::AppState;

/// 分页拉取提交历史与泳道布局。能力等级：`ReadOnly`。
///
/// 返回一页提交（新 → 旧）及其泳道布局（每个提交落在哪条泳道、什么颜色、
/// 与父提交的边怎么连）和下一页游标。布局本身是纯函数
/// （`domain::history::layout`），查询与布局在同一个调用里完成——
/// 拆成两次 IPC 会让前端拿到不一致的数据。
///
/// # 无快照 / 无审计
///
/// 只读遍历不改仓库状态，因此不需要经 `SnapshotManager` 打点，
/// 也不写审计日志（与 `workspace_status` 同一等级）。
#[tauri::command]
pub fn git_log_page(
    state: State<'_, AppState>,
    repo_id: i64,
    query: HistoryQuery,
) -> AppResult<HistoryPage> {
    state.history_service().page(repo_id, &query)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_services::{HistoryQuery, MAX_HISTORY_PAGE_SIZE};

    /// HistoryQuery 从 JSON 反序列化：全字段缺省时得到与 Default 一致的结果。
    #[test]
    fn an_empty_json_object_deserializes_to_the_default_query() {
        let query: HistoryQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(query, HistoryQuery::default());
        assert_eq!(query.page_size, 100);
    }

    /// 前端传 camelCase 字段名，后端正确反序列化。
    #[test]
    fn camel_case_fields_deserialize_correctly() {
        let json = r#"{
            "revision": "main",
            "allBranches": true,
            "paths": ["src/lib.rs", "README.md"],
            "author": "ada",
            "since": 1700000000,
            "until": 1700100000,
            "messageContains": "fix",
            "firstParentOnly": true,
            "followRenames": true,
            "collapseMergedBranches": true,
            "pageSize": 50,
            "cursor": 10
        }"#;
        let query: HistoryQuery = serde_json::from_str(json).unwrap();

        assert_eq!(query.revision.as_deref(), Some("main"));
        assert!(query.all_branches);
        assert_eq!(query.paths.len(), 2);
        assert_eq!(query.paths[0].to_string(), "src/lib.rs");
        assert_eq!(query.author.as_deref(), Some("ada"));
        assert_eq!(query.since, Some(1_700_000_000));
        assert_eq!(query.until, Some(1_700_100_000));
        assert_eq!(query.message_contains.as_deref(), Some("fix"));
        assert!(query.first_parent_only);
        assert!(query.follow_renames);
        assert!(query.collapse_merged_branches);
        assert_eq!(query.page_size, 50);
        assert_eq!(query.cursor, Some(10));
    }

    /// paths 的 JSON 形状是 string[]（与 DiffRequest 先例一致）。
    #[test]
    fn paths_accept_a_json_string_array() {
        let json = r#"{"paths": ["a/b.txt", "c.txt"]}"#;
        let query: HistoryQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.paths.len(), 2);
        assert_eq!(query.paths[0].to_string(), "a/b.txt");
    }

    /// HistoryPage 序列化为 camelCase（与 docs/API.md 契约一致）。
    #[test]
    fn history_page_serializes_to_camel_case() {
        use forgedesk_domain::git::{Commit, Signature, SignatureStatus};
        use forgedesk_domain::history::{GraphLayout, GraphRow};
        use forgedesk_services::HistoryPage;

        let page = HistoryPage {
            commits: vec![Commit {
                oid: "abc123".to_owned(),
                parents: vec![],
                author: Signature::new("Ada", "ada@example.com").with_time(1700000000),
                committer: Signature::new("Ada", "ada@example.com").with_time(1700000000),
                refs: vec!["HEAD -> main".to_owned()],
                signature: SignatureStatus::Unsigned,
                subject: "initial".to_owned(),
                body: None,
            }],
            layout: GraphLayout {
                rows: vec![GraphRow {
                    oid: "abc123".to_owned(),
                    lane: 0,
                    row: 0,
                    color_index: 0,
                    is_merge: false,
                    hidden: false,
                    collapsed: vec![],
                }],
                edges: vec![],
                lane_count: 1,
            },
            next_cursor: Some(1),
        };

        let json = serde_json::to_value(&page).unwrap();
        // 顶层字段名是 camelCase
        assert!(json.get("nextCursor").is_some());
        assert!(json.get("next_cursor").is_none());
        // Commit 字段名是 camelCase
        let commit = &json["commits"][0];
        assert!(commit.get("signature").is_some());
        // SignatureStatus 序列化为 camelCase
        assert_eq!(commit["signature"], "unsigned");
        // GraphRow 字段名是 camelCase
        let row = &json["layout"]["rows"][0];
        assert!(row.get("colorIndex").is_some());
        assert!(row.get("isMerge").is_some());
        assert!(row.get("color_index").is_none());
        // laneCount
        assert!(json["layout"].get("laneCount").is_some());
    }

    /// EdgeKind 序列化为小写字符串。
    #[test]
    fn edge_kind_serializes_as_lowercase() {
        use forgedesk_domain::history::EdgeKind;

        assert_eq!(
            serde_json::to_value(EdgeKind::Straight).unwrap(),
            "straight"
        );
        assert_eq!(serde_json::to_value(EdgeKind::Merge).unwrap(), "merge");
        assert_eq!(serde_json::to_value(EdgeKind::Branch).unwrap(), "branch");
    }

    /// page_size 上限被钳制（服务层负责，此处只确认常量存在且与文档一致）。
    #[test]
    fn max_page_size_matches_the_documented_limit() {
        assert_eq!(MAX_HISTORY_PAGE_SIZE, 500);
    }
}
