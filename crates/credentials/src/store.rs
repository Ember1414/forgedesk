//! 面向用例的凭据存储接口（任务定义的四方法），以及"后端 + 索引"的组合实现。
//!
//! ```text
//! 用例层（services / commands）
//!        │  store/get/delete/list          ← 本模块的 trait
//!        ▼
//! IndexedCredentialStore  ── 元数据与列表 ──▶  CredentialIndex（文件 / 将来的 accounts 表）
//!        │
//!        ▼  密文本体
//!  CredentialBackend（系统 keyring / 加密保险库 / 内存）
//! ```
//!
//! 这样分层的好处：换成加密文件回退时，**只有 `backend` 换了一个实现**，
//! `store/get/delete/list` 的语义、错误映射与列表行为都不变。

use crate::backend::{BackendKind, CredentialBackend};
use crate::error::CredentialsError;
use crate::index::{CredentialIndex, IndexEntry};
use crate::model::{CredentialKind, CredentialMeta, CredentialRef, SERVICE_NAME};
use crate::secret::Secret;

/// 凭据存储的用例接口。
pub trait CredentialStore: Send + Sync {
    /// 保存（同引用覆盖）。`kind` 只用于列表展示，不参与密文存储。
    fn store(
        &self,
        key: &CredentialRef,
        kind: CredentialKind,
        secret: &Secret,
    ) -> Result<(), CredentialsError>;

    /// 读取；不存在时 [`CredentialsError::NotFound`]。
    fn get(&self, key: &CredentialRef) -> Result<Secret, CredentialsError>;

    /// 删除（幂等：连删两次不报错）。
    fn delete(&self, key: &CredentialRef) -> Result<(), CredentialsError>;

    /// 列出已保存的凭据元数据（不含密文）。
    fn list(&self) -> Result<Vec<CredentialMeta>, CredentialsError>;

    /// 密文实际存在哪里（设置页要如实告诉用户）。
    fn backend_kind(&self) -> BackendKind;

    /// 是否已保存过（默认实现走列表，避免各后端重复实现"存在性判断"）。
    fn contains(&self, key: &CredentialRef) -> Result<bool, CredentialsError> {
        let account = key.account();
        Ok(self
            .list()?
            .into_iter()
            .any(|meta| meta.key.account() == account))
    }
}

/// 时钟：写入时间要能"构造"而不是"等时间流逝"（docs/CODING_STYLE.md §2.4）。
pub type Clock = Box<dyn Fn() -> i64 + Send + Sync>;

/// 系统时间（Unix 毫秒）。
///
/// 时间回拨或系统时间未设置时返回 0 而不是 panic：写入时间只用于展示排序，
/// 不值得为它中断一次凭据保存。
pub fn system_clock() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

/// "后端 + 索引"的组合实现。
pub struct IndexedCredentialStore<B: CredentialBackend, I: CredentialIndex> {
    backend: B,
    index: I,
    service: String,
    clock: Clock,
}

impl<B: CredentialBackend, I: CredentialIndex> IndexedCredentialStore<B, I> {
    /// 用默认 service（[`SERVICE_NAME`]）与系统时钟组合。
    pub fn new(backend: B, index: I) -> Self {
        Self {
            backend,
            index,
            service: SERVICE_NAME.to_owned(),
            clock: Box::new(system_clock),
        }
    }

    /// 覆盖 service（测试与将来多实例隔离用；生产固定用 [`SERVICE_NAME`]）。
    #[must_use]
    pub fn with_service(mut self, service: impl Into<String>) -> Self {
        self.service = service.into();
        self
    }

    /// 覆盖时钟（测试用）。
    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// 当前后端（诊断用）。
    pub fn backend(&self) -> &B {
        &self.backend
    }
}

impl<B: CredentialBackend, I: CredentialIndex> CredentialStore for IndexedCredentialStore<B, I> {
    fn store(
        &self,
        key: &CredentialRef,
        kind: CredentialKind,
        secret: &Secret,
    ) -> Result<(), CredentialsError> {
        key.validate()?;
        if secret.is_empty() {
            // 空口令/空令牌写进去等于"存了一坨没用的东西"，用户下次会以为登录成功了
            return Err(CredentialsError::Invalid(
                "secret must not be empty".to_owned(),
            ));
        }

        let account = key.account();
        self.backend.set(&self.service, &account, secret)?;

        // 先写密文再写索引：反过来会出现"索引里有、读不到"的幽灵条目。
        // 索引写失败时密文留在系统库里（下次覆盖即可，不会丢数据），错误照实上抛——
        // 因为列表会因此少一条，用户需要知道。
        self.index.upsert(IndexEntry {
            account,
            kind,
            created_at_ms: (self.clock)(),
        })
    }

    fn get(&self, key: &CredentialRef) -> Result<Secret, CredentialsError> {
        key.validate()?;
        self.backend.get(&self.service, &key.account())
    }

    fn delete(&self, key: &CredentialRef) -> Result<(), CredentialsError> {
        key.validate()?;
        let account = key.account();
        self.backend.delete(&self.service, &account)?;
        self.index.remove(&account)
    }

    fn list(&self) -> Result<Vec<CredentialMeta>, CredentialsError> {
        let mut metas: Vec<CredentialMeta> = self
            .index
            .list()?
            .into_iter()
            .filter_map(|entry| {
                // 索引里坏掉的那条不能猜：跳过而不是伪造一个引用
                entry.reference().map(|key| CredentialMeta {
                    key,
                    kind: entry.kind,
                    created_at_ms: entry.created_at_ms,
                })
            })
            .collect();
        metas.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(metas)
    }

    fn backend_kind(&self) -> BackendKind {
        self.backend.kind()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::backend::MemoryBackend;
    use crate::index::MemoryIndex;

    fn reference(login: &str) -> CredentialRef {
        CredentialRef::new("github", "github.com", login).expect("valid reference")
    }

    fn store() -> IndexedCredentialStore<MemoryBackend, MemoryIndex> {
        IndexedCredentialStore::new(MemoryBackend::new(), MemoryIndex::new())
            .with_service("test.service")
            .with_clock(Box::new(|| 1_700_000_000_000))
    }

    #[test]
    fn a_stored_credential_can_be_read_back_and_listed() {
        let store = store();

        store
            .store(
                &reference("octocat"),
                CredentialKind::Pat,
                &Secret::new("ghp_x"),
            )
            .expect("store");

        assert_eq!(
            store.get(&reference("octocat")).expect("get").expose(),
            "ghp_x"
        );
        let metas = store.list().expect("list");
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].kind, CredentialKind::Pat);
        assert_eq!(metas[0].created_at_ms, 1_700_000_000_000);
        assert!(store.contains(&reference("octocat")).expect("contains"));
    }

    #[test]
    fn listing_is_sorted_by_account_so_the_settings_page_is_stable() {
        let store = store();
        for login in ["zed", "alice", "bob"] {
            store
                .store(&reference(login), CredentialKind::Pat, &Secret::new("x"))
                .expect("store");
        }

        let logins: Vec<String> = store
            .list()
            .expect("list")
            .into_iter()
            .map(|meta| meta.key.login)
            .collect();

        assert_eq!(logins, vec!["alice", "bob", "zed"]);
    }

    #[test]
    fn storing_twice_updates_the_kind_but_keeps_one_entry() {
        let store = store();
        store
            .store(
                &reference("octocat"),
                CredentialKind::Pat,
                &Secret::new("old"),
            )
            .expect("first");
        store
            .store(
                &reference("octocat"),
                CredentialKind::Password,
                &Secret::new("new"),
            )
            .expect("second");

        let metas = store.list().expect("list");
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].kind, CredentialKind::Password);
        assert_eq!(
            store.get(&reference("octocat")).expect("get").expose(),
            "new"
        );
    }

    #[test]
    fn deleting_removes_both_the_secret_and_its_index_entry() {
        let store = store();
        store
            .store(
                &reference("octocat"),
                CredentialKind::Pat,
                &Secret::new("x"),
            )
            .expect("store");

        store.delete(&reference("octocat")).expect("delete");

        assert!(store.list().expect("list").is_empty());
        assert!(matches!(
            store.get(&reference("octocat")),
            Err(CredentialsError::NotFound(_))
        ));
        // 幂等：界面上的"删除"可能被连点
        store.delete(&reference("octocat")).expect("delete again");
    }

    #[test]
    fn an_empty_secret_is_refused_before_it_reaches_the_backend() {
        let store = store();

        let error = store
            .store(&reference("octocat"), CredentialKind::Pat, &Secret::new(""))
            .expect_err("empty secret must be refused");

        assert!(matches!(error, CredentialsError::Invalid(_)));
        assert!(store.list().expect("list").is_empty());
    }

    #[test]
    fn an_invalid_reference_is_refused_on_every_operation() {
        let store = store();
        let broken = CredentialRef {
            provider: "git:hub".to_owned(),
            host: "github.com".to_owned(),
            login: "octocat".to_owned(),
        };

        assert!(store
            .store(&broken, CredentialKind::Pat, &Secret::new("x"))
            .is_err());
        assert!(store.get(&broken).is_err());
        assert!(store.delete(&broken).is_err());
    }

    #[test]
    fn reading_an_unknown_credential_reports_not_found() {
        let store = store();

        assert!(matches!(
            store.get(&reference("nobody")),
            Err(CredentialsError::NotFound(_))
        ));
        assert!(!store.contains(&reference("nobody")).expect("contains"));
    }

    #[test]
    fn the_backend_kind_is_visible_to_the_settings_page() {
        assert_eq!(store().backend_kind(), BackendKind::Memory);
    }

    #[test]
    fn a_broken_index_entry_is_skipped_instead_of_inventing_a_reference() {
        let store = store();
        // 模拟"索引被别的版本写坏了一条"
        store
            .index
            .upsert(IndexEntry {
                account: "broken".to_owned(),
                kind: CredentialKind::Pat,
                created_at_ms: 1,
            })
            .expect("upsert");

        assert!(store.list().expect("list").is_empty());
    }
}
