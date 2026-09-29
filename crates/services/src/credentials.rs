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
use std::sync::{Arc, Mutex, RwLock};

use forgedesk_credentials::{
    parse_remote_url, provider_for_host, AgentStatus, AskpassPlan, BackendKind, CredentialKind,
    CredentialMeta, CredentialRef, CredentialStore, CredentialsError, FileIndex,
    IndexedCredentialStore, KeyringBackend, Secret, SshInventory, VaultBackend,
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

/// 可切换的凭据存储句柄。
///
/// # 为什么需要"可切换"
///
/// 系统凭据库不可用时，用户要能改用加密文件（红线 R8 的回退方案）；而这次切换发生在
/// **运行期**——用户输入保险库口令的那一刻。凭据门与凭据服务因此不能各自持有一个固定的
/// `Arc<dyn CredentialStore>`：它们必须看到同一个"当前存储"，否则会出现
/// "面板里保存成功、同步时却查不到"这类难以解释的现象。
#[derive(Clone)]
pub struct SharedStore {
    inner: Arc<RwLock<Arc<dyn CredentialStore>>>,
}

impl std::fmt::Debug for SharedStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SharedStore")
            .field("backend", &self.current().backend_kind())
            .finish()
    }
}

impl SharedStore {
    /// 包一个存储。
    pub fn new(store: Arc<dyn CredentialStore>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(store)),
        }
    }

    /// 当前存储（每次都取最新的：切换后立即生效）。
    pub fn current(&self) -> Arc<dyn CredentialStore> {
        match self.inner.read() {
            Ok(store) => Arc::clone(&store),
            // 锁中毒只可能来自别处 panic：退回一个拒绝所有操作的存储，
            // 而不是 panic 传染给调用者
            Err(_) => Arc::new(LockedVault),
        }
    }

    /// 换成另一个存储（切换后端时调用）。
    pub fn replace(&self, store: Arc<dyn CredentialStore>) {
        if let Ok(mut current) = self.inner.write() {
            *current = store;
        }
    }
}

/// 凭据存储当前处于哪种形态（设置页据此决定展示"添加表单"还是"解锁表单"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialMode {
    /// 系统凭据库（正常路径）。
    SystemKeyring,
    /// 加密保险库已解锁。
    VaultUnlocked,
    /// 加密保险库存在但未解锁：所有操作都要先输口令。
    VaultLocked,
}

/// 尚未解锁的加密保险库。
///
/// 为什么不用 `None` 表示"锁着"：界面上"还没有凭据"与"保险库锁着"是两个完全不同的
/// 状态——前者要引导添加，后者要引导解锁。混成一个，用户会以为自己的凭据丢了。
#[derive(Debug)]
struct LockedVault;

impl CredentialStore for LockedVault {
    fn store(
        &self,
        _key: &CredentialRef,
        _kind: CredentialKind,
        _secret: &Secret,
    ) -> Result<(), CredentialsError> {
        Err(CredentialsError::VaultLocked)
    }

    fn get(&self, _key: &CredentialRef) -> Result<Secret, CredentialsError> {
        Err(CredentialsError::VaultLocked)
    }

    fn delete(&self, _key: &CredentialRef) -> Result<(), CredentialsError> {
        Err(CredentialsError::VaultLocked)
    }

    fn list(&self) -> Result<Vec<CredentialMeta>, CredentialsError> {
        Err(CredentialsError::VaultLocked)
    }

    fn backend_kind(&self) -> BackendKind {
        BackendKind::EncryptedVault
    }
}

/// 凭据门：解析注入方案 + 记住连续失败次数。
pub struct CredentialGate {
    store: SharedStore,
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
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

impl CredentialGate {
    /// 组装门（`program` 是当前可执行文件路径：git 会用它作 askpass）。
    pub fn new(store: SharedStore, program: impl Into<PathBuf>) -> Self {
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
    pub fn with_app_askpass(store: SharedStore) -> Option<Self> {
        let program = std::env::current_exe().ok()?;
        Some(Self::new(store, program))
    }

    /// 凭据实际存在哪里（设置页展示）。
    pub fn backend_kind(&self) -> BackendKind {
        self.store.current().backend_kind()
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
            .current()
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
        let metas = self
            .store
            .current()
            .list()
            .map_err(|error| error.to_app_error())?;
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

/// 一次网络操作的凭据上下文：解析注入方案 + 认证失败的记账。
///
/// # 为什么把这三件事收在一处
///
/// "解析注入方案""记录一次认证失败""成功后清零"必须给出**同一个答案**。
/// 而网络操作分散在 clone / fetch / pull / push / 探活若干条路径上，各写一遍迟早
/// 会出现"某条路径忘了计数"或"忘了清零"——用户看到的是莫名其妙的锁死，
/// 或者明明刚改好凭据却仍被挡住。
pub struct CredentialContext<'a> {
    gate: Option<&'a CredentialGate>,
    /// 参与失败计数的 host（URL 认不出来时为 `None`）。
    host: Option<String>,
    auth: NetworkAuth,
}

impl std::fmt::Debug for CredentialContext<'_> {
    /// 手写而非 `derive`：这个类型带着注入方案（其环境变量里有明文）。
    ///
    /// 两个字段自身的 `Debug` 都已脱敏（[`NetworkAuth`] 只打印程序路径与变量**个数**，
    /// [`CredentialGate`] 只打印 askpass 程序与后端种类），因此这里可以安全地打印。
    /// 写出来是为了让"它能被打印"这件事**有据可查**，而不是靠读者去追两个 impl。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialContext")
            .field("host", &self.host)
            .field("auth", &self.auth)
            .finish()
    }
}

impl<'a> CredentialContext<'a> {
    /// 不注入任何凭据，也不记账（本地路径、SSH、解析不出远端时）。
    pub fn anonymous() -> Self {
        Self {
            gate: None,
            host: None,
            auth: NetworkAuth::none(),
        }
    }

    /// 按 URL 解析。`login_hint` 是 URL 里带的用户名（`https://user@host/…`）；
    /// 为空时凭据门会自己从 URL 里取。
    ///
    /// 已达"连续失败上限"时**直接拒绝**：继续发起网络操作只会把账号刷到锁定，
    /// 而用户需要的是"去检查凭据"这条明确的下一步。
    pub fn resolve(
        gate: Option<&'a CredentialGate>,
        url: &str,
        login_hint: Option<&str>,
    ) -> AppResult<Self> {
        let host = remote_host(url);
        let Some(gate) = gate else {
            // 没有凭据门：如实降级为匿名/SSH
            return Ok(Self {
                gate: None,
                host,
                auth: NetworkAuth::none(),
            });
        };

        if let Some(host) = host.as_deref() {
            if gate.is_exhausted(host) {
                return Err(AppError::new(
                    ErrorCode::AuthRequired,
                    "authentication has failed repeatedly; not retrying until the credential is updated",
                )
                .with_hint(host.to_owned()));
            }
        }

        Ok(Self {
            gate: Some(gate),
            host,
            auth: gate.auth_for(url, login_hint)?,
        })
    }

    /// 注入方案（没有保存过凭据时是"什么都不注入"）。
    pub fn auth(&self) -> &NetworkAuth {
        &self.auth
    }

    /// 失败时按认证类错误记账；达到上限后换成"不再自动重试"的说法。
    pub fn describe_failure(&self, error: AppError) -> AppError {
        match (self.gate, self.host.as_deref()) {
            (Some(gate), Some(host)) => gate.describe_failure(host, error),
            _ => error,
        }
    }

    /// 成功一次即清零该 host 的连续失败计数（"连续"的字面含义）。
    pub fn note_success(&self) {
        if let (Some(gate), Some(host)) = (self.gate, self.host.as_deref()) {
            gate.note_success(host);
        }
    }
}

/// 设置页要展示的凭据状态。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsStatus {
    /// 密文实际存在哪里。
    pub backend: BackendKind,
    /// 当前形态（keyring / 保险库已解锁 / 保险库未解锁）。
    pub mode: CredentialMode,
    /// 已保存的凭据数量。
    ///
    /// 保险库未解锁时是 `None` 而**不是 0**：报 0 会让用户以为凭据丢了，
    /// 而真实情况只是"还没输入口令"。
    pub count: Option<usize>,
    /// 索引文件路径（keyring 不可用时用户需要知道回退文件在哪）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_path: Option<String>,
    /// 加密保险库文件路径（已存在时）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
    /// 保险库文件是否已存在（界面据此决定展示"新建"还是"解锁"）。
    pub vault_exists: bool,
}

/// 面向界面的凭据服务：列表、保存、删除、状态，以及 keyring ↔ 加密保险库的切换。
pub struct CredentialsService {
    store: SharedStore,
    index_path: PathBuf,
    vault_path: PathBuf,
    mode: Mutex<CredentialMode>,
}

impl std::fmt::Debug for CredentialsService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `AppState` 派生 Debug，因此这里必须可 Debug；但**不能**打印存储内容
        formatter
            .debug_struct("CredentialsService")
            .field("store", &self.store)
            .field("index_path", &self.index_path)
            .field("mode", &self.mode.lock().map(|mode| *mode).ok())
            .finish()
    }
}

impl CredentialsService {
    /// 生产路径：系统凭据库 + 文件索引；保险库文件落在同一目录（同名前缀 `.vault`）。
    pub fn keyring(index_path: impl Into<PathBuf>) -> Self {
        let index_path = index_path.into();
        let vault_path = index_path.with_extension("vault");
        let store = IndexedCredentialStore::new(KeyringBackend::new(), FileIndex::at(&index_path));
        Self {
            store: SharedStore::new(Arc::new(store)),
            index_path,
            vault_path,
            mode: Mutex::new(CredentialMode::SystemKeyring),
        }
    }

    /// 测试用：注入任意存储（索引路径为空）。
    pub fn with_store(store: Arc<dyn CredentialStore>) -> Self {
        Self {
            store: SharedStore::new(store),
            index_path: PathBuf::new(),
            vault_path: PathBuf::new(),
            mode: Mutex::new(CredentialMode::SystemKeyring),
        }
    }

    /// 共享句柄（[`CredentialGate`] 与界面必须看到同一份"当前存储"）。
    pub fn shared(&self) -> SharedStore {
        self.store.clone()
    }

    /// 当前形态。
    pub fn mode(&self) -> CredentialMode {
        self.mode
            .lock()
            .map(|mode| *mode)
            .unwrap_or(CredentialMode::VaultLocked)
    }

    fn set_mode(&self, mode: CredentialMode) {
        if let Ok(mut current) = self.mode.lock() {
            *current = mode;
        }
    }

    /// 保险库文件是否已存在。
    pub fn vault_exists(&self) -> bool {
        self.vault_path.is_file()
    }

    /// 切到"未解锁的保险库"（启动时配置为加密文件时用）。
    ///
    /// 不在这里尝试解锁：口令只能来自用户，而"启动时弹一个口令框"是 T4.x 的
    /// 身份模型该决定的事。现在由设置页引导解锁。
    pub fn lock_vault(&self) {
        self.store.replace(Arc::new(LockedVault));
        self.set_mode(CredentialMode::VaultLocked);
    }

    /// 新建保险库并切换到它。
    ///
    /// 已存在保险库文件时**拒绝**：`Vault::create` 会覆盖文件，那等于把里面
    /// 已保存的凭据悄悄丢掉（用户以为在"新建"，实际在"清空"）。
    pub fn create_vault(&self, passphrase: &Secret) -> AppResult<()> {
        if self.vault_exists() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "an encrypted vault already exists for this data directory",
            )
            .with_hint(self.vault_path.display().to_string()));
        }
        let backend = VaultBackend::create(&self.vault_path, passphrase)
            .map_err(|error| error.to_app_error())?;
        self.activate_vault(backend);
        Ok(())
    }

    /// 解锁已有保险库并切换到它。
    pub fn unlock_vault(&self, passphrase: &Secret) -> AppResult<()> {
        let backend = VaultBackend::open(&self.vault_path, passphrase)
            .map_err(|error| error.to_app_error())?;
        self.activate_vault(backend);
        Ok(())
    }

    fn activate_vault(&self, backend: VaultBackend) {
        let store = IndexedCredentialStore::new(backend, FileIndex::at(&self.index_path));
        self.store.replace(Arc::new(store));
        self.set_mode(CredentialMode::VaultUnlocked);
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
            .current()
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
            .current()
            .delete(&key)
            .map_err(|error| error.to_app_error())
    }

    /// 列出已保存的凭据（不含密文）。
    pub fn list(&self) -> AppResult<Vec<CredentialMeta>> {
        self.store
            .current()
            .list()
            .map_err(|error| error.to_app_error())
    }

    /// 状态（系统凭据库的可用性由探测单独给出，见命令层）。
    pub fn status(&self) -> AppResult<CredentialsStatus> {
        let mode = self.mode();
        let store = self.store.current();
        let count = match store.list() {
            Ok(metas) => Some(metas.len()),
            // 锁着时**不**报 0：那是"凭据没了"的意思，而事实是"还没解锁"
            Err(_) if mode == CredentialMode::VaultLocked => None,
            Err(error) => return Err(error.to_app_error()),
        };
        Ok(CredentialsStatus {
            backend: store.backend_kind(),
            mode,
            count,
            index_path: (!self.index_path.as_os_str().is_empty())
                .then(|| self.index_path.display().to_string()),
            vault_path: self
                .vault_exists()
                .then(|| self.vault_path.display().to_string()),
            vault_exists: self.vault_exists(),
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

/// 组装本地 SSH 盘点（T2.7）。
///
/// # 为什么把 agent 状态作为参数传进来
///
/// "扫 `~/.ssh` 目录"与"问 ssh-agent"是两条独立的路径，失败方式也完全不同：
/// 目录不存在是**正常状态**（新机器还没配过 SSH），而 agent 没运行要引导用户去开。
/// 让调用方传入 [`AgentStatus`]，两件事就能各自测试，也不用在服务层
/// 引入"跑外部程序"的能力（那是引擎的职责，见 `GitEngine::probe_ssh_agent`）。
///
/// `directory` 为 `None`（拿不到 HOME）时同样返回空列表而不是错误：
/// 那不是故障，只是我们没有可盘点的位置。
pub fn ssh_inventory(directory: Option<&Path>, agent: AgentStatus) -> AppResult<SshInventory> {
    let keys = match directory {
        Some(dir) => forgedesk_credentials::scan_keys(dir).map_err(|error| error.to_app_error())?,
        None => Vec::new(),
    };
    Ok(SshInventory {
        directory: directory.map(|dir| dir.display().to_string()),
        keys,
        agent,
    })
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
    use forgedesk_credentials::{AgentKey, MemoryBackend, MemoryIndex};

    fn gate() -> CredentialGate {
        let store = IndexedCredentialStore::new(MemoryBackend::new(), MemoryIndex::new());
        CredentialGate::new(
            SharedStore::new(Arc::new(store)),
            "/opt/forgedesk/forgedesk",
        )
    }

    fn save(gate: &CredentialGate, host: &str, login: &str, secret: &str) {
        gate.store
            .current()
            .store(
                &CredentialRef::new("github", host, login).expect("valid"),
                CredentialKind::Pat,
                &Secret::new(secret),
            )
            .expect("store");
    }

    /// 临时索引路径（保险库与索引都在它的目录里）。
    fn temp_index(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("forgedesk-creds-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.join("credentials.index.json")
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
        assert_eq!(status.count, Some(1));
        assert_eq!(status.backend, BackendKind::Memory);
        assert_eq!(status.mode, CredentialMode::SystemKeyring);

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
    fn creating_a_vault_switches_the_store_and_keeps_working_after_a_reopen() {
        let index = temp_index("vault-create");
        let service = CredentialsService::keyring(&index);
        let passphrase = Secret::new("correct horse battery staple");

        assert!(!service.vault_exists());
        service.create_vault(&passphrase).expect("创建保险库");
        assert_eq!(service.mode(), CredentialMode::VaultUnlocked);

        service
            .save(
                "github",
                "github.com",
                "octocat",
                CredentialKind::Pat,
                &Secret::new("ghp_x"),
            )
            .expect("保存到保险库");
        assert_eq!(service.list().expect("list").len(), 1);

        // 换一个服务实例、走"解锁"这条路：证明密文真的落在文件里
        let reopened = CredentialsService::keyring(&index);
        reopened.lock_vault();
        assert_eq!(reopened.mode(), CredentialMode::VaultLocked);
        assert!(reopened.list().is_err(), "锁着时不该能读到凭据");

        reopened.unlock_vault(&passphrase).expect("解锁");
        assert_eq!(reopened.list().expect("list").len(), 1);

        let _ = std::fs::remove_dir_all(index.parent().expect("parent"));
    }

    #[test]
    fn a_locked_vault_reports_an_unknown_count_and_a_wrong_passphrase_stays_locked() {
        let index = temp_index("vault-locked");
        let service = CredentialsService::keyring(&index);
        service
            .create_vault(&Secret::new("right"))
            .expect("创建保险库");

        let reopened = CredentialsService::keyring(&index);
        reopened.lock_vault();

        // 锁着时 count 是 None（不是 0）：0 的意思是"没有凭据"，与事实不符
        let status = reopened.status().expect("status");
        assert_eq!(status.mode, CredentialMode::VaultLocked);
        assert_eq!(status.count, None);
        assert_eq!(status.backend, BackendKind::EncryptedVault);
        assert!(status.vault_exists);

        let error = reopened
            .unlock_vault(&Secret::new("wrong"))
            .expect_err("口令错必须失败");
        assert_eq!(error.code, ErrorCode::Storage);
        assert_eq!(reopened.mode(), CredentialMode::VaultLocked);

        let _ = std::fs::remove_dir_all(index.parent().expect("parent"));
    }

    #[test]
    fn creating_a_vault_that_already_exists_is_refused_instead_of_overwriting_it() {
        let index = temp_index("vault-exists");
        let service = CredentialsService::keyring(&index);
        service
            .create_vault(&Secret::new("pw"))
            .expect("创建保险库");

        let error = service
            .create_vault(&Secret::new("pw"))
            .expect_err("不得覆盖已有保险库");

        // 覆盖 = 把已保存的凭据悄悄清空，而用户以为自己只是"新建"了一次
        assert_eq!(error.code, ErrorCode::Validation);

        let _ = std::fs::remove_dir_all(index.parent().expect("parent"));
    }

    #[test]
    fn the_gate_sees_the_store_currently_in_use() {
        let index = temp_index("gate-shared");
        let service = CredentialsService::keyring(&index);
        let gate = CredentialGate::new(service.shared(), "/opt/forgedesk/forgedesk");

        service
            .create_vault(&Secret::new("pw"))
            .expect("创建保险库");
        service
            .save(
                "github",
                "github.com",
                "octocat",
                CredentialKind::Pat,
                &Secret::new("ghp_x"),
            )
            .expect("保存");

        // 切换后端之后，门必须立刻看到新存储（否则同步会去 keyring 里找一个不存在的令牌）
        let auth = gate
            .auth_for("https://github.com/octocat/repo.git", None)
            .expect("auth");
        assert_eq!(gate.backend_kind(), BackendKind::EncryptedVault);
        assert!(!auth.is_none());

        let _ = std::fs::remove_dir_all(index.parent().expect("parent"));
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

    #[test]
    fn a_credential_context_resolves_the_plan_and_records_failures() {
        let gate = gate();
        save(&gate, "github.com", "octocat", "ghp_x");

        let context =
            CredentialContext::resolve(Some(&gate), "https://github.com/octocat/repo.git", None)
                .expect("上下文");

        assert!(!context.auth().is_none());

        // 解析、记账、清零绑在一处：否则某条网络路径漏掉一步，用户会看到
        // "明明改好了凭据却仍被挡住"（漏清零）或"被刷到锁定"（漏计数）
        let error = AppError::new(ErrorCode::AuthRequired, "authentication is required");
        context.describe_failure(error);
        assert_eq!(
            gate.remaining_attempts("github.com"),
            MAX_CONSECUTIVE_AUTH_FAILURES - 1
        );

        context.note_success();
        assert_eq!(
            gate.remaining_attempts("github.com"),
            MAX_CONSECUTIVE_AUTH_FAILURES
        );
    }

    #[test]
    fn a_context_without_a_gate_is_anonymous_and_does_not_count_failures() {
        let context = CredentialContext::resolve(None, "https://github.com/octocat/repo.git", None)
            .expect("上下文");

        assert!(context.auth().is_none());

        // 没有门就没有上限：错误原样返回，运营者不会被"连续失败"挡住
        let error = AppError::new(ErrorCode::AuthRequired, "authentication is required");
        assert_eq!(
            context.describe_failure(error.clone()).message,
            error.message
        );
        context.note_success();
    }

    #[test]
    fn an_exhausted_host_is_refused_before_any_network_call() {
        let gate = gate();
        let error = AppError::new(ErrorCode::AuthRequired, "authentication is required");
        for _ in 0..MAX_CONSECUTIVE_AUTH_FAILURES {
            gate.describe_failure("github.com", error.clone());
        }

        let refused = CredentialContext::resolve(Some(&gate), "https://github.com/x/y.git", None)
            .expect_err("达到上限必须直接拒绝");

        assert_eq!(refused.code, ErrorCode::AuthRequired);
        assert!(refused.message.contains("failed repeatedly"), "{refused:?}");
        assert_eq!(refused.hint.as_deref(), Some("github.com"));
    }

    #[test]
    fn a_local_remote_gets_no_credentials_even_when_a_gate_is_present() {
        // `file://` 远端不需要凭据：注入令牌只会在握手时被拒
        let gate = gate();
        save(&gate, "github.com", "octocat", "ghp_x");

        let context = CredentialContext::resolve(Some(&gate), "file:///srv/git/repo.git", None)
            .expect("上下文");

        assert!(context.auth().is_none());
    }

    /// 临时 SSH 目录（用例自己造密钥文件；**不碰**用户真实的 `~/.ssh`）。
    fn temp_ssh_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("forgedesk-ssh-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn ssh_inventory_reports_the_keys_on_disk_and_the_agent_state_it_was_given() {
        let dir = temp_ssh_dir("inventory");
        std::fs::write(
            dir.join("id_ed25519.pub"),
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample octocat@example.com\n",
        )
        .expect("write pub");
        std::fs::write(dir.join("id_ed25519"), b"PRIVATE KEY NEVER READ").expect("write key");
        std::fs::write(dir.join("known_hosts"), b"github.com ssh-ed25519 AAAA").expect("write");

        let inventory = ssh_inventory(
            Some(&dir),
            AgentStatus::Ready(vec![AgentKey {
                bits: Some(256),
                fingerprint: "SHA256:abc".to_owned(),
                comment: Some("id_ed25519".to_owned()),
            }]),
        )
        .expect("盘点");

        let expected_dir = dir.display().to_string();
        assert_eq!(inventory.directory.as_deref(), Some(expected_dir.as_str()));
        assert_eq!(inventory.keys.len(), 1, "{:?}", inventory.keys);
        assert!(inventory.keys[0].is_pair());
        assert_eq!(inventory.keys[0].key_type.as_deref(), Some("ssh-ed25519"));
        // agent 状态原样带出：界面靠它区分"没有密钥"与"agent 没运行"
        assert!(matches!(&inventory.agent, AgentStatus::Ready(keys) if keys.len() == 1));
        assert!(!inventory.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_ssh_directory_is_an_empty_inventory_rather_than_a_failure() {
        // 新机器还没配过 SSH 是正常状态，不该弹错误；agent 没运行则如实带出
        let inventory = ssh_inventory(None, AgentStatus::NotRunning).expect("盘点");

        assert!(inventory.keys.is_empty());
        assert_eq!(inventory.directory, None);
        assert!(inventory.is_empty());
        assert_eq!(inventory.agent, AgentStatus::NotRunning);
    }
}
