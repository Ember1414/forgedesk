//! 版本化迁移。
//!
//! # 设计要点
//!
//! 1. **脚本内嵌进二进制**（`include_str!`）。原因：迁移文件必须与它服务的代码
//!    同版本发布，任何"运行时去某个目录找 .sql"的方案都会遇到"用户机器上文件缺失"。
//! 2. **每个迁移一个事务**。SQLite 的 DDL 是事务性的，因此失败的迁移不会留下半个结构；
//!    但已经成功的旧迁移**不会**被回滚（那是常规做法：回滚旧版本等于让新代码面对旧结构）。
//! 3. **迁移前备份**：只在"已有数据的库"上备份。全新库没有可丢的东西，
//!    给它做备份只会制造噪音文件。备份保留最近 3 份。
//! 4. 迁移记录写在 `schema_migrations` 表里（而不是 `PRAGMA user_version`）：
//!    前者能留下名字与时间，排查"这个库到底是什么时候变成这样的"时有直接证据。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use rusqlite::Connection;
use tracing::{info, warn};

use crate::database::{io_error, storage_error, Database};

/// 备份保留数量。
const KEEP_BACKUPS: usize = 3;

/// 备份文件名的前缀（用于识别与清理）。
const BACKUP_MARKER: &str = ".backup-";

/// 一个迁移。
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// 版本号，必须连续递增（从 1 开始）。
    pub version: i64,
    /// 名称，出现在日志与 `schema_migrations.name` 中。
    pub name: &'static str,
    /// SQL 文本（编译期内嵌）。
    pub sql: &'static str,
}

impl Migration {
    /// 执行本迁移（调用方保证在事务中）。
    pub fn apply(&self, connection: &Connection) -> AppResult<()> {
        connection
            .execute_batch(self.sql)
            .map_err(|error| storage_error(&format!("迁移 {} 执行失败", self.name), &error))?;

        connection
            .execute(
                "INSERT OR REPLACE INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![self.version, self.name, now_millis()],
            )
            .map_err(|error| storage_error("写入迁移记录失败", &error))?;

        Ok(())
    }
}

/// 全部迁移，按版本升序。
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "0001_init",
    sql: include_str!("../migrations/0001_init.sql"),
}];

/// 迁移执行结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// 迁移前的版本。
    pub from_version: i64,
    /// 迁移后的版本。
    pub to_version: i64,
    /// 本次应用的迁移名。
    pub applied: Vec<&'static str>,
    /// 备份文件路径（未备份时为 `None`）。
    pub backup_path: Option<PathBuf>,
}

/// 当前已应用的版本（空库返回 0）。
pub fn current_version(connection: &Connection) -> AppResult<i64> {
    let has_table: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value == 1)
        .map_err(|error| storage_error("检查迁移表失败", &error))?;

    if !has_table {
        return Ok(0);
    }

    connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| storage_error("读取迁移版本失败", &error))
}

/// 执行迁移（使用编译期内的迁移清单）。
///
/// 幂等：已应用的迁移会被跳过，因此启动时无条件调用即可。
pub fn migrate(database: &Database) -> AppResult<MigrationReport> {
    migrate_with(database, MIGRATIONS)
}

/// 用给定的迁移清单执行迁移。
///
/// 为什么把清单参数化：备份与"部分迁移"这类路径只有在**存在待应用迁移且库已有数据**时才会走到，
/// 而当前只有 0001。若把清单写死，这些分支要等到将来某个迁移落地才可能被覆盖——
/// 那正是"最容易出问题的代码从未被执行过"的情形。参数化后测试可以注入一个假的 0002。
pub fn migrate_with(database: &Database, migrations: &[Migration]) -> AppResult<MigrationReport> {
    let from_version = database.with_read(current_version)?;
    let pending: Vec<&Migration> = migrations
        .iter()
        .filter(|migration| migration.version > from_version)
        .collect();

    if pending.is_empty() {
        return Ok(MigrationReport {
            from_version,
            to_version: from_version,
            applied: Vec::new(),
            backup_path: None,
        });
    }

    // 已有数据的库才备份：全新库没有可丢的东西，备份只会制造噪音文件
    let backup_path = if from_version > 0 {
        database
            .path()
            .map(|path| backup_database(database, path, from_version))
            .transpose()?
    } else {
        None
    };

    let mut applied = Vec::new();
    for migration in pending {
        database.with_write(|transaction| migration.apply(transaction))?;
        info!(
            version = migration.version,
            name = migration.name,
            "已应用迁移"
        );
        applied.push(migration.name);
    }

    let to_version = database.with_read(current_version)?;
    info!(from = from_version, to = to_version, "数据库迁移完成");

    Ok(MigrationReport {
        from_version,
        to_version,
        applied,
        backup_path,
    })
}

/// 迁移前备份数据库文件，并清理超量的旧备份。
///
/// 必须先 checkpoint：否则最近的写入还在 `-wal` 里，复制出来的主文件不含它们——
/// 备份看起来成功了，恢复时却少了最后几次操作。
fn backup_database(database: &Database, path: &Path, version: i64) -> AppResult<PathBuf> {
    database.checkpoint()?;

    let backup_path = backup_path_for(path, version);
    std::fs::copy(path, &backup_path).map_err(|error| {
        // 备份失败必须**中止迁移**：不能在没有退路的情况下改用户的数据
        AppError::new(
            ErrorCode::Storage,
            "backup before migration failed; migration aborted",
        )
        .with_detail(format!("{}: {error}", path.display()))
    })?;

    prune_backups(path)?;
    Ok(backup_path)
}

/// 备份文件名：`<db>.backup-<版本>-<时间戳>`。
fn backup_path_for(path: &Path, version: i64) -> PathBuf {
    let file_name = path.file_name().map_or_else(
        || "forgedesk.db".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    path.with_file_name(format!(
        "{file_name}{BACKUP_MARKER}v{version}-{}",
        now_millis()
    ))
}

/// 只保留最近 [`KEEP_BACKUPS`] 份备份（按文件名里的时间戳排序）。
fn prune_backups(path: &Path) -> AppResult<()> {
    let Some(directory) = path.parent() else {
        return Ok(());
    };
    let prefix = format!(
        "{}{BACKUP_MARKER}",
        path.file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    );

    let mut backups: Vec<PathBuf> = std::fs::read_dir(directory)
        .map_err(|error| io_error("读取备份目录失败", &error))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
        })
        .collect();

    // 文件名里带毫秒时间戳，字典序即时间序
    backups.sort();
    while backups.len() > KEEP_BACKUPS {
        let oldest = backups.remove(0);
        if let Err(error) = std::fs::remove_file(&oldest) {
            // 清理失败不影响本次迁移：不该让"删旧备份失败"拦住数据结构升级
            warn!(path = %oldest.display(), %error, "清理旧备份失败");
        }
    }

    Ok(())
}

/// 当前时间（Unix 毫秒）。
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use rusqlite::Connection;

    use super::{
        current_version, migrate, migrate_with, Migration, BACKUP_MARKER, KEEP_BACKUPS, MIGRATIONS,
    };
    use crate::database::test_support::{cleanup_db, temp_db_path};
    use crate::database::Database;

    /// PLAN §5.10 要求的全部表。
    const EXPECTED_TABLES: &[&str] = &[
        "repositories",
        "settings",
        "snapshots",
        "operation_records",
        "accounts",
        "api_cache",
        "audit_log",
    ];

    fn table_exists(connection: &Connection, name: &str) -> bool {
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    }

    #[test]
    fn migration_list_is_contiguous_and_unique() {
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                migration.version,
                i64::try_from(index).unwrap() + 1,
                "迁移版本必须从 1 开始连续递增（第 {index} 项为 {}）",
                migration.version
            );
            assert!(
                !migration.sql.trim().is_empty(),
                "{} 的 SQL 为空",
                migration.name
            );
        }
    }

    /// 验收要求：构造 v0 库 → 迁移到当前 → 断言表存在。
    #[test]
    fn migrates_a_fresh_v0_database_to_current() {
        let path = temp_db_path("migrate-v0");
        let database = Database::open(&path).expect("打开数据库失败");

        // v0：只有空文件，没有任何表
        assert_eq!(database.with_read(current_version).unwrap(), 0);

        let report = migrate(&database).expect("迁移失败");
        assert_eq!(report.from_version, 0);
        assert_eq!(report.to_version, MIGRATIONS.last().unwrap().version);
        assert_eq!(report.applied.len(), MIGRATIONS.len());
        assert!(report.backup_path.is_none(), "全新库不需要备份");

        for table in EXPECTED_TABLES {
            let exists = database
                .with_read(|connection| Ok(table_exists(connection, table)))
                .unwrap();
            assert!(exists, "迁移后应存在表 {table}");
        }

        drop(database);
        cleanup_db(&path);
    }

    #[test]
    fn migrate_is_idempotent() {
        let database = Database::open_in_memory().expect("打开内存库失败");

        let first = migrate(&database).expect("首次迁移失败");
        let second = migrate(&database).expect("重复迁移失败");

        assert_eq!(second.applied.len(), 0, "已应用过的迁移不应重复执行");
        assert_eq!(first.to_version, second.to_version);
        assert!(second.backup_path.is_none(), "无待应用迁移时不应产生备份");
    }

    /// 已有数据的库在迁移前必须留下备份（这是"改坏了也要能捞回来"的底线）。
    ///
    /// 用注入的假 0002 走到这条分支：真实清单里只有 0001，如果只测真实清单，
    /// "备份 + 应用第二个迁移"这条路径会一直没被执行过。
    #[test]
    fn backs_up_existing_database_before_applying_new_migration() {
        let path = temp_db_path("backup");
        let database = Database::open(&path).expect("打开数据库失败");
        migrate(&database).expect("首次迁移失败");

        const FAKE_0002: Migration = Migration {
            version: 2,
            name: "0002_fake_for_test",
            sql: "CREATE TABLE test_only (id INTEGER PRIMARY KEY);",
        };

        let report = migrate_with(&database, &[MIGRATIONS[0], FAKE_0002]).expect("二次迁移失败");

        assert_eq!(report.from_version, 1);
        assert_eq!(report.to_version, 2);
        assert_eq!(report.applied, vec!["0002_fake_for_test"]);

        let backup = report.backup_path.expect("已有数据的库必须备份");
        assert!(backup.exists(), "备份文件应真实存在：{}", backup.display());
        let name = backup.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.contains(BACKUP_MARKER), "备份文件名应含标记：{name}");
        assert!(name.contains("v1"), "备份文件名应记录迁移前版本：{name}");

        // 假迁移的建表语句确实生效，说明事务提交了
        let exists = database
            .with_read(|connection| Ok(table_exists(connection, "test_only")))
            .unwrap();
        assert!(exists);

        let _ = std::fs::remove_file(&backup);
        drop(database);
        cleanup_db(&path);
    }

    /// 迁移 SQL 失败时必须回滚，且**不影响**已有的旧结构。
    #[test]
    fn failed_migration_rolls_back_its_transaction() {
        let database = Database::open_in_memory().expect("打开内存库失败");
        migrate(&database).expect("首次迁移失败");

        const BROKEN_0002: Migration = Migration {
            version: 2,
            name: "0002_broken",
            sql: "CREATE TABLE half_applied (id INTEGER PRIMARY KEY); INSERT INTO nonexistent_table VALUES (1);",
        };

        let result = migrate_with(&database, &[MIGRATIONS[0], BROKEN_0002]);
        assert!(result.is_err(), "坏迁移必须失败");
        assert_eq!(
            result.unwrap_err().code,
            forgedesk_domain::ErrorCode::Storage,
            "迁移失败应给出 STORAGE 错误码"
        );

        // 该迁移内部的建表语句必须被回滚（SQLite 的 DDL 在事务里）
        let exists = database
            .with_read(|connection| Ok(table_exists(connection, "half_applied")))
            .unwrap();
        assert!(!exists, "失败迁移不应留下半张表");

        // 版本号没有被推进，下次启动会重试
        assert_eq!(database.with_read(current_version).unwrap(), 1);
    }

    #[test]
    fn keeps_only_the_newest_backups() {
        let path = temp_db_path("prune");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        // 直接构造 6 个备份文件，验证清理逻辑
        for index in 0..6 {
            let name = format!(
                "{}{BACKUP_MARKER}v1-000000000000{index}",
                path.file_name().unwrap().to_string_lossy()
            );
            std::fs::write(path.with_file_name(name), b"x").unwrap();
        }

        super::prune_backups(&path).expect("清理备份失败");

        let remaining: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(BACKUP_MARKER))
            .collect();
        assert_eq!(
            remaining.len(),
            KEEP_BACKUPS,
            "应只保留最近 3 份备份：{remaining:?}"
        );

        for name in remaining {
            let _ = std::fs::remove_file(path.with_file_name(name));
        }
    }

    /// 全局设置用 NULL 的 repo_id：验证 SQLite 的 NULL 唯一性坑已被索引兜住。
    #[test]
    fn global_scope_settings_are_unique() {
        let database = Database::open_in_memory().expect("打开内存库失败");
        migrate(&database).expect("迁移失败");

        database
            .with_write(|tx| {
                tx.execute(
                    "INSERT INTO settings (scope, repo_id, key, value) VALUES ('global', NULL, 'ui.density', '\"comfortable\"')",
                    [],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();

        // 同一 (scope, NULL, key) 再插一次必须失败——否则会出现"改不动的设置"
        let second = database.with_write(|tx| {
            tx.execute(
                "INSERT INTO settings (scope, repo_id, key, value) VALUES ('global', NULL, 'ui.density', '\"compact\"')",
                [],
            )
            .map_err(|error| crate::database::storage_error("插入失败", &error))?;
            Ok(())
        });
        assert!(
            second.is_err(),
            "全局设置必须唯一（NULL 在 UNIQUE 里互不相等，靠 COALESCE 索引救回）"
        );

        // INSERT OR REPLACE 应当覆盖而不是新增
        database
            .with_write(|tx| {
                tx.execute(
                    "INSERT OR REPLACE INTO settings (scope, repo_id, key, value) VALUES ('global', NULL, 'ui.density', '\"compact\"')",
                    [],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();

        let count: i64 = database
            .with_read(|connection| {
                Ok(connection
                    .query_row("SELECT COUNT(*) FROM settings", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap())
            })
            .unwrap();
        assert_eq!(count, 1, "OR REPLACE 不应产生第二行");
    }
}
