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
use rusqlite::params_from_iter;
use rusqlite::types::Value;

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

/// 审计查询的过滤条件。
///
/// 全部字段都是"可选收窄"：界面上的筛选是逐项加的，任何一项都不该让
/// 另外几项失效（用 `Option` 而不是"空字符串表示不过滤"，后者在
/// `op_type = ''` 这种边界上必然出错）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationQuery {
    /// 只查某个仓库；`None` = 全部仓库。
    pub repo_id: Option<i64>,
    /// 只查某类操作（稳定短名）。
    pub op_type: Option<String>,
    /// 起始时间（含，Unix 毫秒）。
    pub from_ms: Option<i64>,
    /// 结束时间（含，Unix 毫秒）。
    pub to_ms: Option<i64>,
    /// 跳过条数（分页）。
    pub offset: usize,
    /// 返回条数上限（夹在 `1..=500`）。
    pub limit: usize,
}

impl OperationQuery {
    /// 分页上限：一次最多取一页。
    ///
    /// 为什么不给"拉全部"：审计表按保留策略可以有一万条，而界面上一次能看的
    /// 就是一屏。真要全量，导出走 [`OperationStore::query_all`] 那条路。
    pub const MAX_LIMIT: usize = 500;

    /// 缺省页大小。
    pub const DEFAULT_LIMIT: usize = 100;
}

impl Default for OperationQuery {
    fn default() -> Self {
        // 手写而不是 derive：`limit` 的零值在 SQL 里是"一条都不要"，
        // 而 `..Default::default()` 是调用方最常用的写法——它必须给出一个
        // 真的会返回记录的页大小，否则"我明明查了却什么都没有"。
        Self {
            repo_id: None,
            op_type: None,
            from_ms: None,
            to_ms: None,
            offset: 0,
            limit: Self::DEFAULT_LIMIT,
        }
    }
}

/// 一页审计记录（含总数，界面据此算分页）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationPage {
    /// 满足条件的总条数（不受 `offset` / `limit` 影响）。
    pub total: i64,
    /// 当前页的记录（新 → 旧）。
    pub records: Vec<OperationRecord>,
}

/// 保留策略：默认保留 90 天或 10000 条（可配置）。
///
/// 两个条款是**或**的关系：超龄的删掉，超量的也删掉。只按时间删会让
/// "一天跑了几万次暂存"把库撑爆；只按条数删会让半年前的记录一直躺着。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// 当前时间（Unix 毫秒）——由调用方传入，便于测试与"一次操作内时间一致"。
    pub now_ms: i64,
    /// 保留天数（`<= 0` 表示不按时间清理）。
    pub keep_days: i64,
    /// 保留条数上限（`<= 0` 表示不按条数清理）。
    pub keep_rows: i64,
}

impl RetentionPolicy {
    /// 默认策略下的截止时间（`now_ms` 之前 `keep_days` 天）。
    pub const fn cutoff_ms(&self) -> i64 {
        self.now_ms - self.keep_days * 24 * 60 * 60 * 1_000
    }
}

/// 操作记录仓储。
///
/// `Copy` 是刻意的：它就是一个数据库引用，"借出一个运行记录"（见
/// `services::audit::AuditRun`）时不必把仓储的所有权或生命周期绕来绕去。
#[derive(Debug, Clone, Copy)]
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

    /// 按条件查询一页记录（新 → 旧），并给出满足条件的总数。
    ///
    /// 总数与页面在**同一次读事务**里取：分两次查会让界面上的"共 N 条"
    /// 与列表在并发写入时对不上（而审计表恰恰总是在被写）。
    pub fn query(&self, query: &OperationQuery) -> AppResult<OperationPage> {
        let limit = query.limit.clamp(1, OperationQuery::MAX_LIMIT);
        let (where_sql, values) = filter_sql(query);

        self.database.with_read(|connection| {
            let total: i64 = connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM operation_records{where_sql}"),
                    params_from_iter(values.iter()),
                    |row| row.get(0),
                )
                .map_err(|error| storage_error("统计操作记录失败", &error))?;

            let mut page_values = values.clone();
            page_values.push(Value::Integer(limit as i64));
            page_values.push(Value::Integer(query.offset as i64));

            let sql = format!(
                "SELECT {COLUMNS} FROM operation_records{where_sql}
                  ORDER BY started_at DESC, id DESC
                  LIMIT ?{} OFFSET ?{}",
                page_values.len() - 1,
                page_values.len()
            );
            let mut statement = connection
                .prepare(&sql)
                .map_err(|error| storage_error("准备操作记录查询失败", &error))?;
            let rows = statement
                .query_map(params_from_iter(page_values.iter()), record_from_row)
                .map_err(|error| storage_error("查询操作记录失败", &error))?;

            let mut records = Vec::new();
            for row in rows {
                records.push(row.map_err(|error| storage_error("读取操作记录失败", &error))?);
            }
            Ok(OperationPage { total, records })
        })
    }

    /// 按条件取出**全部**匹配记录（新 → 旧）。
    ///
    /// 只给导出用：审计导出必须是"筛选出来的全部"，而不是界面上那一页；
    /// 但调用方要自己保证条件足够窄（导出接口会夹一个上限，见 `services::audit`）。
    pub fn query_all(&self, query: &OperationQuery) -> AppResult<Vec<OperationRecord>> {
        let (where_sql, values) = filter_sql(query);

        self.database.with_read(|connection| {
            let sql = format!(
                "SELECT {COLUMNS} FROM operation_records{where_sql}
                  ORDER BY started_at DESC, id DESC"
            );
            let mut statement = connection
                .prepare(&sql)
                .map_err(|error| storage_error("准备操作记录查询失败", &error))?;
            let rows = statement
                .query_map(params_from_iter(values.iter()), record_from_row)
                .map_err(|error| storage_error("查询操作记录失败", &error))?;

            let mut records = Vec::new();
            for row in rows {
                records.push(row.map_err(|error| storage_error("读取操作记录失败", &error))?);
            }
            Ok(records)
        })
    }

    /// 按保留策略清理旧记录，返回删除条数。
    ///
    /// 两个条款是"或"：超龄的删、超量的也删。`keep_days` / `keep_rows` 传 `<= 0`
    /// 表示**关掉该条款**——把"不清理"表达成"保留 0 天"会让一次误配置清空整张表。
    pub fn prune(&self, policy: &RetentionPolicy) -> AppResult<usize> {
        let mut conditions: Vec<String> = Vec::new();
        let mut values: Vec<Value> = Vec::new();

        if policy.keep_days > 0 {
            values.push(Value::Integer(policy.cutoff_ms()));
            conditions.push(format!(
                "(started_at IS NOT NULL AND started_at < ?{})",
                values.len()
            ));
        }
        if policy.keep_rows > 0 {
            values.push(Value::Integer(policy.keep_rows));
            conditions.push(format!(
                "id NOT IN (SELECT id FROM operation_records
                             ORDER BY started_at DESC, id DESC LIMIT ?{})",
                values.len()
            ));
        }
        if conditions.is_empty() {
            return Ok(0);
        }

        let sql = format!(
            "DELETE FROM operation_records WHERE {}",
            conditions.join(" OR ")
        );

        self.database.with_write(|transaction| {
            transaction
                .execute(&sql, params_from_iter(values.iter()))
                .map_err(|error| storage_error("清理操作记录失败", &error))
        })
    }
}

/// 行 → 记录。列顺序必须与 [`COLUMNS`] 一致：两者写在一处，改一处就够。
fn record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OperationRecord> {
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
}

/// 查询列清单（与 [`record_from_row`] 的下标一一对应）。
const COLUMNS: &str = "id, repo_id, op_type, args_json, started_at, ended_at,
                       exit_code, stderr_summary, snapshot_id, reversible";

/// 把过滤条件编译成 `WHERE` 子句与参数（没有任何条件时返回空子句）。
///
/// 用位置参数动态拼：条数取决于调用方给了几项，而 SQLite 支持 `?N` 形式的
/// 显式编号，因此不必担心中间少了某个参数导致错位。
fn filter_sql(query: &OperationQuery) -> (String, Vec<Value>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    if let Some(repo_id) = query.repo_id {
        values.push(Value::Integer(repo_id));
        conditions.push(format!("repo_id = ?{}", values.len()));
    }
    if let Some(op_type) = &query.op_type {
        values.push(Value::Text(op_type.clone()));
        conditions.push(format!("op_type = ?{}", values.len()));
    }
    if let Some(from_ms) = query.from_ms {
        values.push(Value::Integer(from_ms));
        conditions.push(format!("started_at >= ?{}", values.len()));
    }
    if let Some(to_ms) = query.to_ms {
        values.push(Value::Integer(to_ms));
        conditions.push(format!("started_at <= ?{}", values.len()));
    }

    if conditions.is_empty() {
        return (String::new(), values);
    }
    (format!(" WHERE {}", conditions.join(" AND ")), values)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{NewOperation, OperationOutcome, OperationQuery, OperationStore, RetentionPolicy};
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
    fn the_query_filters_by_repository_type_and_time_and_reports_the_total() {
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);

        // 三条记录：仓库 1 的两条（不同时刻）+ 仓库 2 的一条
        store
            .begin(&NewOperation {
                repo_id: 1,
                op_type: "commit",
                args_json: None,
                started_at_ms: 1_000,
            })
            .unwrap();
        store
            .begin(&NewOperation {
                repo_id: 1,
                op_type: "stage",
                args_json: None,
                started_at_ms: 5_000,
            })
            .unwrap();
        store
            .begin(&NewOperation {
                repo_id: 2,
                op_type: "commit",
                args_json: None,
                started_at_ms: 9_000,
            })
            .unwrap();

        // 不带条件：全部三条，总数与页面一起给出
        let all = store.query(&OperationQuery::default()).unwrap();
        assert_eq!(all.total, 3);
        assert_eq!(all.records.len(), 3);

        // 按仓库
        let repo_one = store
            .query(&OperationQuery {
                repo_id: Some(1),
                ..OperationQuery::default()
            })
            .unwrap();
        assert_eq!(repo_one.total, 2);

        // 按类型（跨仓库）
        let commits = store
            .query(&OperationQuery {
                op_type: Some("commit".to_owned()),
                ..OperationQuery::default()
            })
            .unwrap();
        assert_eq!(commits.total, 2);
        assert!(commits.records.iter().all(|row| row.op_type == "commit"));

        // 按时间区间：`from` 与 `to` 都含边界
        let window = store
            .query(&OperationQuery {
                from_ms: Some(5_000),
                to_ms: Some(9_000),
                ..OperationQuery::default()
            })
            .unwrap();
        assert_eq!(window.total, 2);
        assert_eq!(window.records[0].started_at_ms, Some(9_000), "新 → 旧");

        // 总数是"满足条件的总数"，不受分页影响
        let page = store
            .query(&OperationQuery {
                limit: 1,
                offset: 1,
                ..OperationQuery::default()
            })
            .unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].started_at_ms, Some(5_000), "第二新的那条");
    }

    #[test]
    fn the_export_query_returns_everything_matching() {
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);

        for index in 0..5 {
            store
                .begin(&NewOperation {
                    repo_id: 1,
                    op_type: if index % 2 == 0 { "commit" } else { "stage" },
                    args_json: None,
                    started_at_ms: 1_000 + index,
                })
                .unwrap();
        }

        // 导出走的是"不限一页"的路径：只要筛选条件命中，就必须全部拿到
        let all = store
            .query_all(&OperationQuery {
                op_type: Some("stage".to_owned()),
                ..OperationQuery::default()
            })
            .unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn pruning_removes_aged_records_and_keeps_recent_ones() {
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);

        let day = 24 * 60 * 60 * 1_000_i64;
        let now = 100 * day;
        for days_ago in [95_i64, 91, 89, 1] {
            store
                .begin(&NewOperation {
                    repo_id: 1,
                    op_type: "commit",
                    args_json: None,
                    started_at_ms: now - days_ago * day,
                })
                .unwrap();
        }

        // 90 天以内保留：删掉 95 天与 91 天前的那两条
        let removed = store
            .prune(&RetentionPolicy {
                now_ms: now,
                keep_days: 90,
                keep_rows: 0,
            })
            .unwrap();
        assert_eq!(removed, 2);

        let left = store.query(&OperationQuery::default()).unwrap();
        assert_eq!(left.total, 2, "90 天内的记录必须留着");
    }

    #[test]
    fn pruning_by_row_count_keeps_the_newest() {
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);

        for index in 0..5_i64 {
            store
                .begin(&NewOperation {
                    repo_id: 1,
                    op_type: "stage",
                    args_json: None,
                    started_at_ms: 1_000 + index,
                })
                .unwrap();
        }

        let removed = store
            .prune(&RetentionPolicy {
                now_ms: 999_999,
                keep_days: 0,
                keep_rows: 2,
            })
            .unwrap();
        assert_eq!(removed, 3);

        let left = store.query(&OperationQuery::default()).unwrap();
        assert_eq!(left.total, 2);
        assert_eq!(
            left.records[0].started_at_ms,
            Some(1_004),
            "留下的是最新的两条"
        );
    }

    #[test]
    fn pruning_with_both_clauses_disabled_removes_nothing() {
        let database = Database::open_in_memory().unwrap();
        let store = store_of(&database);
        begin(&store, "stage", 1);

        // "不清理"必须是显式语义，而不是"保留 0 天"（那会清空整张表）
        let removed = store
            .prune(&RetentionPolicy {
                now_ms: 999_999,
                keep_days: 0,
                keep_rows: 0,
            })
            .unwrap();
        assert_eq!(removed, 0);
        assert_eq!(store.query(&OperationQuery::default()).unwrap().total, 1);
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
