//! 设置仓储（`settings` 表）。
//!
//! # 契约
//!
//! - `scope`：`global`（全局）或 `repo`（仓库级）。仓库级必须带 `repo_id`。
//! - `value`：**JSON 字符串**。为什么不让这一层理解具体的值类型：
//!   设置项会随里程碑不断增加（主题、密度、代理、并发数…），
//!   让存储层解析每一种值意味着每加一个设置项都要改存储层。
//!   这里只负责"以 JSON 字符串存取"，类型解析交给上层（T0.7 的命令层与前端 store）。
//! - 全局设置的 `repo_id` 存 `NULL`。注意 SQLite 的坑：`NULL` 在唯一约束里互不相等，
//!   因此唯一性靠迁移里的 `COALESCE(repo_id, -1)` 索引保证（见 0001_init.sql 的注释）。

use std::collections::BTreeMap;

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use rusqlite::{params, OptionalExtension};

use crate::database::{storage_error, Database};

/// 设置的归属范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// 全局设置（对所有仓库生效）。
    Global,
    /// 仓库级设置。
    Repo {
        /// 仓库 id（对应 `repositories.id`）。
        repo_id: i64,
    },
}

impl Scope {
    /// 数据库中的 scope 字面量。
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Repo { .. } => "repo",
        }
    }

    /// 数据库中的 repo_id（全局为 NULL）。
    pub const fn repo_id(&self) -> Option<i64> {
        match self {
            Self::Global => None,
            Self::Repo { repo_id } => Some(*repo_id),
        }
    }

    /// 从 IPC 参数解析（外部输入，必须收敛到已知取值）。
    pub fn parse(scope: &str, repo_id: Option<i64>) -> AppResult<Self> {
        match scope {
            "global" => Ok(Self::Global),
            "repo" => match repo_id {
                Some(repo_id) => Ok(Self::Repo { repo_id }),
                // 仓库级设置没有 repo_id 是明确的参数错误，不能静默降级成全局——
                // 那会让"仓库 A 的设置"悄悄写到所有仓库上
                None => Err(AppError::new(
                    ErrorCode::Validation,
                    "repo scope requires repoId",
                )),
            },
            other => Err(
                AppError::new(ErrorCode::Validation, "unknown settings scope")
                    .with_detail(other.to_owned()),
            ),
        }
    }
}

/// 设置仓储。
#[derive(Debug)]
pub struct SettingsRepository<'a> {
    database: &'a Database,
}

impl<'a> SettingsRepository<'a> {
    /// 绑定到某个数据库。
    pub const fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// 读取一个设置项（不存在返回 `None`，而不是报错）。
    pub fn get(&self, scope: &Scope, key: &str) -> AppResult<Option<String>> {
        self.database.with_read(|connection| {
            connection
                .query_row(
                    "SELECT value FROM settings WHERE scope = ?1 AND COALESCE(repo_id, -1) = COALESCE(?2, -1) AND key = ?3",
                    params![scope.as_str(), scope.repo_id(), key],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| storage_error("读取设置失败", &error))
        })
    }

    /// 写入（或覆盖）一个设置项。
    ///
    /// 使用 `INSERT OR REPLACE`：设置是**幂等覆盖**语义，不需要"先查后插"，
    /// 也就不会有两个并发写入者都以为自己是第一个的问题。
    pub fn set(&self, scope: &Scope, key: &str, value: &str) -> AppResult<()> {
        if key.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "settings key is empty",
            ));
        }

        self.database.with_write(|transaction| {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO settings (scope, repo_id, key, value) VALUES (?1, ?2, ?3, ?4)",
                    params![scope.as_str(), scope.repo_id(), key, value],
                )
                .map_err(|error| storage_error("写入设置失败", &error))?;
            Ok(())
        })
    }

    /// 读取某个范围下的全部设置。
    ///
    /// 返回 `BTreeMap` 而不是 `HashMap`：调用方（命令层）会把它序列化成 JSON 给前端，
    /// 稳定的键序让"两次读取结果是否一致"这类对比在测试与排查中更可靠。
    pub fn all(&self, scope: &Scope) -> AppResult<BTreeMap<String, String>> {
        self.database.with_read(|connection| {
            let mut statement = connection
                .prepare_cached(
                    "SELECT key, value FROM settings WHERE scope = ?1 AND COALESCE(repo_id, -1) = COALESCE(?2, -1) ORDER BY key",
                )
                .map_err(|error| storage_error("准备查询设置失败", &error))?;

            let rows = statement
                .query_map(params![scope.as_str(), scope.repo_id()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| storage_error("查询设置失败", &error))?;

            let mut result = BTreeMap::new();
            for row in rows {
                let (key, value) = row.map_err(|error| storage_error("读取设置行失败", &error))?;
                result.insert(key, value);
            }
            Ok(result)
        })
    }

    /// 删除一个设置项（用于"恢复默认值"）。
    pub fn remove(&self, scope: &Scope, key: &str) -> AppResult<()> {
        self.database.with_write(|transaction| {
            transaction
                .execute(
                    "DELETE FROM settings WHERE scope = ?1 AND COALESCE(repo_id, -1) = COALESCE(?2, -1) AND key = ?3",
                    params![scope.as_str(), scope.repo_id(), key],
                )
                .map_err(|error| storage_error("删除设置失败", &error))?;
            Ok(())
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::ErrorCode;

    use super::{Scope, SettingsRepository};
    use crate::database::Database;
    use crate::migrations::migrate;

    fn repository() -> Database {
        let database = Database::open_in_memory().expect("打开内存库失败");
        migrate(&database).expect("迁移失败");
        database
    }

    #[test]
    fn set_then_get_round_trips_json_values() {
        let database = repository();
        let settings = SettingsRepository::new(&database);

        settings
            .set(&Scope::Global, "ui.density", r#"{"mode":"compact"}"#)
            .expect("写入失败");

        let value = settings
            .get(&Scope::Global, "ui.density")
            .expect("读取失败");
        assert_eq!(value.as_deref(), Some(r#"{"mode":"compact"}"#));
    }

    #[test]
    fn missing_key_returns_none_instead_of_error() {
        let database = repository();
        let settings = SettingsRepository::new(&database);

        assert_eq!(settings.get(&Scope::Global, "nope").unwrap(), None);
    }

    #[test]
    fn set_overwrites_the_same_key() {
        let database = repository();
        let settings = SettingsRepository::new(&database);

        settings.set(&Scope::Global, "ui.density", "1").unwrap();
        settings.set(&Scope::Global, "ui.density", "2").unwrap();

        assert_eq!(
            settings
                .get(&Scope::Global, "ui.density")
                .unwrap()
                .as_deref(),
            Some("2")
        );
        assert_eq!(settings.all(&Scope::Global).unwrap().len(), 1);
    }

    /// 全局与仓库级必须互不干扰：这是"改了仓库 A 的设置影响到仓库 B"的防线。
    #[test]
    fn scopes_are_isolated() {
        let database = repository();
        let settings = SettingsRepository::new(&database);

        settings
            .set(&Scope::Global, "ui.density", "global-value")
            .unwrap();
        settings
            .set(&Scope::Repo { repo_id: 1 }, "ui.density", "repo-1-value")
            .unwrap();
        settings
            .set(&Scope::Repo { repo_id: 2 }, "ui.density", "repo-2-value")
            .unwrap();

        assert_eq!(
            settings
                .get(&Scope::Global, "ui.density")
                .unwrap()
                .as_deref(),
            Some("global-value")
        );
        assert_eq!(
            settings
                .get(&Scope::Repo { repo_id: 1 }, "ui.density")
                .unwrap()
                .as_deref(),
            Some("repo-1-value")
        );
        assert_eq!(
            settings
                .get(&Scope::Repo { repo_id: 2 }, "ui.density")
                .unwrap()
                .as_deref(),
            Some("repo-2-value")
        );

        // all() 也必须按范围隔离
        assert_eq!(settings.all(&Scope::Global).unwrap().len(), 1);
        assert_eq!(settings.all(&Scope::Repo { repo_id: 1 }).unwrap().len(), 1);
    }

    #[test]
    fn all_returns_sorted_keys() {
        let database = repository();
        let settings = SettingsRepository::new(&database);

        settings.set(&Scope::Global, "z", "1").unwrap();
        settings.set(&Scope::Global, "a", "2").unwrap();
        settings.set(&Scope::Global, "m", "3").unwrap();

        let keys: Vec<String> = settings.all(&Scope::Global).unwrap().into_keys().collect();
        assert_eq!(keys, vec!["a", "m", "z"]);
    }

    #[test]
    fn remove_deletes_only_the_targeted_key() {
        let database = repository();
        let settings = SettingsRepository::new(&database);

        settings.set(&Scope::Global, "a", "1").unwrap();
        settings.set(&Scope::Global, "b", "2").unwrap();
        settings.remove(&Scope::Global, "a").unwrap();

        assert_eq!(settings.get(&Scope::Global, "a").unwrap(), None);
        assert_eq!(
            settings.get(&Scope::Global, "b").unwrap().as_deref(),
            Some("2")
        );
    }

    /// T0.7 验收项："重启应用后设置保持"。
    ///
    /// 用"关闭连接 → 重新打开同一个文件 → 再迁移一次（幂等）→ 读回值"来模拟重启：
    /// 这是能在测试里复现的最接近真实的路径，比手工点界面更可靠，
    /// 也顺带覆盖了"第二次启动时迁移不重复执行"。
    #[test]
    fn settings_survive_a_restart() {
        let path = crate::database::test_support::temp_db_path("restart");

        {
            let database = Database::open(&path).expect("首次打开失败");
            migrate(&database).expect("首次迁移失败");
            SettingsRepository::new(&database)
                .set(&Scope::Global, "ui.density", "\"compact\"")
                .expect("写入失败");
        } // 连接在此关闭，等价于退出应用

        {
            let database = Database::open(&path).expect("重新打开失败");
            let report = migrate(&database).expect("二次迁移失败");
            assert!(report.applied.is_empty(), "重启不应重复执行迁移");

            let value = SettingsRepository::new(&database)
                .get(&Scope::Global, "ui.density")
                .expect("读取失败");
            assert_eq!(
                value.as_deref(),
                Some("\"compact\""),
                "重启后设置必须仍然存在"
            );
        }

        crate::database::test_support::cleanup_db(&path);
    }

    #[test]
    fn empty_key_is_rejected() {
        let database = repository();
        let settings = SettingsRepository::new(&database);

        let error = settings.set(&Scope::Global, "   ", "x").unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[test]
    fn scope_parsing_rejects_invalid_input() {
        assert_eq!(Scope::parse("global", None).unwrap(), Scope::Global);
        assert_eq!(
            Scope::parse("repo", Some(7)).unwrap(),
            Scope::Repo { repo_id: 7 }
        );

        // 仓库级缺 repo_id：必须报错，不能静默当成全局
        let error = Scope::parse("repo", None).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);

        let error = Scope::parse("everywhere", None).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(
            error.detail.as_deref(),
            Some("everywhere"),
            "原始输入应保留在 detail 里便于定位"
        );
    }
}
