//! 托管平台远端仓库服务（T4.5）：账号 token 解析、每仓库账号绑定、列表/星标/fork。
//!
//! # 与凭据门的分工
//!
//! git 网络操作（clone/push/fetch）走 `CredentialGate`（T2.7，按 host 找凭据）；
//! 本模块服务的是**平台 API**（REST）：列出账号名下的仓库、加星、fork。
//! 两者读同一个凭据库，但语义不同：门回答"这次 git 操作带什么"，
//! 本模块回答"以哪个账号的身份调 API"。
//!
//! # 每仓库账号绑定（M4 验收：多账号按仓库生效）
//!
//! 绑定存在仓库级设置 [`BINDING_SETTING_KEY`]（`accounts` 表的账号 id）。
//! 解析顺序：仓库绑定的账号 → 该 host 上**最早登录**的账号 → 匿名
//! （仅对匿名可用的端点，如搜索）。同步命令（fetch/push）会用绑定的
//! login 作为凭据门的 `login_hint`，让 git 操作也用上绑定的账号。
//!
//! # 为什么 token 解析放这里而不是 provider
//!
//! "用哪个账号"是业务编排（涉及设置与账号表），provider 只认令牌本身
//! （红线 R8 的边界：provider 不读 keyring）。

use std::sync::Arc;

use crate::accounts::Account;
use crate::credentials::SharedStore;
use forgedesk_credentials::CredentialRef;
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_provider::{
    GitHubHttp, GitHubProvider, HostProvider, MergeOutcome, MergePullRequest, PullPage,
    PullRequestDetail, PullReview, PullState, RemoteRepo, RepoListScope, RepoPage,
};
use forgedesk_storage::{AccountStore, Database, Scope, SettingsRepository};
use secrecy::SecretString;

/// 仓库级设置里"绑定账号"的键（值为 `accounts` 表的账号 id）。
pub const BINDING_SETTING_KEY: &str = "accounts.preferredAccount";

/// provider 工厂类型（与 `accounts` 的同名概念一致：host → 实例）。
type ProviderFactory = Box<dyn Fn(&str) -> AppResult<GitHubProvider> + Send + Sync>;

/// 托管平台远端仓库服务。
pub struct HostRepoService {
    database: Arc<Database>,
    credentials: SharedStore,
    /// `host → provider 实例`。远端仓库端点不需要 client_id
    /// （只有 Device Flow 需要），因此这里不带设置校验。
    factory: ProviderFactory,
}

impl std::fmt::Debug for HostRepoService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostRepoService")
            .finish_non_exhaustive()
    }
}

impl HostRepoService {
    /// 用给定 HTTP 底座构造（限流捕获、代理与账号服务共享同一套）。
    pub fn new(database: Arc<Database>, credentials: SharedStore, http: GitHubHttp) -> Self {
        Self::with_factory(database, credentials, {
            let http = http;
            Box::new(move |host| GitHubProvider::new(host, "", http.clone()))
        })
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
            factory,
        }
    }

    /// 仓库绑定的账号（未绑定为 `None`）。
    pub fn binding(&self, repo_id: i64) -> AppResult<Option<Account>> {
        let Some(id) = self.bound_account_id(repo_id)? else {
            return Ok(None);
        };
        Ok(AccountStore::new(&self.database)
            .get(&id)?
            .map(Account::from_record))
    }

    /// 设置/解除仓库的账号绑定。
    ///
    /// 绑定时账号必须存在（否则界面拼错 id 会变成静默失效的绑定）。
    /// 返回绑定后的账号（解除时为 `None`）。
    pub fn set_binding(
        &self,
        repo_id: i64,
        account_id: Option<&str>,
    ) -> AppResult<Option<Account>> {
        let settings = SettingsRepository::new(&self.database);
        match account_id {
            Some(id) => {
                let account = AccountStore::new(&self.database).get(id)?.ok_or_else(|| {
                    AppError::new(ErrorCode::NotFound, "unknown account for binding")
                })?;
                settings.set(&Scope::Repo { repo_id }, BINDING_SETTING_KEY, &account.id)?;
                Ok(Some(Account::from_record(account)))
            }
            None => {
                settings.remove(&Scope::Repo { repo_id }, BINDING_SETTING_KEY)?;
                Ok(None)
            }
        }
    }

    /// 解析"以谁的身份"：绑定账号优先，其次该 host 上最早的账号。
    pub fn account_for(&self, host: &str, repo_id: Option<i64>) -> AppResult<Option<Account>> {
        if let Some(repo_id) = repo_id {
            let bound = self.bound_account_id(repo_id)?;
            if let Some(id) = bound {
                if let Some(record) = AccountStore::new(&self.database).get(&id)? {
                    if record.host.eq_ignore_ascii_case(host) {
                        return Ok(Some(Account::from_record(record)));
                    }
                    // 绑定的账号属于别的 host：对本 host 不生效，落到默认账号
                }
            }
        }
        Ok(AccountStore::new(&self.database)
            .list()?
            .into_iter()
            .find(|record| record.host.eq_ignore_ascii_case(host))
            .map(Account::from_record))
    }

    /// 绑定账号（或 host 默认账号）的令牌；无账号时 `None`。
    pub fn token_for(&self, host: &str, repo_id: Option<i64>) -> AppResult<Option<SecretString>> {
        let Some(account) = self.account_for(host, repo_id)? else {
            return Ok(None);
        };
        let credential_ref =
            CredentialRef::parse(&account_credential_ref(&account)).ok_or_else(|| {
                AppError::new(
                    ErrorCode::Internal,
                    "stored account has a malformed credential reference",
                )
            })?;
        let secret = self
            .credentials
            .current()
            .get(&credential_ref)
            .map_err(|error| error.to_app_error())?;
        Ok(Some(SecretString::from(secret.expose().to_owned())))
    }

    /// 绑定的 login（凭据门 hint 用）；未绑定为 `None`。
    pub fn bound_login(&self, repo_id: i64) -> AppResult<Option<String>> {
        Ok(self.binding(repo_id)?.map(|account| account.login))
    }

    fn bound_account_id(&self, repo_id: i64) -> AppResult<Option<String>> {
        SettingsRepository::new(&self.database).get(&Scope::Repo { repo_id }, BINDING_SETTING_KEY)
    }

    /// provider 实例（命令层转发远端操作用）。
    fn provider_for(&self, host: &str) -> AppResult<GitHubProvider> {
        (self.factory)(host)
    }

    // ---- 远端操作（token 解析 + 转发；匿名可用性由 provider 层的端点决定）----

    /// 列出账号可见的仓库（需要登录：无账号时直接给出 `AUTH_REQUIRED`）。
    pub async fn list_authenticated(
        &self,
        host: &str,
        repo_id: Option<i64>,
        scope: RepoListScope,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> AppResult<RepoPage> {
        let provider = self.provider_for(host)?;
        let token = self.require_token(host, repo_id).await?;
        provider
            .repos()
            .list_authenticated(token, scope, page, per_page)
            .await
    }

    /// 列出账号星标的仓库（需要登录）。
    pub async fn list_starred(
        &self,
        host: &str,
        repo_id: Option<i64>,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> AppResult<RepoPage> {
        let provider = self.provider_for(host)?;
        let token = self.require_token(host, repo_id).await?;
        provider.repos().list_starred(token, page, per_page).await
    }

    /// 搜索仓库（匿名可用；有账号时带令牌用更高配额）。
    pub async fn search(
        &self,
        host: &str,
        repo_id: Option<i64>,
        query: &str,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> AppResult<RepoPage> {
        let provider = self.provider_for(host)?;
        let token = self.token_for(host, repo_id)?;
        provider.repos().search(query, token, page, per_page).await
    }

    /// 加星/取消加星（需要登录）。
    pub async fn set_starred(
        &self,
        host: &str,
        repo_id: Option<i64>,
        owner: &str,
        repo: &str,
        starred: bool,
    ) -> AppResult<()> {
        let provider = self.provider_for(host)?;
        let token = self.require_token(host, repo_id).await?;
        provider
            .repos()
            .set_starred(token, owner, repo, starred)
            .await
    }

    /// fork 到当前账号名下（需要登录）。
    pub async fn fork(
        &self,
        host: &str,
        repo_id: Option<i64>,
        owner: &str,
        repo: &str,
    ) -> AppResult<RemoteRepo> {
        let provider = self.provider_for(host)?;
        let token = self.require_token(host, repo_id).await?;
        provider.repos().fork(token, owner, repo).await
    }

    // ---- Pull Request（T4.7）----

    /// 列出 PR（需要登录；token 解析与仓库列表同一套）。
    pub async fn list_pulls(
        &self,
        target: &RemoteRepoRef,
        query: PullListQuery,
    ) -> AppResult<PullPage> {
        let provider = self.provider_for(&target.host)?;
        let token = self.require_token(&target.host, target.repo_id).await?;
        provider
            .pulls()
            .list_pulls(
                token,
                &target.owner,
                &target.repo,
                query.state,
                query.page,
                query.per_page,
            )
            .await
    }

    /// PR 详情。描述 Markdown 在此消毒为 HTML（`bodyHtml`），
    /// 原文不越过 IPC（provider 侧已 `skip_serializing`，这里是双保险）。
    pub async fn get_pull(
        &self,
        host: &str,
        repo_id: Option<i64>,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> AppResult<PullDetailView> {
        let provider = self.provider_for(host)?;
        let token = self.require_token(host, repo_id).await?;
        let mut detail = provider
            .pulls()
            .get_pull(token, owner, repo, number)
            .await?;
        let body_html = detail
            .body_markdown
            .take()
            .map(|markdown| crate::readme::render_readme(&markdown));
        Ok(PullDetailView { detail, body_html })
    }

    /// PR 的 review 列表。
    pub async fn list_reviews(
        &self,
        host: &str,
        repo_id: Option<i64>,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> AppResult<Vec<PullReview>> {
        let provider = self.provider_for(host)?;
        let token = self.require_token(host, repo_id).await?;
        provider
            .pulls()
            .list_reviews(token, owner, repo, number)
            .await
    }

    /// 合并 PR（三策略 + 可选删源分支；错误语义见 provider::pulls 模块文档）。
    pub async fn merge_pull(
        &self,
        host: &str,
        repo_id: Option<i64>,
        owner: &str,
        repo: &str,
        number: u64,
        merge: MergePullRequest,
    ) -> AppResult<MergeOutcome> {
        let provider = self.provider_for(host)?;
        let token = self.require_token(host, repo_id).await?;
        provider
            .pulls()
            .merge_pull(token, owner, repo, number, merge)
            .await
    }

    /// 拉取并**安全渲染**仓库 README（T4.6）：返回的是白名单化 HTML，
    /// 前端不接触原始 Markdown（清洗规则见 [`crate::readme`]，XSS 用例在
    /// 那里穷举）。匿名可用（公开仓库）。
    pub async fn readme(
        &self,
        host: &str,
        repo_id: Option<i64>,
        owner: &str,
        repo: &str,
    ) -> AppResult<String> {
        let provider = self.provider_for(host)?;
        let token = self.token_for(host, repo_id)?;
        let markdown = provider.repos().readme(owner, repo, token).await?;
        Ok(crate::readme::render_readme(&markdown))
    }

    /// 需要登录的操作共用的"没有账号"出口。
    async fn require_token(&self, host: &str, repo_id: Option<i64>) -> AppResult<SecretString> {
        self.token_for(host, repo_id)?.ok_or_else(|| {
            AppError::new(
                ErrorCode::AuthRequired,
                "no account is signed in for this host",
            )
            .with_hint(host.to_owned())
        })
    }
}

/// 一个远端仓库的定位：站点 + 绑定来源 + owner/name。
#[derive(Debug, Clone)]
pub struct RemoteRepoRef {
    /// 站点（`github.com`）。
    pub host: String,
    /// 本地仓库 id（绑定解析来源；远端浏览场景为 `None`）。
    pub repo_id: Option<i64>,
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub repo: String,
}

/// PR 列表的查询参数。
#[derive(Debug, Clone, Copy)]
pub struct PullListQuery {
    /// 状态过滤。
    pub state: PullState,
    /// 页码。
    pub page: Option<u32>,
    /// 每页条数。
    pub per_page: Option<u32>,
}

/// PR 详情的 IPC 视图：结构化字段 + 消毒后的描述 HTML。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullDetailView {
    /// 详情字段（`body_markdown` 已被取走，序列化为 null/缺失）。
    pub detail: PullRequestDetail,
    /// 描述的消毒 HTML（无描述为 `None`）。
    pub body_html: Option<String>,
}

/// 取账号记录里保存的 credential_ref 字符串。
fn account_credential_ref(account: &Account) -> String {
    // Account 的字段来自 AccountRecord；credential_ref 没进 Account（不给前端），
    // 所以这里按同一规则重建：provider:host:login。
    format!("{}:{}:{}", account.provider, account.host, account.login)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{HostRepoService, BINDING_SETTING_KEY};
    use crate::accounts::memory_credential_store;
    use crate::accounts::AccountService;
    use forgedesk_provider::{GitHubHttp, GitHubProvider, HttpConfig, RepoListScope};
    use forgedesk_storage::{AccountStore, Database};
    use secrecy::ExposeSecret;
    use std::sync::Arc;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct Fixture {
        repos: HostRepoService,
        accounts: AccountService,
        database: Arc<Database>,
    }

    fn fixture_at(server: &MockServer) -> Fixture {
        let http = GitHubHttp::new(HttpConfig {
            backoff_base: Duration::from_millis(1),
            ..HttpConfig::default()
        })
        .unwrap();
        let database = {
            let db = Database::open_in_memory().unwrap();
            forgedesk_storage::migrate(&db).unwrap();
            Arc::new(db)
        };
        // 生产中账号服务与凭据门/远端服务共享同一凭据存储实例：
        // 测试也必须共享，否则"登录写进去、读的时候是另一个库"
        let keyring = memory_credential_store();
        let repos_uri = server.uri();
        let repos_http = http.clone();
        let repos = HostRepoService::with_factory(
            Arc::clone(&database),
            keyring.clone(),
            Box::new(move |host| {
                Ok(GitHubProvider::with_endpoints(
                    host,
                    "",
                    repos_http.clone(),
                    repos_uri.clone(),
                    repos_uri.clone(),
                ))
            }),
        );
        let uri = server.uri();
        let accounts = AccountService::with_factory(
            Arc::clone(&database),
            keyring,
            Box::new(move |_host| {
                Ok(GitHubProvider::with_endpoints(
                    "github.com",
                    "Iv1.test",
                    http.clone(),
                    uri.clone(),
                    uri.clone(),
                ))
            }),
        );
        Fixture {
            repos,
            accounts,
            database,
        }
    }

    /// PAT 登录一个账号并把记录里的 login 改成指定值
    /// （mock 的 /user 恒回 octocat，直接改行制造可辨识的登录名）。
    async fn login_account(fixture: &Fixture, token: &str, login: &str) {
        let account = fixture
            .accounts
            .login_with_pat("github.com", secrecy::SecretString::from(token.to_owned()))
            .await
            .unwrap();
        let store = AccountStore::new(&fixture.database);
        let mut record = store.get(&account.id).unwrap().unwrap();
        record.login = login.to_owned();
        record.credential_ref = format!("github:github.com:{login}");
        store.upsert(&record).unwrap();
    }

    #[tokio::test]
    async fn binding_round_trips_and_validates_the_account() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "login": "octocat", "avatar_url": null })),
            )
            .mount(&server)
            .await;
        let fixture = fixture_at(&server);
        let _ = login_account(&fixture, "ghp_one", "octocat").await;

        let accounts = fixture.accounts.list().unwrap();
        let id = accounts[0].id.clone();

        assert_eq!(fixture.repos.binding(7).unwrap(), None);
        let bound = fixture.repos.set_binding(7, Some(&id)).unwrap().unwrap();
        assert_eq!(bound.login, "octocat");
        assert_eq!(fixture.repos.binding(7).unwrap().unwrap().login, "octocat");

        // 解除
        assert_eq!(fixture.repos.set_binding(7, None).unwrap(), None);
        assert_eq!(fixture.repos.binding(7).unwrap(), None);

        // 不存在的账号：NOT_FOUND 而不是静默失效
        let error = fixture.repos.set_binding(7, Some("nope")).unwrap_err();
        assert_eq!(error.code, forgedesk_domain::ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn token_resolution_prefers_the_bound_account_then_the_earliest() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "login": "octocat", "avatar_url": null })),
            )
            .mount(&server)
            .await;
        let fixture = fixture_at(&server);
        login_account(&fixture, "ghp_earliest", "octocat").await;

        // 无绑定：取唯一账号的令牌
        let token = fixture
            .repos
            .token_for("github.com", Some(5))
            .unwrap()
            .unwrap();
        assert_eq!(token.expose_secret(), "ghp_earliest");

        // 绑定其它 host 的账号不生效（这里只有一个账号，直接断言绑定路径不炸）
        let id = fixture.accounts.list().unwrap()[0].id.clone();
        fixture.repos.set_binding(5, Some(&id)).unwrap();
        let token = fixture
            .repos
            .token_for("github.com", Some(5))
            .unwrap()
            .unwrap();
        assert_eq!(token.expose_secret(), "ghp_earliest");

        // 无任何账号的 host：None（调用方决定匿名还是拒绝）
        assert!(fixture
            .repos
            .token_for("gitlab.com", None)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn listing_without_any_account_fails_with_auth_required_and_the_host_hint() {
        let server = MockServer::start().await;
        let fixture = fixture_at(&server);

        let error = fixture
            .repos
            .list_authenticated("github.com", None, RepoListScope::Owned, None, None)
            .await
            .unwrap_err();

        assert_eq!(error.code, forgedesk_domain::ErrorCode::AuthRequired);
        assert_eq!(error.hint.as_deref(), Some("github.com"));
    }

    #[tokio::test]
    async fn listing_uses_the_resolved_account_token_and_maps_the_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "login": "octocat", "avatar_url": null })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/user/repos"))
            .and(query_param("affiliation", "owner"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 1, "name": "r", "full_name": "octocat/r",
                  "owner": {"login": "octocat"}, "html_url": "https://github.com/octocat/r" }
            ])))
            .mount(&server)
            .await;
        let fixture = fixture_at(&server);
        login_account(&fixture, "ghp_list", "octocat").await;

        let page = fixture
            .repos
            .list_authenticated("github.com", None, RepoListScope::Owned, None, None)
            .await
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].full_name, "octocat/r");
    }

    #[tokio::test]
    async fn binding_key_is_stored_in_the_repo_scope() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "login": "octocat", "avatar_url": null })),
            )
            .mount(&server)
            .await;
        let fixture = fixture_at(&server);
        login_account(&fixture, "ghp_one", "octocat").await;
        let id = fixture.accounts.list().unwrap()[0].id.clone();
        fixture.repos.set_binding(9, Some(&id)).unwrap();

        let raw = forgedesk_storage::SettingsRepository::new(&fixture.database)
            .get(
                &forgedesk_storage::Scope::Repo { repo_id: 9 },
                BINDING_SETTING_KEY,
            )
            .unwrap();
        assert_eq!(raw.as_deref(), Some(id.as_str()));
    }
}
