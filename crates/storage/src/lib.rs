//! 本地存储层：SQLite 仓储、迁移与查询。
//!
//! 归属里程碑：M0 / T0.7（连接管理、迁移框架、设置与仓库登记两张表）。
//!
//! # 边界
//!
//! - 本 crate **只做数据存取**，不含业务逻辑（"这个路径是不是仓库""默认分支叫什么"
//!   属于 M1 的 `services` 层）；
//! - 值的类型解析（设置项的具体 schema）不在这里：`settings.value` 就是一段 JSON 字符串，
//!   由上层决定含义。这样新增设置项不需要改存储层；
//! - 迁移脚本随二进制一起发布（`include_str!`），不存在"用户机器上缺 .sql 文件"的可能。
//!
//! # 并发模型（详见 [`database::Database`]）
//!
//! 单写多读：一个互斥的写连接（写操作走事务）+ 少量复用的读连接。
//! 所有连接都启用 WAL、外键与 5 秒 `busy_timeout`，避免短事务之间互相报 `SQLITE_BUSY`。

#![forbid(unsafe_code)]

pub mod database;
pub mod migrations;
pub mod operations;
pub mod repositories;
pub mod settings;
pub mod snapshots;

pub use database::{storage_error, Database};
pub use migrations::{
    current_version, migrate, migrate_with, Migration, MigrationReport, MIGRATIONS,
};
pub use operations::{NewOperation, OperationOutcome, OperationRecord, OperationStore};
pub use repositories::{RepositoryRecord, RepositoryStore, RepositoryUpsert};
pub use settings::{Scope, SettingsRepository};
pub use snapshots::{NewSnapshot, SnapshotRecord, SnapshotStore};

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
