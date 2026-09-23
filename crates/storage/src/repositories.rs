//! 仓库登记仓储（`repositories` 表）。
//!
//! M0/T0.7 只提供"建表 + CRUD"，**不含业务逻辑**：发现仓库、校验是否 Git 仓库、
//! 读取默认分支等属于 M1 的 `services` 层。这里刻意只做数据存取，
//! 是为了让"本地记录的仓库列表"这件事可以在没有 git 引擎的情况下被测试。

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::database::{storage_error, Database};

/// 一个被登记的本地仓库。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryRecord {
    /// 主键。
    pub id: i64,
    /// 仓库根目录的绝对路径（大小写与分隔符由调用方规范化后传入）。
    pub path: String,
    /// 展示名（通常是目录名）。
    pub name: String,
    /// 默认分支（未知时为 `None`，M1 由 git 引擎填充）。
    pub default_branch: Option<String>,
    /// 托管平台标识（如 `github`），未连接时为 `None`。
    pub provider_id: Option<String>,
    /// 最近一次打开时间（Unix 毫秒）。
    pub last_opened_at: Option<i64>,
    /// 体量分级（`small` / `medium` / `large`），用于选择交互策略（M2 的性能开关）。
    pub size_class: Option<String>,
    /// 首次登记时间（Unix 毫秒）。
    pub created_at: i64,
}

/// 新增或更新时提供的字段（不含 `id` 与 `created_at`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryUpsert {
    /// 仓库路径（唯一键）。
    pub path: String,
    /// 展示名。
    pub name: String,
    /// 默认分支。
    pub default_branch: Option<String>,
    /// 托管平台标识。
    pub provider_id: Option<String>,
    /// 体量分级。
    pub size_class: Option<String>,
}

/// 仓库登记仓储。
#[derive(Debug)]
pub struct RepositoryStore<'a> {
    database: &'a Database,
}

impl<'a> RepositoryStore<'a> {
    /// 绑定到某个数据库。
    pub const fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// 新增或更新（按 `path` 唯一）。
    ///
    /// 为什么用 upsert 而不是"先查再插"：打开同一个仓库是最高频的操作，
    /// upsert 让它只产生一次写事务；同时避免"两个窗口同时打开同一仓库"时的竞态。
    /// `created_at` 在冲突时保持不变（首次登记时间不该被覆盖）。
    pub fn upsert(&self, input: &RepositoryUpsert, now_millis: i64) -> AppResult<i64> {
        if input.path.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "repository path is empty",
            ));
        }
        if input.name.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "repository name is empty",
            ));
        }

        self.database.with_write(|transaction| {
            transaction
                .execute(
                    "INSERT INTO repositories (path, name, default_branch, provider_id, size_class, created_at, last_opened_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                     ON CONFLICT(path) DO UPDATE SET
                        name = excluded.name,
                        default_branch = COALESCE(excluded.default_branch, repositories.default_branch),
                        provider_id = COALESCE(excluded.provider_id, repositories.provider_id),
                        size_class = COALESCE(excluded.size_class, repositories.size_class),
                        last_opened_at = excluded.last_opened_at",
                    params![
                        input.path,
                        input.name,
                        input.default_branch,
                        input.provider_id,
                        input.size_class,
                        now_millis
                    ],
                )
                .map_err(|error| storage_error("保存仓库记录失败", &error))?;

            transaction
                .query_row(
                    "SELECT id FROM repositories WHERE path = ?1",
                    params![input.path],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| storage_error("读取仓库 id 失败", &error))
        })
    }

    /// 按路径查找。
    pub fn find_by_path(&self, path: &str) -> AppResult<Option<RepositoryRecord>> {
        self.database.with_read(|connection| {
            connection
                .query_row(
                    "SELECT id, path, name, default_branch, provider_id, last_opened_at, size_class, created_at
                     FROM repositories WHERE path = ?1",
                    params![path],
                    map_record,
                )
                .optional()
                .map_err(|error| storage_error("查询仓库失败", &error))
        })
    }

    /// 按 id 查找。
    pub fn find_by_id(&self, id: i64) -> AppResult<Option<RepositoryRecord>> {
        self.database.with_read(|connection| {
            connection
                .query_row(
                    "SELECT id, path, name, default_branch, provider_id, last_opened_at, size_class, created_at
                     FROM repositories WHERE id = ?1",
                    params![id],
                    map_record,
                )
                .optional()
                .map_err(|error| storage_error("查询仓库失败", &error))
        })
    }

    /// 最近打开的仓库列表（供仪表盘与仓库切换器使用）。
    pub fn recent(&self, limit: usize) -> AppResult<Vec<RepositoryRecord>> {
        let limit = i64::try_from(limit.min(200)).unwrap_or(50);
        self.database.with_read(|connection| {
            let mut statement = connection
                .prepare_cached(
                    "SELECT id, path, name, default_branch, provider_id, last_opened_at, size_class, created_at
                     FROM repositories ORDER BY COALESCE(last_opened_at, created_at) DESC LIMIT ?1",
                )
                .map_err(|error| storage_error("准备查询仓库列表失败", &error))?;

            let rows = statement
                .query_map(params![limit], map_record)
                .map_err(|error| storage_error("查询仓库列表失败", &error))?;

            let mut result = Vec::new();
            for row in rows {
                result.push(row.map_err(|error| storage_error("读取仓库行失败", &error))?);
            }
            Ok(result)
        })
    }

    /// 更新最近打开时间（只改时间戳，不碰其它字段）。
    pub fn touch(&self, id: i64, now_millis: i64) -> AppResult<()> {
        self.database.with_write(|transaction| {
            let affected = transaction
                .execute(
                    "UPDATE repositories SET last_opened_at = ?2 WHERE id = ?1",
                    params![id, now_millis],
                )
                .map_err(|error| storage_error("更新最近打开时间失败", &error))?;
            if affected == 0 {
                return Err(AppError::new(ErrorCode::NotFound, "repository not found")
                    .with_detail(id.to_string()));
            }
            Ok(())
        })
    }

    /// 从列表中移除（**只删记录，不动仓库文件**）。
    pub fn forget(&self, id: i64) -> AppResult<()> {
        self.database.with_write(|transaction| {
            transaction
                .execute("DELETE FROM repositories WHERE id = ?1", params![id])
                .map_err(|error| storage_error("移除仓库记录失败", &error))?;
            Ok(())
        })
    }
}

fn map_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RepositoryRecord> {
    Ok(RepositoryRecord {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        default_branch: row.get(3)?,
        provider_id: row.get(4)?,
        last_opened_at: row.get(5)?,
        size_class: row.get(6)?,
        created_at: row.get(7)?,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::ErrorCode;

    use super::{RepositoryStore, RepositoryUpsert};
    use crate::database::Database;
    use crate::migrations::migrate;

    fn store() -> Database {
        let database = Database::open_in_memory().expect("打开内存库失败");
        migrate(&database).expect("迁移失败");
        database
    }

    fn sample(path: &str) -> RepositoryUpsert {
        RepositoryUpsert {
            path: path.to_owned(),
            name: "forgedesk".to_owned(),
            default_branch: Some("main".to_owned()),
            provider_id: None,
            size_class: None,
        }
    }

    #[test]
    fn upsert_inserts_then_updates_without_losing_created_at() {
        let database = store();
        let repositories = RepositoryStore::new(&database);

        let id = repositories
            .upsert(&sample("E:/projects/a"), 1_000)
            .unwrap();
        let mut again = sample("E:/projects/a");
        again.name = "renamed".to_owned();
        again.default_branch = None; // 不提供时不应清空已有值
        let same_id = repositories.upsert(&again, 2_000).unwrap();

        assert_eq!(id, same_id, "同一路径必须复用同一条记录");

        let record = repositories.find_by_path("E:/projects/a").unwrap().unwrap();
        assert_eq!(record.name, "renamed");
        assert_eq!(
            record.default_branch.as_deref(),
            Some("main"),
            "缺省字段不应覆盖已有值"
        );
        assert_eq!(record.created_at, 1_000, "首次登记时间不应被覆盖");
        assert_eq!(record.last_opened_at, Some(2_000));
    }

    #[test]
    fn rejects_empty_path_or_name() {
        let database = store();
        let repositories = RepositoryStore::new(&database);

        let error = repositories.upsert(&sample("   "), 0).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);

        let mut empty_name = sample("E:/projects/a");
        empty_name.name = String::new();
        let error = repositories.upsert(&empty_name, 0).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[test]
    fn recent_orders_by_last_opened() {
        let database = store();
        let repositories = RepositoryStore::new(&database);

        repositories
            .upsert(&sample("E:/projects/old"), 1_000)
            .unwrap();
        repositories
            .upsert(&sample("E:/projects/new"), 3_000)
            .unwrap();
        repositories
            .upsert(&sample("E:/projects/middle"), 2_000)
            .unwrap();

        let recent = repositories.recent(10).unwrap();
        let paths: Vec<&str> = recent.iter().map(|record| record.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["E:/projects/new", "E:/projects/middle", "E:/projects/old"]
        );
    }

    #[test]
    fn touch_updates_timestamp_and_reports_missing_id() {
        let database = store();
        let repositories = RepositoryStore::new(&database);

        let id = repositories
            .upsert(&sample("E:/projects/a"), 1_000)
            .unwrap();
        repositories.touch(id, 9_000).unwrap();
        assert_eq!(
            repositories.find_by_id(id).unwrap().unwrap().last_opened_at,
            Some(9_000)
        );

        let error = repositories.touch(9_999, 0).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn forget_removes_only_the_record() {
        let database = store();
        let repositories = RepositoryStore::new(&database);

        let id = repositories
            .upsert(&sample("E:/projects/a"), 1_000)
            .unwrap();
        repositories.forget(id).unwrap();

        assert!(repositories.find_by_id(id).unwrap().is_none());
        assert!(repositories.recent(10).unwrap().is_empty());
    }
}
