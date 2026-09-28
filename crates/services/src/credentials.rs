//! 凭据编排（T2.7）：把"存过什么凭据"翻译成"这次网络操作要带什么"。
//!
//! # 为什么需要这一层
//!
//! `forgedesk-credentials` 只回答"某条凭据是什么"，引擎只接受"用哪个程序、
//! 带哪些环境变量"。中间的判断全在这里：
//!
//! - 这个远端**要不要**凭据（本地路径不要、SSH 走密钥不要）；
//! - 要哪个凭据（按 host 找；URL 里带用户名时优先匹配它）；
//! - 没有存过怎么办（**不**注入，让 git 报 `AUTH_REQUIRED`，界面再引导保存）；
//! - 连续失败几次就停下（防止把账号刷到锁定）。
//!
//! # 红线 R8
//!
//! 明文只在 [`forgedesk_credentials::Secret`] 与 [`NetworkAuth::env`] 里存在，
//! 且只注入到**这一次**网络操作拉起的进程。本模块不写日志、不进审计。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use forgedesk_credentials::{
    parse_remote_url, provider_for_host, AskpassPlan, BackendKind, CredentialKind, CredentialMeta,
    CredentialRef, CredentialStore, FileIndex, IndexedCredentialStore, KeyringBackend, Secret,
};
use forgedesk_domain::git::{Remote, RepoId};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_git_engine::process::NetworkAuth;

/// 连续认证失败达到这个次数后不再重试。
///
/// 为什么是 3：托管平台普遍在前几次失败后开始限流甚至临时锁定账号
/// （尤其是带 2FA 的账号）。桌面应用不该成为一个自动化的密码猜测器——
/// 用户在界面上连点三次"重试"就足以说明"这不是偶发的手滑"。
pub const MAX_CONSECUTIVE_AUTH_FAILURES: u32 = 3;

/// 凭据门：解析注入方案 + 记住连续失败次数。
pub struct CredentialGate {
    store: Arc<dyn CredentialStore>,
    /// askpass 程序（应用自身；见 `forgedesk-credentials::askpass`）。
    program: PathBuf,
    /// host → 连续失败次数。
    failures: Mutex<HashMap<String, u32>>,
}

impl std::fmt::Debug for CredentialGate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialGate")
            .field("program", &self.program)
            .field("backend", &self.store.backend_kind())
            .finish_non_exhaustive()
    }
}

impl CredentialGate {
    /// 组装门（`program` 是当前可执行文件路径：git 会用它作 askpass）。
    pub fn new(store: Arc<dyn CredentialStore>, program: impl Into<PathBuf>) -> Self {
        Self {
            store,
            program: program.into(),
            failures: Mutex::new(HashMap::new()),
        }
    }

    /// 用**应用自身**作为 askpass 程序组装门。
    ///
    /// 拿不到自身路径时返回 `None`：无法充当 askpass 程序时，"不做注入"
    /// 比"注入一个空路径"要诚实——后者会让 git 在需要凭据时以一个更费解的方式失败。
    pub fn with_app_askpass(store: Arc<dyn CredentialStore>) -> Option<Self> {
        let program = std::env::current_exe().ok()?;
        Some(Self::new(store, program))
    }

    /// 凭据实际存在哪里（设置页展示）。
    pub fn backend_kind(&self) -> BackendKind {
        self.store.backend_kind()
    }

    /// 为某个远端 URL 构造注入方案。
    ///
    /// 三种"不注入"的情形都是刻意为之：
    /// - 本地路径 / `file://`：不需要凭据；
    /// - SSH：认证走密钥与 agent，往里塞令牌毫无作用；
    /// - 没存过：让 git 以 `AUTH_REQUIRED` 失败，界面据此引导用户保存凭据——
    ///   比我们发明一个"缺少凭据"的错误更贴近 git 的真实语义。
    pub fn auth_for(&self, url: &str, login_hint: Option<&str>) -> AppResult<NetworkAuth> {
        let Some(endpoint) = parse_remote_url(url) else {
            // 认不出的 URL 不猜主机：交给 git 自己报错
            return Ok(NetworkAuth::none());
        };
        if !endpoint.needs_credentials() || endpoint.is_ssh() {
            return Ok(NetworkAuth::none());
        }
        let Some(host) = endpoint.credential_host() else {
            return Ok(NetworkAuth::none());
        };

        let hint = login_hint.or(endpoint.user.as_deref());
        let Some(meta) = self.find(&host, hint)? else {
            return Ok(NetworkAuth::none());
        };

        let secret = self
            .store
            .get(&meta.key)
            .map_err(|error| error.to_app_error())?;
        let Some(plan) = AskpassPlan::new(&self.program, meta.key.login.clone(), &secret) else {
            // 空令牌：等同于"没有可用的凭据"
            return Ok(NetworkAuth::none());
        };
        Ok(NetworkAuth::askpass(plan.program(), plan.env()))
    }

    /// 按 host 找凭据（同 host 多账号时：`login_hint` 优先，否则取排序后的第一条）。
    fn find(&self, host: &str, login_hint: Option<&str>) -> AppResult<Option<CredentialMeta>> {
        let metas = self.store.list().map_err(|error| error.to_app_error())?;
        let mut candidates: Vec<CredentialMeta> = metas
            .into_iter()
            .filter(|meta| meta.key.host.eq_ignore_ascii_case(host))
            .collect();
        if candidates.is_empty() {
            return Ok(None);
        }
        if let Some(login) = login_hint {
            if let Some(position) = candidates.iter().position(|meta| meta.key.login == login) {
                return Ok(Some(candidates.swap_remove(position)));
            }
        }
        // `list()` 已按 account 排序，这里取第一条即可（行为稳定、可解释）
        Ok(Some(candidates.remove(0)))
    }

    /// 记一次连续失败，返回当前次数。
    pub fn note_auth_failure(&self, host: &str) -> u32 {
        match self.failures.lock() {
            Ok(mut failures) => {
                let counter = failures.entry(host.to_owned()).or_insert(0);
                *counter = counter.saturating_add(1);
                *counter
            }
            // 锁中毒（别处 panic）：退化成"不计数"，不能让错误处理路径再失败
            Err(_) => 0,
        }
    }

    /// 一次成功清空计数——"连续"的字面含义。
    pub fn note_success(&self, host: &str) {
        if let Ok(mut failures) = self.failures.lock() {
            failures.remove(host);
        }
    }

    /// 该 host 是否已用尽重试机会。
    pub fn is_exhausted(&self, host: &str) -> bool {
        self.failures
            .lock()
            .map(|failures| failures.get(host).copied().unwrap_or(0))
            .unwrap_or(0)
            >= MAX_CONSECUTIVE_AUTH_FAILURES
    }

    /// 剩余可尝试次数（界面提示用）。
    pub fn remaining_attempts(&self, host: &str) -> u32 {
        let used = self
            .failures
            .lock()
            .map(|failures| failures.get(host).copied().unwrap_or(0))
            .unwrap_or(0);
        MAX_CONSECUTIVE_AUTH_FAILURES.saturating_sub(used)
    }

    /// 把一次认证失败转成给用户看的错误。
    ///
    /// 达到上限时**换掉** message 与 hint 的方向：继续原样重试不会成功，
    /// 用户必须去检查凭据；只说"认证失败"会让他一直点重试。
    pub fn describe_failure(&self, host: &str, error: AppError) -> AppError {
        if error.code != ErrorCode::AuthRequired && error.code != ErrorCode::AuthExpired {
            return error;
        }
        let attempts = self.note_auth_failure(host);
        if attempts < MAX_CONSECUTIVE_AUTH_FAILURES {
            return error.with_hint(host.to_owned());
        }
        AppError::new(
            error.code,
            format!("authentication failed {attempts} times in a row; not retrying automatically"),
        )
        .with_hint(host.to_owned())
        .with_detail(error.detail.clone().unwrap_or_default())
    }
}

/// 设置页要展示的凭据状态。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsStatus {
    /// 密文实际存在哪里。
    pub backend: BackendKind,
    /// 已保存的凭据数量。
    pub count: usize,
    /// 索引文件路径（keyring 不可用时用户需要知道回退文件在哪）。
    pub index_path: Option<String>,
}

/// 面向界面的凭据服务：列表、保存、删除、状态。
pub struct CredentialsService {
    store: Arc<dyn CredentialStore>,
    index_path: Option<PathBuf>,
}

impl std::fmt::Debug for CredentialsService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `AppState` 派生 Debug，因此这里必须可 Debug；但**不能**打印存储内容
        formatter
            .debug_struct("CredentialsService")
            .field("backend", &self.store.backend_kind())
            .field("index_path", &self.index_path)
            .finish()
    }
}

impl CredentialsService {
    /// 用系统 keyring + 文件索引构造（生产路径）。
    pub fn keyring(index_path: impl Into<PathBuf>) -> Self {
        let index_path = index_path.into();
        let store = IndexedCredentialStore::new(KeyringBackend::new(), FileIndex::at(&index_path));
        Self {
            store: Arc::new(store),
            index_path: Some(index_path),
        }
    }

    /// 用给定的存储构造（测试注入内存实现）。
    pub fn with_store(store: Arc<dyn CredentialStore>) -> Self {
        Self {
            store,
            index_path: None,
        }
    }

    /// 底层存储（供 [`CredentialGate`] 共享同一份索引）。
    pub fn store(&self) -> Arc<dyn CredentialStore> {
        Arc::clone(&self.store)
    }

    /// 保存（覆盖同引用的旧值）。
    pub fn save(
        &self,
        provider: &str,
        host: &str,
        login: &str,
        kind: CredentialKind,
        secret: &Secret,
    ) -> AppResult<CredentialMeta> {
        let key =
            CredentialRef::new(provider, host, login).map_err(|error| error.to_app_error())?;
        self.store
            .store(&key, kind, secret)
            .map_err(|error| error.to_app_error())?;
        Ok(CredentialMeta {
            key,
            kind,
            created_at_ms: forgedesk_credentials::system_clock(),
        })
    }

    /// 删除（幂等）。
    pub fn delete(&self, provider: &str, host: &str, login: &str) -> AppResult<()> {
        let key =
            CredentialRef::new(provider, host, login).map_err(|error| error.to_app_error())?;
        self.store
            .delete(&key)
            .map_err(|error| error.to_app_error())
    }

    /// 列出已保存的凭据（不含密文）。
    pub fn list(&self) -> AppResult<Vec<CredentialMeta>> {
        self.store.list().map_err(|error| error.to_app_error())
    }

    /// 状态（后端的可用性由 T2.7 的探测命令单独给出）。
    pub fn status(&self) -> AppResult<CredentialsStatus> {
        let count = self.list()?.len();
        Ok(CredentialsStatus {
            backend: self.store.backend_kind(),
            count,
            index_path: self
                .index_path
                .as_ref()
                .map(|path| path.display().to_string()),
        })
    }
}

/// 远端名 → URL（找不到时返回 `None`）。
pub fn remote_url(remotes: &[Remote], remote: Option<&str>) -> Option<String> {
    let name = remote.unwrap_or("origin");
    remotes
        .iter()
        .find(|candidate| candidate.name == name)
        .map(|candidate| candidate.fetch_url.clone())
}

/// 当前分支的上游远端名（`origin/main` → `origin`）。
///
/// 为什么不能直接假定 `origin`：远端可以被重命名，也可能叫 `upstream`。
/// 用**分支自己的上游**才能与 git 实际会去联系的那个远端一致——
/// 而这正是"该用哪条凭据"的答案。取不到时返回 `None`（调用方退回 `origin`）。
pub fn tracking_remote(engines: &GitEngines, workdir: &Path) -> Option<String> {
    let branches = engines.read().branch_list(&RepoId::new(workdir)).ok()?;
    let head = branches.into_iter().find(|branch| branch.is_head)?;
    let upstream = head.upstream?;
    let (remote, _branch) = upstream.split_once('/')?;
    Some(remote.to_owned())
}

/// 本次操作**实际会联系**的远端 URL。
///
/// 顺序与 git 一致：显式指定的远端 → 当前分支的上游远端 → `origin` 兜底
/// （新建分支还没有上游时，`git push -u` 与 `fetch` 的默认目标都是 `origin`）。
pub fn resolve_remote_url(
    engines: &GitEngines,
    workdir: &Path,
    remote: Option<&str>,
) -> AppResult<Option<String>> {
    let name = match remote {
        Some(name) => name.to_owned(),
        None => tracking_remote(engines, workdir).unwrap_or_else(|| "origin".to_owned()),
    };
    let remotes = read_remotes(engines, workdir)?;
    Ok(remote_url(&remotes, Some(&name)))
}

/// 解析某个远端的主机（用于失败计数与错误提示）。
pub fn remote_host(url: &str) -> Option<String> {
    parse_remote_url(url).and_then(|endpoint| endpoint.credential_host())
}

/// 远端 URL 的 provider（凭据分组的键）。
pub fn remote_provider(url: &str) -> String {
    parse_remote_url(url)
        .and_then(|endpoint| endpoint.host)
        .map_or_else(
            || "generic".to_owned(),
            |host| provider_for_host(&host).to_owned(),
        )
}

/// 由服务层读取远端列表（同步路径与凭据解析共用）。
pub fn read_remotes(engines: &GitEngines, workdir: &Path) -> AppResult<Vec<Remote>> {
    engines
        .read()
        .remote_list(&forgedesk_domain::git::RepoId::new(workdir))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use forgedesk_credentials::{MemoryBackend, MemoryIndex};

    fn gate() -> CredentialGate {
        let store = IndexedCredentialStore::new(MemoryBackend::new(), MemoryIndex::new());
        CredentialGate::new(Arc::new(store), "/opt/forgedesk/forgedesk")
    }

    fn save(gate: &CredentialGate, host: &str, login: &str, secret: &str) {
        gate.store
            .store(
                &CredentialRef::new("github", host, login).expect("valid"),
                CredentialKind::Pat,
                &Secret::new(secret),
            )
            .expect("store");
    }

    #[test]
    fn an_https_remote_with_a_stored_credential_gets_an_askpass_plan() {
        let gate = gate();
        save(&gate, "github.com", "octocat", "ghp_x");

        let auth = gate
            .auth_for("https://github.com/octocat/repo.git", None)
            .expect("auth");

        assert_eq!(
            auth.askpass_program.as_deref(),
            Some(Path::new("/opt/forgedesk/forgedesk"))
        );
        let env: HashMap<String, String> = auth.env.into_iter().collect();
        assert_eq!(
            env.get(forgedesk_credentials::ASKPASS_USERNAME_ENV)
                .map(String::as_str),
            Some("octocat")
        );
        assert_eq!(
            env.get(forgedesk_credentials::ASKPASS_SECRET_ENV)
                .map(String::as_str),
            Some("ghp_x")
        );
    }

    #[test]
    fn a_remote_without_stored_credentials_is_left_anonymous_so_git_reports_authentication_required(
    ) {
        let gate = gate();

        let auth = gate
            .auth_for("https://github.com/octocat/repo.git", None)
            .expect("auth");

        // 关键：不要发明"缺少凭据"的错误，让 git 用它的 AUTH_REQUIRED 说话
        assert!(auth.is_none());
    }

    #[test]
    fn ssh_and_local_remotes_never_receive_a_token() {
        let gate = gate();
        save(&gate, "github.com", "octocat", "ghp_x");

        for url in [
            "git@github.com:octocat/repo.git",
            "ssh://git@github.com/octocat/repo.git",
            "/srv/git/repo.git",
            "file:///srv/git/repo.git",
        ] {
            let auth = gate.auth_for(url, None).expect("auth");
            assert!(auth.is_none(), "{url} 不该被注入令牌");
        }
    }

    #[test]
    fn the_login_in_the_url_wins_when_one_host_has_several_accounts() {
        let gate = gate();
        save(&gate, "github.com", "alice", "token-alice");
        save(&gate, "github.com", "bob", "token-bob");

        let auth = gate
            .auth_for("https://bob@github.com/team/repo.git", None)
            .expect("auth");

        let env: HashMap<String, String> = auth.env.into_iter().collect();
        assert_eq!(
            env.get(forgedesk_credentials::ASKPASS_USERNAME_ENV)
                .map(String::as_str),
            Some("bob")
        );
    }

    #[test]
    fn a_host_with_a_port_is_matched_as_its_own_credential_key() {
        let gate = gate();
        save(&gate, "git.internal:8443", "alice", "token");

        let same_port = gate
            .auth_for("https://git.internal:8443/team/repo.git", None)
            .expect("auth");
        let other_port = gate
            .auth_for("https://git.internal:443/team/repo.git", None)
            .expect("auth");

        assert!(!same_port.is_none());
        assert!(other_port.is_none(), "不同端口的账号不能互相顶替");
    }

    #[test]
    fn an_unparsable_url_is_passed_through_to_git_instead_of_being_guessed() {
        let gate = gate();
        save(&gate, "github.com", "octocat", "ghp_x");

        assert!(gate
            .auth_for("gopher://github.com/x", None)
            .expect("auth")
            .is_none());
    }

    #[test]
    fn three_consecutive_failures_stop_the_retry_and_change_the_message() {
        let gate = gate();
        let error = AppError::new(ErrorCode::AuthRequired, "authentication is required");

        let first = gate.describe_failure("github.com", error.clone());
        assert!(first.message.contains("authentication is required"));
        assert_eq!(gate.note_auth_failure("github.com"), 2);

        let third = gate.describe_failure("github.com", error);
        assert!(
            third.message.contains("3 times in a row"),
            "第三次必须换成不再自动重试的说法，实际: {}",
            third.message
        );
        assert!(gate.is_exhausted("github.com"));
        assert_eq!(gate.remaining_attempts("github.com"), 0);
    }

    #[test]
    fn a_successful_operation_resets_the_failure_counter() {
        let gate = gate();
        let error = AppError::new(ErrorCode::AuthRequired, "authentication is required");

        gate.describe_failure("github.com", error.clone());
        gate.describe_failure("github.com", error);
        gate.note_success("github.com");

        assert_eq!(
            gate.remaining_attempts("github.com"),
            MAX_CONSECUTIVE_AUTH_FAILURES
        );
        assert!(!gate.is_exhausted("github.com"));
    }

    #[test]
    fn non_authentication_failures_are_not_counted_against_the_account() {
        let gate = gate();
        let network = AppError::new(ErrorCode::Network, "could not resolve host");

        let described = gate.describe_failure("github.com", network.clone());

        // 网络问题不该消耗"重试机会"：否则一次断网就把账号的额度用完了
        assert_eq!(described.message, network.message);
        assert_eq!(
            gate.remaining_attempts("github.com"),
            MAX_CONSECUTIVE_AUTH_FAILURES
        );
    }

    #[test]
    fn failures_are_counted_per_host_so_one_bad_host_does_not_block_another() {
        let gate = gate();
        let error = AppError::new(ErrorCode::AuthRequired, "authentication is required");

        for _ in 0..MAX_CONSECUTIVE_AUTH_FAILURES {
            gate.describe_failure("github.com", error.clone());
        }

        assert!(gate.is_exhausted("github.com"));
        assert!(!gate.is_exhausted("gitlab.com"));
    }

    #[test]
    fn the_service_round_trips_a_credential_and_reports_its_status() {
        let store = Arc::new(IndexedCredentialStore::new(
            MemoryBackend::new(),
            MemoryIndex::new(),
        ));
        let service = CredentialsService::with_store(store);

        let saved = service
            .save(
                "github",
                "github.com",
                "octocat",
                CredentialKind::Pat,
                &Secret::new("ghp_x"),
            )
            .expect("save");

        assert_eq!(saved.key.account(), "github:github.com:octocat");
        assert_eq!(service.list().expect("list").len(), 1);
        let status = service.status().expect("status");
        assert_eq!(status.count, 1);
        assert_eq!(status.backend, BackendKind::Memory);

        service
            .delete("github", "github.com", "octocat")
            .expect("delete");
        assert!(service.list().expect("list").is_empty());
    }

    #[test]
    fn an_empty_secret_is_refused_by_the_service_before_it_reaches_the_store() {
        let store = Arc::new(IndexedCredentialStore::new(
            MemoryBackend::new(),
            MemoryIndex::new(),
        ));
        let service = CredentialsService::with_store(store);

        let error = service
            .save(
                "github",
                "github.com",
                "octocat",
                CredentialKind::Pat,
                &Secret::new(""),
            )
            .expect_err("empty secret");

        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[test]
    fn a_remote_name_is_looked_up_verbatim_and_a_missing_one_is_not_guessed() {
        let remotes = vec![Remote {
            name: "upstream".to_owned(),
            fetch_url: "https://gitlab.com/team/repo.git".to_owned(),
            push_url: None,
            kind: forgedesk_domain::git::RemoteKind::Https,
        }];

        assert_eq!(
            remote_url(&remotes, Some("upstream")).as_deref(),
            Some("https://gitlab.com/team/repo.git")
        );
        // `origin` 兜底只在"这个远端真的存在"时才有意义：不存在就交给 git 报错，
        // 不要凭空拼一条 URL 去查凭据（那会把令牌交给不相干的主机）
        assert_eq!(remote_url(&remotes, Some("origin")), None);
        assert_eq!(remote_url(&remotes, Some("missing")), None);
        assert_eq!(
            remote_provider("https://gitlab.com/team/repo.git"),
            "gitlab"
        );
        assert_eq!(
            remote_host("https://gitlab.com/team/repo.git").as_deref(),
            Some("gitlab.com")
        );
    }
}
