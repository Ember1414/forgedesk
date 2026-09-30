//! 快照记录（`snapshots` 表）。
//!
//! # 记录什么、为什么
//!
//! 一条快照要能回答"**回到这个时点**需要哪些事实"，缺一样回滚就只能靠猜：
//!
//! - `head_oid`：HEAD 的位置（回滚的主目标）；
//! - `index_tree_oid`：索引的树（`git write-tree`，T1.7 的指纹）——`reset --hard`
//!   只能把索引带到 HEAD 的树，恢复不了"已暂存但未提交"的内容；
//! - `reflog_ref`：一个**自定义 ref**（`refs/forgedesk/snapshots/<id>`）指向快照的
//!   HEAD 提交。它存在的唯一理由是**防止对象被 gc**：HEAD 移走之后，如果没有任何
//!   引用指着那个提交，`git gc` 会把它当垃圾收掉，回滚就永远失败了。
//!   刻意**不用** `git reflog` / `HEAD@{n}`：reflog 会被外部操作（别的工具、
//!   `git gc --prune=now`、clone）改写或清空，任务定义明确禁止把它当唯一依据；
//! - `branch` / `detached` / `operation_state`：分支语境。只恢复 oid 会让用户
//!   落在一个意料之外的 HEAD 上（比如正处于变基中途时打的快照）；
//! - `untracked_paths`：未跟踪文件清单（v1 只记路径，不备份内容——M1 的写路径
//!   不删除未跟踪文件，内容备份属于 T3.8 快照 v2）。
//!
//! # 脱敏（红线 R8）
//!
//! 路径与分支名可能包含用户名；它们会原样进库并在界面上显示，这与日志的处理一致：
//! 存储层只负责存，脱敏发生在**写入前**的调用方（services 层已统一过 `sanitize_log`）。

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use rusqlite::params;

use crate::database::{storage_error, Database};

/// 插入一条快照所需的信息。
#[derive(Debug, Clone)]
pub struct NewSnapshot {
    /// 存储层记录 id。
    pub repo_id: i64,
    /// 展示标签（如 `pre-commit`）。
    pub label: String,
    /// 场景短名（与 [`forgedesk_snapshot::SnapshotKind::key`] 一致）。
    pub kind: String,
    /// HEAD 的 oid。
    pub head_oid: String,
    /// 索引的树 oid（`git write-tree`）。
    pub index_tree_oid: String,
    /// 指向 `head_oid` 的自定义 ref（防 gc 锚点）。
    pub snapshot_ref: String,
    /// 当前的分支名；游离 HEAD 或空分支语境时为 `None`。
    pub branch: Option<String>,
    /// 是否游离 HEAD。
    pub detached: bool,
    /// 当时的仓库操作状态（合并 / 变基中途等）；正常时为 `None`。
    pub operation_state: Option<String>,
    /// 未跟踪文件的路径清单（JSON 数组文本）。
    pub untracked_paths: String,
    /// 未跟踪内容备份的清单（JSON 数组文本；空备份为 `[]`）。
    pub manifest_json: String,
    /// 备份内容的字节总数（无备份为 0）。
    pub backup_bytes: i64,
    /// 创建时间（Unix 毫秒）。
    pub created_at_ms: i64,
}

/// 一条快照记录（读出来的形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRecord {
    /// 主键。
    pub id: i64,
    /// 存储层记录 id。
    pub repo_id: i64,
    /// 展示标签。
    pub label: String,
    /// 场景短名。
    pub kind: String,
    /// HEAD 的 oid。
    pub head_oid: String,
    /// 索引的树 oid。
    pub index_tree_oid: String,
    /// 防 gc 的自定义 ref。
    pub snapshot_ref: String,
    /// 当时的分支名。
    pub branch: Option<String>,
    /// 是否游离 HEAD。
    pub detached: bool,
    /// 当时的操作状态。
    pub operation_state: Option<String>,
    /// 未跟踪文件路径清单（JSON 数组文本）。
    pub untracked_paths: String,
    /// 未跟踪内容备份的清单（JSON 数组文本）。
    pub manifest_json: String,
    /// 备份内容的字节总数。
    pub backup_bytes: i64,
    /// 创建时间（Unix 毫秒）。
    pub created_at: i64,
}

/// 快照记录仓储。
#[derive(Debug)]
pub struct SnapshotStore<'a> {
    database: &'a Database,
}

impl<'a> SnapshotStore<'a> {
    /// 绑定到某个数据库。
    pub const fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// 插入一条快照，返回主键。
    pub fn insert(&self, snapshot: &NewSnapshot) -> AppResult<i64> {
        self.database.with_write(|connection| {
            connection
                .execute(
                    "INSERT INTO snapshots (
                             repo_id, label, kind, head_oid, index_tree_oid, reflog_ref,
                             branch, detached, operation_state, untracked_paths,
                             manifest_json, backup_bytes, created_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        snapshot.repo_id,
                        snapshot.label,
                        snapshot.kind,
                        snapshot.head_oid,
                        snapshot.index_tree_oid,
                        snapshot.snapshot_ref,
                        snapshot.branch,
                        i64::from(snapshot.detached),
                        snapshot.operation_state,
                        snapshot.untracked_paths,
                        snapshot.manifest_json,
                        snapshot.backup_bytes,
                        snapshot.created_at_ms,
                    ],
                )
                .map_err(|error| storage_error("写入快照记录失败", &error))?;
            Ok(connection.last_insert_rowid())
        })
    }

    /// 按 id 读取；不存在返回 `None`。
    pub fn find(&self, id: i64) -> AppResult<Option<SnapshotRecord>> {
        self.database.with_read(|connection| {
            connection
                .query_row(
                    "SELECT id, repo_id, label, kind, head_oid, index_tree_oid, reflog_ref,
                            branch, detached, operation_state, untracked_paths,
                            manifest_json, backup_bytes, created_at
                     FROM snapshots WHERE id = ?1",
                    [id],
                    row_to_record,
                )
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(storage_error("读取快照记录失败", &other)),
                })
        })
    }

    /// 某个仓库的快照，新的在前。
    pub fn list(&self, repo_id: i64, limit: i64) -> AppResult<Vec<SnapshotRecord>> {
        self.database.with_read(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, repo_id, label, kind, head_oid, index_tree_oid, reflog_ref,
                            branch, detached, operation_state, untracked_paths,
                            manifest_json, backup_bytes, created_at
                     FROM snapshots WHERE repo_id = ?1
                     ORDER BY created_at DESC, id DESC LIMIT ?2",
                )
                .map_err(|error| storage_error("准备快照查询失败", &error))?;
            let records = statement
                .query_map([repo_id, limit], row_to_record)
                .map_err(|error| storage_error("读取快照列表失败", &error))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| storage_error("读取快照列表失败", &error))?;
            Ok(records)
        })
    }

    /// 删除一条快照记录。
    pub fn delete(&self, id: i64) -> AppResult<()> {
        self.database.with_write(|connection| {
            connection
                .execute("DELETE FROM snapshots WHERE id = ?1", [id])
                .map_err(|error| storage_error("删除快照记录失败", &error))?;
            Ok(())
        })
    }

    /// 回填锚点 ref 的列值。
    ///
    /// 为什么分两步（先插行、再回填 ref）：ref 名里含主键，插入之前不知道 id。
    /// 回填失败时调用方必须同时删掉锚点 ref 与这条记录——
    /// 表里不允许存在"记录说有锚点、ref 其实没打上"的快照。
    pub fn update_ref_column(&self, id: i64, snapshot_ref: &str) -> AppResult<()> {
        self.database.with_write(|connection| {
            connection
                .execute(
                    "UPDATE snapshots SET reflog_ref = ?1 WHERE id = ?2",
                    params![snapshot_ref, id],
                )
                .map_err(|error| storage_error("回填快照锚点失败", &error))?;
            Ok(())
        })
    }

    /// 改写一条快照的内容备份清单与体积。
    ///
    /// 用途只有一个：备份目录改名失败时把记录**回退**成"没有内容备份"。
    /// 磁盘与数据库必须一致——记录里写着有备份、目录却不存在，
    /// 恢复时会在一个不存在的路径上白费一次尝试，然后告诉用户"部分失败"。
    pub fn update_backup(&self, id: i64, manifest_json: &str, backup_bytes: i64) -> AppResult<()> {
        self.database.with_write(|connection| {
            connection
                .execute(
                    "UPDATE snapshots SET manifest_json = ?1, backup_bytes = ?2 WHERE id = ?3",
                    params![manifest_json, backup_bytes, id],
                )
                .map_err(|error| storage_error("回填快照备份清单失败", &error))?;
            Ok(())
        })
    }

    /// 保留策略的清理候选：按时间倒序跳过 `keep` 条之后剩下的记录。
    pub fn prune_candidates(
        &self,
        repo_id: i64,
        keep: i64,
        created_before_ms: i64,
    ) -> AppResult<Vec<SnapshotRecord>> {
        self.database.with_read(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, repo_id, label, kind, head_oid, index_tree_oid, reflog_ref,
                            branch, detached, operation_state, untracked_paths,
                            manifest_json, backup_bytes, created_at
                     FROM snapshots
                     WHERE repo_id = ?1
                       AND (id NOT IN (
                             SELECT id FROM snapshots WHERE repo_id = ?1
                             ORDER BY created_at DESC, id DESC LIMIT ?2
                           )
                           OR created_at < ?3)
                     ORDER BY created_at ASC",
                )
                .map_err(|error| storage_error("准备快照清理查询失败", &error))?;
            let records = statement
                .query_map(params![repo_id, keep, created_before_ms], row_to_record)
                .map_err(|error| storage_error("读取清理候选失败", &error))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| storage_error("读取清理候选失败", &error))?;
            Ok(records)
        })
    }

    /// 某仓库的快照数量（测试与诊断用）。
    pub fn count(&self, repo_id: i64) -> AppResult<i64> {
        self.database.with_read(|connection| {
            connection
                .query_row(
                    "SELECT COUNT(*) FROM snapshots WHERE repo_id = ?1",
                    [repo_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| storage_error("统计快照数量失败", &error))
        })
    }

    /// 某仓库备份内容的字节总数（磁盘配额与设置页占用显示都读它）。
    ///
    /// 用 SQL 求和而不是把记录读回来累加：配额检查发生在**每次创建快照**时，
    /// 一个仓库上百条快照、每条带一份 JSON 清单，为了一个整数把它们全读回来
    /// 是白费 IO。
    pub fn total_backup_bytes(&self, repo_id: i64) -> AppResult<i64> {
        self.database.with_read(|connection| {
            connection
                .query_row(
                    "SELECT COALESCE(SUM(backup_bytes), 0) FROM snapshots WHERE repo_id = ?1",
                    [repo_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| storage_error("统计快照备份体积失败", &error))
        })
    }

    /// 某仓库的全部快照，**最旧的在前**（磁盘配额按 LRU 清理时从它开始删）。
    ///
    /// 与 [`Self::list`] 相反的顺序是有意的：列表页要"新的在前"，
    /// 而配额清理要"先删最旧的"。让 SQL 决定顺序，调用方不必再排一次。
    pub fn list_oldest_first(&self, repo_id: i64) -> AppResult<Vec<SnapshotRecord>> {
        self.database.with_read(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, repo_id, label, kind, head_oid, index_tree_oid, reflog_ref,
                            branch, detached, operation_state, untracked_paths,
                            manifest_json, backup_bytes, created_at
                     FROM snapshots WHERE repo_id = ?1
                     ORDER BY created_at ASC, id ASC",
                )
                .map_err(|error| storage_error("准备快照查询失败", &error))?;
            let records = statement
                .query_map([repo_id], row_to_record)
                .map_err(|error| storage_error("读取快照列表失败", &error))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| storage_error("读取快照列表失败", &error))?;
            Ok(records)
        })
    }
}

/// 快照 id 必须是正数（外部输入的二次校验共用这个文案）。
pub fn ensure_snapshot_id(id: i64) -> AppResult<()> {
    if id <= 0 {
        return Err(AppError::new(
            ErrorCode::Validation,
            "snapshotId must be a positive record id",
        )
        .with_hint("snapshot_id"));
    }
    Ok(())
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<SnapshotRecord> {
    Ok(SnapshotRecord {
        id: row.get(0)?,
        repo_id: row.get(1)?,
        label: row.get(2)?,
        kind: row.get(3)?,
        head_oid: row.get(4)?,
        index_tree_oid: row.get(5)?,
        snapshot_ref: row.get(6)?,
        branch: row.get(7)?,
        detached: row.get::<_, i64>(8)? != 0,
        operation_state: row.get(9)?,
        untracked_paths: row.get(10)?,
        // 0003 前写入的历史记录这两列是 NULL：读成空清单 / 0 字节，
        // 与"这份快照没有内容备份"是同一件事
        manifest_json: row
            .get::<_, Option<String>>(11)?
            .unwrap_or_else(|| "[]".to_owned()),
        backup_bytes: row.get(12)?,
        created_at: row.get(13)?,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{NewSnapshot, SnapshotStore};
    use crate::database::Database;

    /// 带最新结构的内存库。
    fn memory_database() -> Database {
        let database = Database::open_in_memory().expect("打开内存库失败");
        crate::migrations::migrate(&database).expect("执行迁移失败");
        database
    }

    fn sample(repo_id: i64, created_at: i64) -> NewSnapshot {
        NewSnapshot {
            repo_id,
            label: "pre-commit".to_owned(),
            kind: "pre-commit".to_owned(),
            head_oid: "a".repeat(40),
            index_tree_oid: "b".repeat(40),
            snapshot_ref: format!("refs/forgedesk/snapshots/{created_at}"),
            branch: Some("main".to_owned()),
            detached: false,
            operation_state: None,
            untracked_paths: r#"["scratch.txt"]"#.to_owned(),
            manifest_json: "[]".to_owned(),
            backup_bytes: 0,
            created_at_ms: created_at,
        }
    }

    /// 带内容备份的样本（T3.8 的两列）。
    fn sample_with_backup(repo_id: i64, created_at: i64, bytes: i64) -> NewSnapshot {
        NewSnapshot {
            manifest_json: format!(r#"[{{"path":"scratch.txt","bytes":{bytes},"ignored":false}}]"#),
            backup_bytes: bytes,
            ..sample(repo_id, created_at)
        }
    }

    #[test]
    fn a_snapshot_round_trips_through_insert_find_and_list() {
        let database = memory_database();
        let store = SnapshotStore::new(&database);

        let id = store.insert(&sample(1, 1_000)).unwrap();
        let found = store.find(id).unwrap().expect("应能读回刚插入的快照");

        assert_eq!(found.label, "pre-commit");
        assert_eq!(found.branch.as_deref(), Some("main"));
        assert!(!found.detached);
        assert_eq!(found.untracked_paths, r#"["scratch.txt"]"#);
        assert_eq!(store.list(1, 10).unwrap().len(), 1);
    }

    #[test]
    fn listing_is_ordered_newest_first_and_respects_the_limit() {
        let database = memory_database();
        let store = SnapshotStore::new(&database);
        for created_at in [1_000, 2_000, 3_000] {
            store.insert(&sample(1, created_at)).unwrap();
        }

        let listed = store.list(1, 2).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].created_at, 3_000, "新的在前");
        assert_eq!(listed[1].created_at, 2_000);
    }

    #[test]
    fn prune_candidates_skip_the_newest_entries_regardless_of_age() {
        let database = memory_database();
        let store = SnapshotStore::new(&database);
        // 四条快照：两条很新、两条很旧
        for created_at in [1_000, 2_000, 90_000, 91_000] {
            store.insert(&sample(1, created_at)).unwrap();
        }

        // 保留最近 2 条，同时清理 50_000 之前的旧记录
        let candidates = store.prune_candidates(1, 2, 50_000).unwrap();
        let ids: Vec<i64> = candidates.iter().map(|record| record.id).collect();

        assert_eq!(
            ids.len(),
            2,
            "最旧的两条都该进清理候选（数量与年龄各命中一条）"
        );
        assert!(candidates.iter().all(|record| record.created_at < 50_000));
        store.delete(candidates[0].id).unwrap();
        assert_eq!(store.count(1).unwrap(), 3);
    }

    #[test]
    fn records_of_other_repositories_never_leak_into_a_list() {
        let database = memory_database();
        let store = SnapshotStore::new(&database);
        store.insert(&sample(1, 1_000)).unwrap();
        store.insert(&sample(2, 2_000)).unwrap();

        assert_eq!(store.list(1, 10).unwrap().len(), 1);
        assert_eq!(store.count(1).unwrap(), 1);
    }

    #[test]
    fn backup_columns_round_trip_and_the_total_is_per_repository() {
        let database = memory_database();
        let store = SnapshotStore::new(&database);
        let first = store.insert(&sample_with_backup(1, 1_000, 100)).unwrap();
        store.insert(&sample_with_backup(1, 2_000, 250)).unwrap();
        store.insert(&sample_with_backup(2, 3_000, 999)).unwrap();

        let found = store.find(first).unwrap().expect("应能读回刚插入的快照");
        assert_eq!(found.backup_bytes, 100);
        assert!(found.manifest_json.contains("scratch.txt"));

        // 配额按仓库算：别的仓库的体积不能算进来
        assert_eq!(store.total_backup_bytes(1).unwrap(), 350);
        assert_eq!(store.total_backup_bytes(2).unwrap(), 999);
    }

    #[test]
    fn oldest_first_listing_is_the_reverse_of_the_ui_listing() {
        let database = memory_database();
        let store = SnapshotStore::new(&database);
        for created_at in [1_000, 2_000, 3_000] {
            store.insert(&sample(1, created_at)).unwrap();
        }

        let oldest = store.list_oldest_first(1).unwrap();
        assert_eq!(oldest[0].created_at, 1_000, "配额清理从最旧的开始");
        assert_eq!(oldest[1].created_at, 2_000);
        assert_eq!(oldest[2].created_at, 3_000);
        assert_eq!(
            store.list(1, 10).unwrap()[0].created_at,
            3_000,
            "列表页仍新的在前"
        );
    }
}
