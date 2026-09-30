//! 托管平台账号服务（T4.3/T4.4）：登录、多账号模型、凭据落地。
//!
//! # 职责与边界
//!
//! - 登录编排：PAT 校验（一次 `/user`）、Device Flow（启动 → 轮询等待 →
//!   用新令牌取账号信息）；轮询协议本身在 `provider::poll_until_authorized`；
//! - 落地：令牌密文进凭据库（[`SharedStore`]，keyring / 加密保险库），
//!   账号元数据进 `accounts` 表（`AccountStore`）。**本服务绝不持有令牌**：
//!   参数进来、存进去、就离开（红线 R8）；
//! - 多账号：同一平台同一 host 下多个登录名并存（按 `provider+host+login`
//!   去重，重复登录沿用旧 id 与创建时间）。
//!
//! # 写入顺序（失败模式考量）
//!
//! 先写凭据库、后写 `accounts` 行。凭据库写失败 → 两边都没有（干净）；
//! 凭据库成功、DB 失败 → 一个"查不到列表但密文还在"的孤儿条目，
//! 下次登录同一账号会覆盖它——比反过来（行指向不存在的密文）无害得多。
//!
//! # client_id 从哪来
//!
//! Device Flow 需要 OAuth App 的 client_id（公开值）。默认工厂从全局设置
//! [`CLIENT_ID_SETTING_KEY`] 读取；未配置时**快速失败**（`VALIDATION`，
//! hint 是设置键），不让用户在向导走完一半后撞上 GitHub 的错误。
//! 测试经 [`AccountService::with_factory`] 注入指向本地假服务的工厂。

use std::collections::HashMap;
use std::sync::Arc;

use crate::credentials::SharedStore;
use forgedesk_credentials::{
    system_clock, Clock, CredentialKind, CredentialRef, IndexedCredentialStore, MemoryBackend,
    MemoryIndex, Secret,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_provider::{
    poll_until_authorized, DeviceFlowStart, GitHubHttp, GitHubProvider, HostProvider, PollOptions,
    ProviderId, VerifiedAccount,
};
use forgedesk_storage::{AccountStore, Database, Scope, SettingsRepository};
use parking_lot::Mutex;
use secrecy::{ExposeSecret, SecretString};
use tokio_util::sync::CancellationToken;

/// GitHub OAuth App 的 client_id 存放的全局设置键。
///
/// 值是公开的（Device Flow 的 client_id 不是秘密），但注册 OAuth App
/// 归应用维护者，所以经设置注入而不是硬编码。
pub const CLIENT_ID_SETTING_KEY: &str = "provider.github.clientId";

/// 登录成功的账号。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// 稳定 id（重复登录沿用）。
    pub id: String,
    /// 平台标识（`github`；GitLab/Gitea 实现落地后扩充）。
    pub provider: String,
    /// 主机。
    pub host: String,
    /// 登录名。
    pub login: String,
    /// 头像地址。
    pub avatar_url: Option<String>,
    /// 令牌作用域。
    pub scopes: Vec<String>,
    /// 首次登录时间（Unix 毫秒）。
    pub created_at: Option<i64>,
}

/// 一次已启动的 Device Flow（`flow_id` 用于后续的 wait/cancel）。
///
/// [`DeviceFlowStart`] 含 `device_code`（秘密），**不可序列化**——
/// 它只活在后端会话表里，前端拿到的是命令层拆出的非秘密字段。
#[derive(Debug)]
pub struct StartedDeviceFlow {
    /// 后端会话 id。
    pub flow_id: String,
    /// Flow 启动数据（秘密持有端）。
    pub start: DeviceFlowStart,
}

struct FlowSession {
    host: String,
    start: DeviceFlowStart,
}

/// provider 工厂：`host → provider 实例`。生产环境按设置里的 client_id 构造；
/// 契约测试注入指向本地假服务的实例。
type ProviderFactory = Box<dyn Fn(&str) -> AppResult<GitHubProvider> + Send + Sync>;

/// 账号服务：登录编排 + 凭据/账号两处落地。
pub struct AccountService {
    database: Arc<Database>,
    credentials: SharedStore,
    flows: Mutex<HashMap<String, FlowSession>>,
    factory: ProviderFactory,
    clock: Clock,
}

impl std::fmt::Debug for AccountService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccountService")
            .field("flows", &self.flows.lock().len())
            .finish_non_exhaustive()
    }
}

impl AccountService {
    /// 默认构造：client_id 从全局设置 [`CLIENT_ID_SETTING_KEY`] 读取。
    pub fn new(database: Arc<Database>, credentials: SharedStore, http: GitHubHttp) -> Self {
        let factory_database = Arc::clone(&database);
        Self::with_factory(
            database,
            credentials,
            Box::new(move |host: &str| -> AppResult<GitHubProvider> {
                let client_id = SettingsRepository::new(factory_database.as_ref())
                    .get(&Scope::Global, CLIENT_ID_SETTING_KEY)?
                    .unwrap_or_default();
                if client_id.trim().is_empty() {
                    // hint 只放数据（设置键），建议性文案由前端按错误码补
                    return Err(AppError::new(
                        ErrorCode::Validation,
                        "GitHub OAuth client id is not configured",
                    )
                    .with_hint(CLIENT_ID_SETTING_KEY));
                }
                GitHubProvider::new(host, client_id, http.clone())
            }),
        )
    }

    /// 注入 provider 工厂（契约测试把端点指到本地假服务时用）。
    pub fn with_factory(
        database: Arc<Database>,
        credentials: SharedStore,
        factory: ProviderFactory,
    ) -> Self {
        Self {
            database,
            credentials,
            flows: Mutex::new(HashMap::new()),
            factory,
            clock: Box::new(system_clock),
        }
    }

    /// PAT 登录：校验令牌（`/user`），有效则落地。
    pub async fn login_with_pat(&self, host: &str, token: SecretString) -> AppResult<Account> {
        let provider = (self.factory)(host)?;
        let verified = provider.auth().verify_pat(token.clone()).await?;
        let host = provider.host().to_owned();
        self.persist(provider.id(), &host, verified, CredentialKind::Pat, token)
    }

    /// 启动 Device Flow：返回 UI 三步引导需要的非秘密数据，
    /// 秘密（device_code）留在后端会话表。
    pub async fn start_device_flow(
        &self,
        host: &str,
        scopes: Option<Vec<String>>,
    ) -> AppResult<StartedDeviceFlow> {
        let provider = (self.factory)(host)?;
        let requested: Vec<String> = match scopes {
            Some(list) if !list.is_empty() => list,
            // 用户没挑作用域 → 用 M4 全集；GitHub 会回传实际授予的集合
            _ => forgedesk_provider::DEFAULT_SCOPES
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
        };
        let requested: Vec<&str> = requested.iter().map(String::as_str).collect();
        let start = provider.auth().start_device_flow(&requested).await?;

        let flow_id = uuid::Uuid::new_v4().to_string();
        self.flows.lock().insert(
            flow_id.clone(),
            FlowSession {
                host: provider.host().to_owned(),
                start: start.clone(),
            },
        );
        Ok(StartedDeviceFlow { flow_id, start })
    }

    /// 等待 Device Flow 完成（轮询直到授权/过期/取消）。
    ///
    /// 由命令层放进 `JobRunner` 的任务线程里跑：任务线程自建
    /// current-thread 运行时来执行这段异步逻辑（见 `commands::account`）。
    /// `cancel` 用 JobRunner 的取消令牌：`job_cancel` 即停止轮询。
    pub async fn wait_device_flow(
        &self,
        flow_id: &str,
        cancel: &CancellationToken,
    ) -> AppResult<Account> {
        // remove：一个会话只能等一次。再等同一个 flow_id 是前端 bug 或
        // 重放，按"不存在"处理比静默复用旧 device_code 安全
        let session = self
            .flows
            .lock()
            .remove(flow_id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "unknown device flow session"))?;
        let provider = (self.factory)(&session.host)?;

        let login = poll_until_authorized(
            provider.auth(),
            &session.start,
            cancel,
            PollOptions::default(),
        )
        .await?;
        // 授权令牌还不知道"是谁"：再查一次 /user 拿登录名与作用域
        let verified = provider.auth().verify_pat(login.token.clone()).await?;
        self.persist(
            provider.id(),
            &session.host,
            verified,
            CredentialKind::Oauth,
            login.token,
        )
    }

    /// 全部账号（设置页的账号列表）。
    pub fn list(&self) -> AppResult<Vec<Account>> {
        Ok(AccountStore::new(&self.database)
            .list()?
            .into_iter()
            .map(Account::from_record)
            .collect())
    }

    /// 删除账号：先删凭据库条目，再删 `accounts` 行（顺序见模块文档）。
    pub fn remove(&self, id: &str) -> AppResult<()> {
        let store = AccountStore::new(&self.database);
        let record = store
            .get(id)?
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "unknown account"))?;

        // keyring 条目删不掉时终止：留着一行指向"删不掉的密文"的记录
        // 会让用户以为删干净了，而凭据还躺在系统凭据库里
        let credential_ref = CredentialRef::parse(&record.credential_ref).ok_or_else(|| {
            AppError::new(
                ErrorCode::Internal,
                "stored account has a malformed credential reference",
            )
        })?;
        self.credentials
            .current()
            .delete(&credential_ref)
            .map_err(|error| error.to_app_error())?;
        store.delete(id)
    }

    /// 两处落地（顺序见模块文档）。登录名作为 keyring 的 login 段。
    fn persist(
        &self,
        provider: ProviderId,
        host: &str,
        verified: VerifiedAccount,
        kind: CredentialKind,
        token: SecretString,
    ) -> AppResult<Account> {
        let store = AccountStore::new(&self.database);
        let existing = store.find_by_login(provider.as_str(), host, &verified.login)?;
        let id = existing
            .as_ref()
            .map(|record| record.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let created_at = existing
            .and_then(|record| record.created_at)
            .unwrap_or_else(|| (self.clock)());

        let credential_ref = CredentialRef::new(provider.as_str(), host, &verified.login)
            .map_err(|error| error.to_app_error())?;
        self.credentials
            .current()
            .store(
                &credential_ref,
                kind,
                &Secret::new(token.expose_secret().to_owned()),
            )
            .map_err(|error| error.to_app_error())?;

        let record = forgedesk_storage::AccountRecord {
            id: id.clone(),
            provider: provider.as_str().to_owned(),
            host: host.to_owned(),
            login: verified.login.clone(),
            avatar_url: verified.avatar_url.clone(),
            scopes: Some(verified.scopes.join(",")),
            credential_ref: credential_ref.account(),
            created_at: Some(created_at),
        };
        store.upsert(&record)?;

        Ok(Account {
            id,
            provider: provider.as_str().to_owned(),
            host: host.to_owned(),
            login: verified.login,
            avatar_url: verified.avatar_url,
            scopes: verified.scopes,
            created_at: Some(created_at),
        })
    }
}

impl Account {
    pub(crate) fn from_record(record: forgedesk_storage::AccountRecord) -> Self {
        Self {
            id: record.id,
            provider: record.provider,
            host: record.host,
            login: record.login,
            avatar_url: record.avatar_url,
            scopes: record
                .scopes
                .unwrap_or_default()
                .split(',')
                .filter(|scope| !scope.trim().is_empty())
                .map(str::to_owned)
                .collect(),
            created_at: record.created_at,
        }
    }
}

/// 测试与"本次会话临时账号"用的内存凭据存储组合。
///
/// 公开原因：命令层冒烟测试与插件宿主（M6）都需要不碰真实 keyring 的
/// 存储组合；放这里是因为它与 [`AccountService`] 的落地路径共享同一抽象。
pub fn memory_credential_store() -> SharedStore {
    SharedStore::new(Arc::new(IndexedCredentialStore::new(
        MemoryBackend::new(),
        MemoryIndex::new(),
    )))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{memory_credential_store, AccountService, CLIENT_ID_SETTING_KEY};
    use crate::accounts::StartedDeviceFlow;
    use crate::credentials::SharedStore;
    use forgedesk_credentials::{CredentialKind, CredentialRef};
    use forgedesk_domain::ErrorCode;
    use forgedesk_provider::{GitHubHttp, GitHubProvider, HttpConfig};
    use forgedesk_storage::Database;
    use secrecy::SecretString;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn db() -> Arc<Database> {
        let database = Database::open_in_memory().unwrap();
        forgedesk_storage::migrate(&database).unwrap();
        Arc::new(database)
    }

    /// 构造指向本地假服务的被测对象，并把手柄一并交还：断言要用同一份
    /// 数据库与凭据存储（不是服务内部的私有字段）。
    struct Fixture {
        service: AccountService,
        keyring: SharedStore,
    }

    fn service_at(server: &MockServer) -> Fixture {
        let http = GitHubHttp::new(HttpConfig {
            backoff_base: Duration::from_millis(1),
            ..HttpConfig::default()
        })
        .unwrap();
        let uri = server.uri();
        let keyring = memory_credential_store();
        let database = db();
        let service = AccountService::with_factory(
            Arc::clone(&database),
            keyring.clone(),
            Box::new(move |_host| {
                Ok(GitHubProvider::with_endpoints(
                    "github.com",
                    "Iv1.testclientid",
                    http.clone(),
                    uri.clone(),
                    uri.clone(),
                ))
            }),
        );
        Fixture { service, keyring }
    }

    fn user_mock() -> Mock {
        Mock::given(method("GET")).and(path("/user")).respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-oauth-scopes", "repo, read:org")
                .set_body_json(serde_json::json!({
                    "login": "octocat",
                    "avatar_url": "https://avatars/u/1"
                })),
        )
    }

    #[tokio::test]
    async fn pat_login_persists_the_account_and_the_keyring_secret() {
        let server = MockServer::start().await;
        user_mock().mount(&server).await;
        let Fixture {
            service, keyring, ..
        } = service_at(&server);

        let account = service
            .login_with_pat("github.com", SecretString::from("ghp_good".to_owned()))
            .await
            .unwrap();

        assert_eq!(account.login, "octocat");
        assert_eq!(account.provider, "github");
        assert_eq!(account.scopes, vec!["repo", "read:org"]);

        // 凭据库里是 PAT 类型，且能读回明文（内存后端，非真实 keyring）
        let entries = keyring.current().list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, CredentialKind::Pat);
        let secret = keyring
            .current()
            .get(&CredentialRef::new("github", "github.com", "octocat").unwrap())
            .unwrap();
        assert_eq!(secret.expose(), "ghp_good");
    }

    #[tokio::test]
    async fn pat_login_with_a_rejected_token_leaves_no_trace() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"message":"Bad credentials"}"#),
            )
            .mount(&server)
            .await;
        let Fixture {
            service, keyring, ..
        } = service_at(&server);

        let error = service
            .login_with_pat("github.com", SecretString::from("ghp_bad".to_owned()))
            .await
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::AuthExpired);
        // 两处落地都必须干净：无效令牌不留半条记录
        assert!(service.list().unwrap().is_empty());
        assert!(keyring.current().list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn logging_in_twice_keeps_one_account_with_a_stable_id() {
        let server = MockServer::start().await;
        user_mock().mount(&server).await;
        let Fixture {
            service, keyring, ..
        } = service_at(&server);

        let first = service
            .login_with_pat("github.com", SecretString::from("ghp_one".to_owned()))
            .await
            .unwrap();
        let second = service
            .login_with_pat("github.com", SecretString::from("ghp_two".to_owned()))
            .await
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(
            first.created_at, second.created_at,
            "重复登录不刷新创建时间"
        );
        assert_eq!(service.list().unwrap().len(), 1);
        // 凭据被覆盖成最新令牌
        let secret = keyring
            .current()
            .get(&CredentialRef::new("github", "github.com", "octocat").unwrap())
            .unwrap();
        assert_eq!(secret.expose(), "ghp_two");
    }

    #[tokio::test]
    async fn device_flow_wait_persists_an_oauth_account() {
        let server = MockServer::start().await;
        // 1) 启动：POST /login/device/code
        Mock::given(method("POST"))
            .and(path("/login/device/code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_code": "dc_secret",
                "user_code": "WDJB-MJTK",
                "verification_uri": "https://github.com/login/device",
                "expires_in": 900,
                "interval": 1
            })))
            .mount(&server)
            .await;
        // 2) 轮询：POST /login/oauth/access_token → 直接授权成功
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "gho_flowtoken",
                "scope": "repo,read:org"
            })))
            .mount(&server)
            .await;
        // 3) 用新令牌取账号
        user_mock().mount(&server).await;
        let Fixture {
            service, keyring, ..
        } = service_at(&server);

        let StartedDeviceFlow { flow_id, start } =
            service.start_device_flow("github.com", None).await.unwrap();
        // 前端能拿到的是非秘密引导字段
        assert_eq!(start.user_code, "WDJB-MJTK");
        assert_eq!(start.interval_secs, 1);

        let account = service
            .wait_device_flow(&flow_id, &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(account.login, "octocat");
        let entries = keyring.current().list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].kind,
            CredentialKind::Oauth,
            "Device Flow 令牌按 OAuth 类型登记"
        );
        assert_eq!(
            keyring
                .current()
                .get(&CredentialRef::new("github", "github.com", "octocat").unwrap())
                .unwrap()
                .expose(),
            "gho_flowtoken"
        );
    }

    #[tokio::test]
    async fn waiting_on_an_unknown_or_consumed_flow_id_is_not_found() {
        let server = MockServer::start().await;
        let Fixture { service, .. } = service_at(&server);

        let error = service
            .wait_device_flow("no-such-flow", &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn removing_an_account_clears_both_the_row_and_the_keyring_entry() {
        let server = MockServer::start().await;
        user_mock().mount(&server).await;
        let Fixture {
            service, keyring, ..
        } = service_at(&server);
        let account = service
            .login_with_pat("github.com", SecretString::from("ghp_good".to_owned()))
            .await
            .unwrap();

        service.remove(&account.id).unwrap();

        assert!(service.list().unwrap().is_empty());
        assert!(keyring.current().list().unwrap().is_empty());
        // 账号已不在：再次删除按"未知账号"报错（命令层转成明确的 404 语义）
        let error = service.remove(&account.id).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn the_default_factory_fails_fast_without_a_configured_client_id() {
        // 未配置 client_id：PAT 登录在构造 provider 那一步就失败，且不出网
        let http = GitHubHttp::new(HttpConfig::default()).unwrap();
        let service = AccountService::new(db(), memory_credential_store(), http);

        let error = service
            .login_with_pat("github.com", SecretString::from("ghp_x".to_owned()))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.hint.as_deref(), Some(CLIENT_ID_SETTING_KEY));
    }
}
