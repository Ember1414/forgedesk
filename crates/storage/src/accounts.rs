//! 托管平台账号仓储（`accounts` 表）。
//!
//! 只做数据存取，不含登录编排（那在 `services::accounts`）。
//! `credential_ref` 存的是 keyring 的 account 名（`provider:host:login`），
//! **密文本体永远在系统凭据库里**（红线 R8）：这张表丢了最多丢"列表"，
//! 不会丢令牌；反过来，删除账号必须先删 keyring 条目再删行，
//! 否则会留下一个查不到、也读不出来的孤儿凭据。

use forgedesk_domain::AppResult;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::database::{storage_error, Database};

/// 一条托管平台账号。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    /// 主键（UUID；服务层在"同 provider+host+login 再次登录"时沿用旧 id）。
    pub id: String,
    /// 平台标识（`github` / `gitlab` / `gitea`）。
    pub provider: String,
    /// 主机（`github.com`、`acme.ghe.com`、自建 GHE 域名）。
    pub host: String,
    /// 登录名（`octocat`）。
    pub login: String,
    /// 头像地址。
    pub avatar_url: Option<String>,
    /// 令牌作用域（逗号分隔；细粒度 PAT 可为空串/`NULL`）。
    pub scopes: Option<String>,
    /// keyring 条目名（`provider:host:login`），凭据本体的唯一线索。
    pub credential_ref: String,
    /// 首次登录时间（Unix 毫秒；沿用旧记录时保持不变）。
    pub created_at: Option<i64>,
}

/// 账号仓储。
#[derive(Debug)]
pub struct AccountStore<'a> {
    database: &'a Database,
}

impl<'a> AccountStore<'a> {
    /// 绑定到某个数据库。
    pub const fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// 按 id 覆盖写入。
    pub fn upsert(&self, record: &AccountRecord) -> AppResult<()> {
        self.database.with_write(|connection| {
            connection
                .execute(
                    "INSERT INTO accounts (id, provider, host, login, avatar_url, scopes, \
                     credential_ref, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(id) DO UPDATE SET
                       provider = excluded.provider,
                       host = excluded.host,
                       login = excluded.login,
                       avatar_url = excluded.avatar_url,
                       scopes = excluded.scopes,
                       credential_ref = excluded.credential_ref,
                       created_at = excluded.created_at",
                    params![
                        record.id,
                        record.provider,
                        record.host,
                        record.login,
                        record.avatar_url,
                        record.scopes,
                        record.credential_ref,
                        record.created_at,
                    ],
                )
                .map_err(|error| storage_error("写入账号失败", &error))?;
            Ok(())
        })
    }

    /// 按"平台 + 主机 + 登录名"找账号（重复登录同一账号时沿用旧 id 与 created_at）。
    pub fn find_by_login(
        &self,
        provider: &str,
        host: &str,
        login: &str,
    ) -> AppResult<Option<AccountRecord>> {
        self.database.with_read(|connection| {
            connection
                .query_row(
                    "SELECT id, provider, host, login, avatar_url, scopes, credential_ref, \
                     created_at
                     FROM accounts WHERE provider = ?1 AND host = ?2 AND login = ?3",
                    params![provider, host, login],
                    Self::map_row,
                )
                .optional()
                .map_err(|error| storage_error("查询账号失败", &error))
        })
    }

    /// 全部账号（按创建时间排序，界面列表的顺序）。
    pub fn list(&self) -> AppResult<Vec<AccountRecord>> {
        self.database.with_read(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, provider, host, login, avatar_url, scopes, credential_ref, \
                     created_at
                     FROM accounts ORDER BY COALESCE(created_at, 0) ASC",
                )
                .map_err(|error| storage_error("列出账号失败", &error))?;
            let rows = statement
                .query_map([], Self::map_row)
                .map_err(|error| storage_error("列出账号失败", &error))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| storage_error("列出账号失败", &error))?;
            Ok(rows)
        })
    }

    /// 按 id 读取。
    pub fn get(&self, id: &str) -> AppResult<Option<AccountRecord>> {
        self.database.with_read(|connection| {
            connection
                .query_row(
                    "SELECT id, provider, host, login, avatar_url, scopes, credential_ref, \
                     created_at
                     FROM accounts WHERE id = ?1",
                    params![id],
                    Self::map_row,
                )
                .optional()
                .map_err(|error| storage_error("查询账号失败", &error))
        })
    }

    /// 按 id 删除（幂等：不存在也成功）。
    pub fn delete(&self, id: &str) -> AppResult<()> {
        self.database.with_write(|connection| {
            connection
                .execute("DELETE FROM accounts WHERE id = ?1", params![id])
                .map_err(|error| storage_error("删除账号失败", &error))?;
            Ok(())
        })
    }

    fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccountRecord> {
        Ok(AccountRecord {
            id: row.get(0)?,
            provider: row.get(1)?,
            host: row.get(2)?,
            login: row.get(3)?,
            avatar_url: row.get(4)?,
            scopes: row.get(5)?,
            credential_ref: row.get(6)?,
            created_at: row.get(7)?,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::AccountRecord;
    use crate::database::Database;
    use crate::migrations::migrate;

    fn db() -> Database {
        let database = Database::open_in_memory().expect("打开内存库失败");
        migrate(&database).expect("迁移失败");
        database
    }

    fn record(id: &str, login: &str) -> AccountRecord {
        AccountRecord {
            id: id.to_owned(),
            provider: "github".to_owned(),
            host: "github.com".to_owned(),
            login: login.to_owned(),
            avatar_url: Some("https://avatars/u/1".to_owned()),
            scopes: Some("repo,read:org".to_owned()),
            credential_ref: format!("github:github.com:{login}"),
            created_at: Some(1_790_000_000),
        }
    }

    #[test]
    fn upsert_then_list_round_trips_every_column() {
        let database = db();
        let store = super::AccountStore::new(&database);

        store.upsert(&record("a1", "octocat")).unwrap();
        let listed = store.list().unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], record("a1", "octocat"));
    }

    #[test]
    fn find_by_login_matches_all_three_key_parts() {
        let database = db();
        let store = super::AccountStore::new(&database);
        store.upsert(&record("a1", "octocat")).unwrap();
        store
            .upsert(&AccountRecord {
                id: "a2".to_owned(),
                host: "acme.ghe.com".to_owned(),
                credential_ref: "github:acme.ghe.com:octocat".to_owned(),
                ..record("a2", "octocat")
            })
            .unwrap();

        assert_eq!(
            store
                .find_by_login("github", "github.com", "octocat")
                .unwrap()
                .unwrap()
                .id,
            "a1"
        );
        assert_eq!(
            store
                .find_by_login("github", "acme.ghe.com", "octocat")
                .unwrap()
                .unwrap()
                .id,
            "a2"
        );
        assert_eq!(
            store
                .find_by_login("gitlab", "github.com", "octocat")
                .unwrap(),
            None
        );
    }

    #[test]
    fn upsert_with_the_same_id_overwrites_instead_of_duplicating() {
        let database = db();
        let store = super::AccountStore::new(&database);
        store.upsert(&record("a1", "octocat")).unwrap();
        store
            .upsert(&AccountRecord {
                scopes: Some("repo".to_owned()),
                ..record("a1", "octocat")
            })
            .unwrap();

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].scopes.as_deref(), Some("repo"));
    }

    #[test]
    fn delete_is_idempotent_and_get_reflects_it() {
        let database = db();
        let store = super::AccountStore::new(&database);
        store.upsert(&record("a1", "octocat")).unwrap();

        store.delete("a1").unwrap();
        store.delete("a1").unwrap();
        assert_eq!(store.get("a1").unwrap(), None);
        assert_eq!(store.get("missing").unwrap(), None);
    }

    #[test]
    fn list_orders_by_created_at_with_nulls_first() {
        let database = db();
        let store = super::AccountStore::new(&database);
        store
            .upsert(&AccountRecord {
                created_at: Some(200),
                ..record("b", "beta")
            })
            .unwrap();
        store
            .upsert(&AccountRecord {
                created_at: Some(100),
                ..record("a", "alpha")
            })
            .unwrap();
        store
            .upsert(&AccountRecord {
                created_at: None,
                ..record("c", "gamma")
            })
            .unwrap();

        let ids = store
            .list()
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["c", "a", "b"]);
    }
}
