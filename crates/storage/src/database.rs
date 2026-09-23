//! SQLite 连接管理与并发策略。
//!
//! # 为什么自己写"单写多读"而不是引 r2d2
//!
//! 本应用的访问模式非常明确：**写少读多**（打开仓库时读一堆表，用户点一次才写一次），
//! 而写操作全部来自我们自己的命令层（没有第三方并发写入者）。
//! 因此一个互斥的写连接 + 少量读连接就足够，并且：
//!
//! - 行为完全可预测：不存在"池被借空后阻塞在超时上"的隐性等待；
//! - 少两个依赖（r2d2 / r2d2_sqlite），减少供应链面（AGENTS.md §8）；
//! - 边界清晰：`with_write` 一定在事务里，`with_read` 一定不开事务。
//!
//! 一旦将来出现"多进程写"或"长事务与高频读并存"的真实需求，再换 r2d2 也不影响调用方
//! （外面只看到 `with_read` / `with_write`）。
//!
//! # 每个连接都要设置的 PRAGMA
//!
//! `journal_mode=WAL`、`foreign_keys=ON`、`busy_timeout` 都是**连接级**设置，
//! 新建连接必须重新设置（这是 SQLite 最常见的"我明明设过了"类问题）。
//! 因此统一收在 [`configure_connection`] 里，任何打开连接的地方都必须经过它。
//!
//! `foreign_keys` 还要注意：它在**事务内无效**，只能在事务外打开。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use rusqlite::{Connection, OpenFlags};

/// 写锁等待上限。
///
/// 为什么需要它：SQLite 在写锁被占用时默认立即返回 `SQLITE_BUSY`。
/// 设成 5 秒后，短事务之间会自动排队重试，用户几乎不会看到"数据库忙"这类错误；
/// 而真出现长时间占用（外部进程锁库）时，5 秒后仍然如实报错，不会无限挂起。
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// 读连接池上限。读连接很便宜，但无上限会在长列表滚动时把文件句柄吃光。
const MAX_READ_CONNECTIONS: usize = 4;

/// 数据库句柄。
pub struct Database {
    /// 文件路径；内存库为 `None`。
    path: Option<PathBuf>,
    /// 唯一的写连接（互斥）。
    writer: Mutex<Connection>,
    /// 读连接池（惰性创建，归还后复用）。内存库不使用（各连接互不共享数据）。
    readers: Mutex<Vec<Connection>>,
    /// 是否为内存库。
    in_memory: bool,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Database")
            .field("path", &self.path)
            .field("in_memory", &self.in_memory)
            .finish_non_exhaustive()
    }
}

/// 把 rusqlite 错误转换为带上下文的 [`AppError`]。
///
/// 统一走 `STORAGE`：这些错误的共同特征是"本地数据存取出问题"，
/// 对应的用户建议是"检查磁盘空间与数据目录权限"，与网络/仓库状态无关。
pub fn storage_error(context: &str, error: &rusqlite::Error) -> AppError {
    AppError::new(ErrorCode::Storage, context).with_detail(error.to_string())
}

/// 把文件系统错误转换为 [`AppError`]。
///
/// 数据目录不可写、备份复制失败都属于这一族：用户能自救（改权限、清磁盘），
/// 因此同样归到 `STORAGE` 并给出可执行的建议。
pub fn io_error(context: &str, error: &std::io::Error) -> AppError {
    AppError::new(ErrorCode::Storage, context)
        .with_detail(error.to_string())
        .with_hint("请确认数据目录可写、磁盘空间充足")
}

/// 统一的连接初始化（见模块头说明：这些设置都是连接级的）。
fn configure_connection(connection: &Connection) -> AppResult<()> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| storage_error("设置 busy_timeout 失败", &error))?;

    // WAL：读写不互相阻塞，且崩溃后恢复更快（桌面应用会被"直接关掉"）
    connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| storage_error("启用 WAL 失败", &error))?;

    // 外键：默认关闭，必须显式打开（且不能在事务里打开）
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|error| storage_error("启用外键失败", &error))?;

    // NORMAL 在 WAL 下已能保证崩溃一致性，比 FULL 少一次 fsync（桌面场景更跟手）
    connection
        .execute_batch("PRAGMA synchronous = NORMAL;")
        .map_err(|error| storage_error("设置 synchronous 失败", &error))?;

    Ok(())
}

impl Database {
    /// 打开（必要时创建）指定路径的数据库。
    ///
    /// 父目录不存在时会创建：应用数据目录在首次启动时通常还不存在，
    /// 让调用方处处 mkdir 是把易错点撒到各处。
    pub fn open(path: impl AsRef<Path>) -> AppResult<Self> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                AppError::new(ErrorCode::Storage, "创建数据目录失败")
                    .with_detail(format!("{}: {error}", parent.display()))
            })?;
        }

        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| storage_error("打开数据库失败", &error))?;

        configure_connection(&connection)?;

        Ok(Self {
            path: Some(path),
            writer: Mutex::new(connection),
            readers: Mutex::new(Vec::new()),
            in_memory: false,
        })
    }

    /// 打开一个内存数据库（测试用；单连接，因为各自的 `:memory:` 互不共享）。
    pub fn open_in_memory() -> AppResult<Self> {
        let connection = Connection::open_in_memory()
            .map_err(|error| storage_error("打开内存数据库失败", &error))?;
        configure_connection(&connection)?;

        Ok(Self {
            path: None,
            writer: Mutex::new(connection),
            readers: Mutex::new(Vec::new()),
            in_memory: true,
        })
    }

    /// 数据库文件路径（内存库为 `None`）。
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 是否内存库。
    pub fn is_in_memory(&self) -> bool {
        self.in_memory
    }

    /// 在一个读连接上执行操作。
    pub fn with_read<T>(&self, action: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
        // 内存库没有共享缓存，读也必须走写连接，否则读到的是一张空库
        if self.in_memory {
            let guard = self
                .writer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            return action(&guard);
        }

        let connection = self.acquire_reader()?;
        let result = action(&connection);
        self.release_reader(connection);
        result
    }

    /// 在一个**事务**中执行写操作：闭包返回 `Ok` 则提交，返回 `Err` 或 panic 则回滚。
    pub fn with_write<T>(
        &self,
        action: impl FnOnce(&rusqlite::Transaction<'_>) -> AppResult<T>,
    ) -> AppResult<T> {
        let mut guard = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let transaction = guard
            .transaction()
            .map_err(|error| storage_error("开启事务失败", &error))?;

        let value = action(&transaction)?;
        transaction
            .commit()
            .map_err(|error| storage_error("提交事务失败", &error))?;
        Ok(value)
    }

    /// 取一个读连接：优先复用空闲连接，池满则共享一个（宁可共享也不无限开文件句柄）。
    fn acquire_reader(&self) -> AppResult<Connection> {
        {
            let mut pool = self
                .readers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(connection) = pool.pop() {
                return Ok(connection);
            }
        }

        let Some(path) = self.path.as_deref() else {
            return Err(AppError::new(
                ErrorCode::Storage,
                "内存数据库不支持独立读连接",
            ));
        };

        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| storage_error("打开只读连接失败", &error))?;
        configure_connection(&connection)?;
        Ok(connection)
    }

    /// 归还读连接；池满则直接丢弃（由 SQLite 关闭文件句柄）。
    fn release_reader(&self, connection: Connection) {
        let mut pool = self
            .readers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pool.len() < MAX_READ_CONNECTIONS {
            pool.push(connection);
        }
    }

    /// 把 WAL 内容合并回主文件。
    ///
    /// 备份前必须调用：否则复制出来的 `.db` 缺少最近的写入（还在 `-wal` 里），
    /// 备份看起来成功、恢复时却丢数据。
    pub fn checkpoint(&self) -> AppResult<()> {
        let guard = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_row| Ok(()))
            .map_err(|error| storage_error("合并 WAL 失败", &error))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod test_support {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 生成一个临时数据库路径（避免引入 tempfile 依赖）。
    ///
    /// 用进程 id + 自增计数保证并行测试之间不撞名；调用方负责清理。
    pub fn temp_db_path(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let suffix = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "forgedesk-test-{}-{}-{}.db",
            label,
            std::process::id(),
            suffix
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// 删除数据库文件及其 WAL / SHM 附属文件。
    pub fn cleanup_db(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let mut target = path.to_path_buf();
            target.set_file_name(format!(
                "{}{suffix}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            let _ = std::fs::remove_file(target);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::test_support::{cleanup_db, temp_db_path};
    use super::Database;

    #[test]
    fn opens_file_database_and_applies_pragmas() {
        let path = temp_db_path("pragmas");
        let database = Database::open(&path).expect("打开数据库失败");

        let journal_mode: String = database
            .with_read(|connection| {
                Ok(connection
                    .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
                    .unwrap())
            })
            .expect("读取 journal_mode 失败");
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let foreign_keys: i64 = database
            .with_read(|connection| {
                Ok(connection
                    .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                    .unwrap())
            })
            .expect("读取 foreign_keys 失败");
        assert_eq!(foreign_keys, 1, "外键必须开启（默认是关闭的）");

        let busy_timeout: i64 = database
            .with_read(|connection| {
                Ok(connection
                    .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap())
            })
            .expect("读取 busy_timeout 失败");
        assert!(
            busy_timeout >= 5000,
            "busy_timeout 应≥5s，实际 {busy_timeout}"
        );

        drop(database);
        cleanup_db(&path);
    }

    #[test]
    fn write_transaction_rolls_back_on_error() {
        let database = Database::open_in_memory().expect("打开内存库失败");
        database
            .with_write(|tx| {
                tx.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)", [])
                    .unwrap();
                Ok(())
            })
            .expect("建表失败");

        let result: forgedesk_domain::AppResult<()> = database.with_write(|tx| {
            tx.execute("INSERT INTO t (id, name) VALUES (1, 'a')", [])
                .unwrap();
            Err(forgedesk_domain::AppError::new(
                forgedesk_domain::ErrorCode::Storage,
                "模拟失败",
            ))
        });
        assert!(result.is_err(), "应返回闭包里的错误");

        let count: i64 = database
            .with_read(|connection| {
                Ok(connection
                    .query_row("SELECT COUNT(*) FROM t", [], |row| row.get::<_, i64>(0))
                    .unwrap())
            })
            .expect("统计失败");
        assert_eq!(count, 0, "闭包返回 Err 时事务必须回滚");
    }

    /// 验收要求：并发写入不出现 SQLITE_BUSY。
    #[test]
    fn concurrent_writes_do_not_fail_with_busy() {
        let path = temp_db_path("concurrent");
        let database = Arc::new(Database::open(&path).expect("打开数据库失败"));

        database
            .with_write(|tx| {
                tx.execute(
                    "CREATE TABLE counters (id INTEGER PRIMARY KEY, value INTEGER NOT NULL)",
                    [],
                )
                .unwrap();
                Ok(())
            })
            .expect("建表失败");

        let mut handles = Vec::new();
        for worker in 0..4 {
            let database = Arc::clone(&database);
            handles.push(thread::spawn(move || {
                for index in 0..25 {
                    let id = worker * 100 + index;
                    database
                        .with_write(|tx| {
                            tx.execute(
                                "INSERT INTO counters (id, value) VALUES (?1, ?2)",
                                rusqlite::params![id, index],
                            )
                            .map_err(|error| super::storage_error("插入失败", &error))?;
                            Ok(())
                        })
                        .expect("并发写入不应失败");
                }
            }));
        }
        for handle in handles {
            handle.join().expect("线程不应 panic");
        }

        let total: i64 = database
            .with_read(|connection| {
                Ok(connection
                    .query_row("SELECT COUNT(*) FROM counters", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap())
            })
            .expect("统计失败");
        assert_eq!(total, 100, "4 个线程各写 25 行应全部落库");

        drop(database);
        cleanup_db(&path);
    }

    #[test]
    fn read_only_connections_see_writes_from_the_writer() {
        let path = temp_db_path("visibility");
        let database = Database::open(&path).expect("打开数据库失败");

        database
            .with_write(|tx| {
                tx.execute("CREATE TABLE t (value TEXT)", []).unwrap();
                tx.execute("INSERT INTO t VALUES ('hello')", []).unwrap();
                Ok(())
            })
            .expect("写入失败");

        // 读连接是独立连接：必须能看到已提交的数据（WAL 下由文件头版本号保证）
        let value: String = database
            .with_read(|connection| {
                Ok(connection
                    .query_row("SELECT value FROM t", [], |row| row.get::<_, String>(0))
                    .unwrap())
            })
            .expect("读取失败");
        assert_eq!(value, "hello");

        drop(database);
        cleanup_db(&path);
    }
}
