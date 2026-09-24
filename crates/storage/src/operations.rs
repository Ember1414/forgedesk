//! 写操作的审计记录（`operation_records` 表）。
//!
//! # 为什么每一次写操作都要落一条记录
//!
//! 红线 R7 要的是"可回滚"，而"可回滚"的前提是**知道刚才发生了什么**：改了哪个仓库、
//! 跑了什么、退出码多少、有没有快照可回。这些信息在出问题之前都没人看，
//! 因此必须在写的时候无条件记下来——事后补记是不可能的。
//!
//! # 两条写入约定
//!
//! 1. **先 begin 再执行，失败也要 finish**：只有一条"开始"记录而没有"结束"记录，
//!    正是"应用崩在写操作中间"的可观测特征，比什么都不留下有用得多。
//! 2. **落库前必须脱敏**（红线 R8）：`args_json` 与 `stderr_summary` 都可能夹带凭据
//!    （远端 URL 里的 token、hook 打印的环境变量）。本层不做脱敏——它只负责存，
//!    脱敏由调用方在写之前用 `forgedesk_diagnostics::sanitize_log` 做掉。

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use rusqlite::params;

use crate::database::{storage_error, Database};

/// 开始一次写操作所需的信息。
#[derive(Debug, Clone, Copy)]
pub struct NewOperation<'a> {
    /// 存储层记录 id（`repositories.id`）。
    pub repo_id: i64,
    /// 操作类型（稳定短名，如 `commit` / `stage` / `discard`）。
    pub op_type: &'a str,
    /// 参数摘要（**已脱敏**的 JSON；无参数时为 `None`）。
    pub args_json: Option<&'a str>,
    /// 开始时间（Unix 毫秒）。
    pub started_at_ms: i64,
}

/// 结束一次写操作所需的信息。
#[derive(Debug, Clone, Copy)]
pub struct OperationOutcome<'a> {
    /// 结束时间（Unix 毫秒）。
    pub ended_at_ms: i64,
    /// git 的退出码；未执行到 git（例如被计划校验拦下）时为 `None`。
    pub exit_code: Option<i32>,
    /// 失败摘要（**已脱敏**；成功时为 `None`）。
    pub stderr_summary: Option<&'a str>,
    /// 本次操作关联的快照 id；没有快照（M3 之前）时为 `None`。
    pub snapshot_id: Option<i64>,
    /// 这次操作是否可回滚。
    pub reversible: bool,
}

/// 一条写操作记录（读出来的形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRecord {
    /// 主键。
    pub id: i64,
    /// 存储层记录 id。
    pub repo_id: i64,
    /// 操作类型。
    pub op_type: String,
    /// 参数摘要（已脱敏）。
    pub args_json: Option<String>,
    /// 开始时间（Unix 毫秒）。
    pub started_at_ms: Option<i64>,
    /// 结束时间（Unix 毫秒）。`None` 表示"这条记录没有正常收尾"。
    pub ended_at_ms: Option<i64>,
    /// git 的退出码。
    pub exit_code: Option<i32>,
    /// 失败摘要（已脱敏）。
    pub stderr_summary: Option<String>,
    /// 关联的快照 id。
    pub snapshot_id: Option<i64>,
    /// 是否可回滚。
    pub reversible: bool,
}

/// 操作记录仓储。
#[derive(Debug)]
pub struct OperationStore<'a> {
    database: &'a Database,
}

impl<'a> OperationStore<'a> {
    /// 绑定到某个数据库。
    pub const fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// 记录一次操作的开始，返回记录 id（收尾时要用）。
    pub fn begin(&self, operation: &NewOperation<'_>) -> AppResult<i64> {
        if operation.op_type.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "operation type is empty",
            ));
        }

        self.database.with_write(|transaction| {
            transaction
                .execute(
                    "INSERT INTO operation_records (repo_id, op_type, args_json, started_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        operation.repo_id,
                        operation.op_type,
                        operation.args_json,
                        operation.started_at_ms,
                    ],
                )
                .map_err(|error| storage_error("写入操作记录失败", &error))?;
            Ok(transaction.last_insert_rowid())
        })
    }

    /// 收尾一次操作。
    ///
    /// 记录不存在时返回 `NOT_FOUND` 而不是静默成功：调用方拿到的是一个 id，
    /// 它丢了就意味着"有一次写操作没有任何收尾痕迹"，这必须响亮地暴露出来。
    pub fn finish(&self, id: i64, outcome: &OperationOutcome<'_>) -> AppResult<()> {
        self.database.with_write(|transaction| {
            let changed = transaction
                .execute(
                    "UPDATE operation_records
                        SET ended_at = ?2, exit_code = ?3, stderr_summary = ?4,
                            snapshot_id = ?5, reversible = ?6
                      WHERE id = ?1",
                    params![
                        id,
                        outcome.ended_at_ms,
                        outcome.exit_code,
                        outcome.stderr_summary,
                        outcome.snapshot_id,
                        i64::from(outcome.reversible),
                    ],
                )
                .map_err(|error| storage_error("更新操作记录失败", &error))?;

            if changed == 0 {
                return Err(AppError::new(
                    ErrorCode::NotFound,
                    "the operation record does not exist",
                )
                .with_detail(format!("id: {id}")));
            }
            Ok(())
        })
    }

    /// 最近的操作记录（新 → 旧）。
    ///
    /// `limit` 被夹在 `1..=200`：审计表的读取只服务于排查与将来的"回放"，
    /// 一次拉走整张表没有意义。
    pub fn recent(&self, repo_id: i64, limit: usize) -> AppResult<Vec<OperationRecord>> {
        let limit = limit.clamp(1, 200);

        self.database.with_read(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, repo_id, op_type, args_json, started_at, ended_at,
                            exit_code, stderr_summary, snapshot_id, reversible
                       FROM operation_records
                      WHERE repo_id = ?1
                      ORDER BY started_at DESC, id DESC
                      LIMIT ?2",
                )
                .map_err(|error| storage_error("准备操作记录查询失败", &error))?;

            let rows = statement
                .query_map(params![repo_id, limit as i64], |row| {
                    Ok(OperationRecord {
                        id: row.get(0)?,
                        repo_id: row.get(1)?,
                        op_type: row.get(2)?,
                        args_json: row.get(3)?,
                        started_at_ms: row.get(4)?,
                        ended_at_ms: row.get(5)?,
                        exit_code: row.get(6)?,
                        stderr_summary: row.get(7)?,
                        snapshot_id: row.get(8)?,
                        reversible: row.get::<_, i64>(9)? != 0,
                    })
                })
                .map_err(|error| storage_error("查询操作记录失败", &error))?;

            let mut records = Vec::new();
            for row in rows {
                records.push(row.map_err(|error| storage_error("读取操作记录失败", &error))?);
            }
            Ok(records)
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{NewOperation, OperationOutcome, OperationStore};
    use crate::migrations::migrate;
    use crate::Database;
    use forgedesk_domain::ErrorCode;

    fn store_of(database: &Database) -> OperationStore<'_> {
        migrate(database).unwrap();
        OperationStore::new(database)
    }

    fn begin(store: &OperationStore<'_>, op_type: &str, repo_id: i64) -> i64 {
        store
            .begin(&NewOperation {
                repo_id,
                op_type,
                args_json: Some(r#"{"amend":false}"#),
                started_at_ms: 1_000,
            })
            .unwrap()
    }

    #[test]
    fn an_operation_is_recorded_and_can_be_closed() {
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);

        let id = begin(&store, "commit", 1);
        store
            .finish(
                id,
                &OperationOutcome {
                    ended_at_ms: 2_000,
                    exit_code: Some(0),
                    stderr_summary: None,
                    snapshot_id: None,
                    reversible: true,
                },
            )
            .unwrap();

        let records = store.recent(1, 10).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].op_type, "commit");
        assert_eq!(records[0].args_json.as_deref(), Some(r#"{"amend":false}"#));
        assert_eq!(records[0].started_at_ms, Some(1_000));
        assert_eq!(records[0].ended_at_ms, Some(2_000));
        assert_eq!(records[0].exit_code, Some(0));
        assert!(records[0].reversible);
    }

    #[test]
    fn a_record_that_was_never_closed_is_visible_as_such() {
        // "只有开始没有结束"正是"应用崩在写操作中间"的可观测特征，
        // 界面与排查都依赖这一点，所以这里把它钉住
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);

        let id = begin(&store, "commit", 1);
        let record = store.recent(1, 10).unwrap().remove(0);

        assert_eq!(record.id, id);
        assert_eq!(record.ended_at_ms, None);
        assert_eq!(record.exit_code, None);
    }

    #[test]
    fn a_missing_snapshot_is_null_rather_than_a_zero_id() {
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);

        let id = begin(&store, "commit", 1);
        store
            .finish(
                id,
                &OperationOutcome {
                    ended_at_ms: 2_000,
                    exit_code: Some(0),
                    stderr_summary: None,
                    snapshot_id: None,
                    reversible: true,
                },
            )
            .unwrap();

        // 0 会被读成"快照 id 0"，而 NULL 才是"没有快照"
        assert_eq!(store.recent(1, 1).unwrap()[0].snapshot_id, None);
    }

    #[test]
    fn a_failed_operation_keeps_the_exit_code_and_the_summary() {
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);

        let id = begin(&store, "commit", 1);
        store
            .finish(
                id,
                &OperationOutcome {
                    ended_at_ms: 2_000,
                    exit_code: Some(1),
                    stderr_summary: Some("pre-commit hook exited with code 1"),
                    snapshot_id: Some(7),
                    reversible: false,
                },
            )
            .unwrap();

        let record = store.recent(1, 1).unwrap().remove(0);
        assert_eq!(record.exit_code, Some(1));
        assert_eq!(
            record.stderr_summary.as_deref(),
            Some("pre-commit hook exited with code 1")
        );
        assert_eq!(record.snapshot_id, Some(7));
        assert!(!record.reversible);
    }

    #[test]
    fn an_empty_operation_type_is_rejected() {
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);

        let error = store
            .begin(&NewOperation {
                repo_id: 1,
                op_type: "  ",
                args_json: None,
                started_at_ms: 1_000,
            })
            .expect_err("空操作类型必须被拒绝");

        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[test]
    fn closing_an_unknown_record_reports_not_found() {
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);

        let error = store
            .finish(
                4242,
                &OperationOutcome {
                    ended_at_ms: 2_000,
                    exit_code: Some(0),
                    stderr_summary: None,
                    snapshot_id: None,
                    reversible: true,
                },
            )
            .expect_err("不存在的记录必须报错，不能静默成功");

        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn records_are_scoped_to_one_repository_and_limited() {
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);

        begin(&store, "commit", 1);
        begin(&store, "stage", 1);
        begin(&store, "commit", 2);

        let first = store.recent(1, 10).unwrap();
        assert_eq!(first.len(), 2, "只应看到自己仓库的记录");
        assert_eq!(store.recent(1, 1).unwrap().len(), 1);
        assert_eq!(store.recent(2, 10).unwrap().len(), 1);
    }
}
