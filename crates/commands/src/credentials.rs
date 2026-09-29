//! 凭据相关的 Tauri 命令（T2.7）：保存 / 删除 / 列表 / 状态。
//!
//! # 为什么只有这四个，没有"登录"
//!
//! T2.7 提供的是**凭据通道**：把令牌存进系统凭据库、并让 git 在网络操作里用它。
//! OAuth 设备码登录（`auth_login_device_*`）与多账号模型属于 T4.4，
//! 它们会复用本模块的存储，不需要重写这一层。
//!
//! # 明文只经过一次边界
//!
//! `secret` 从前端传进来后立刻被 [`Secret`] 包装（Debug 脱敏、Drop 清零），
//! 写入系统凭据库；本模块**不**把它写进日志、审计或错误详情（红线 R8）。
//! 因此这里也**不允许**出现 `tracing::*` 打印 `secret` 的语句。

use forgedesk_credentials::{
    default_ssh_dir, parse_agent_listing, AgentStatus, BackendKind, CredentialKind, CredentialMeta,
    Secret, SshInventory,
};
use forgedesk_domain::AppResult;
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_services::{ssh_inventory, CredentialMode};
use forgedesk_storage::{Database, Scope, SettingsRepository};

use crate::state::AppState;

/// 凭据类型的 IPC 取值（与前端 DTO 一致：camelCase）。
#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialKindDto {
    /// 个人访问令牌。
    Pat,
    /// OAuth 令牌。
    Oauth,
    /// 用户名 + 密码。
    Password,
}

impl CredentialKindDto {
    fn to_domain(self) -> CredentialKind {
        match self {
            Self::Pat => CredentialKind::Pat,
            Self::Oauth => CredentialKind::Oauth,
            Self::Password => CredentialKind::Password,
        }
    }
}

/// 保存一条凭据（同引用覆盖）。
///
/// 参数校验在后端做（前端零信任，AGENTS §6）：provider / host / login 的合法性
/// 由 `CredentialRef::new` 收敛，空令牌被拒——空令牌存进去等于"存了一坨没用的东西"，
/// 用户下次会以为登录成功了。
#[tauri::command]
pub fn credentials_save(
    state: tauri::State<'_, AppState>,
    provider: String,
    host: String,
    login: String,
    kind: CredentialKindDto,
    secret: String,
) -> AppResult<CredentialMeta> {
    let service = state.credentials_service();
    let saved = service.save(
        &provider,
        &host,
        &login,
        kind.to_domain(),
        &Secret::new(secret),
    )?;

    // 保存新凭据即视为"账号状态已更新"：清掉该 host 的连续失败计数，
    // 否则用户刚改完令牌仍会被"连续失败 3 次"挡在门外。
    if let Some(gate) = state.credential_gate.as_deref() {
        gate.note_success(&saved.key.host);
    }
    Ok(saved)
}

/// 删除一条凭据（幂等）。
#[tauri::command]
pub fn credentials_delete(
    state: tauri::State<'_, AppState>,
    provider: String,
    host: String,
    login: String,
) -> AppResult<()> {
    state.credentials_service().delete(&provider, &host, &login)
}

/// 列出已保存的凭据（**不含**密文）。
#[tauri::command]
pub fn credentials_list(state: tauri::State<'_, AppState>) -> AppResult<Vec<CredentialMeta>> {
    state.credentials_service().list()
}

/// 本地 SSH 盘点（T2.7）：`~/.ssh` 里的密钥 + ssh-agent 状态。
///
/// # 它能回答什么、不能回答什么
///
/// 能：本地有哪些密钥、公私钥是否配对、`~/.ssh` 在哪、agent 里当前加载了哪几把
/// （以及 agent 在不在）。这是 `Permission denied (publickey)` 之后最常被问的那一半。
///
/// 不能：服务端是否接受这把公钥——那只能靠 [`credential_test_remote`] 实际连一次。
///
/// # 红线 R8
///
/// **不读私钥内容**：扫描只对私钥文件做 `is_file()`（存在性），只有公钥读首行
/// 取类型与注释。私钥路径会返回给界面（用户需要知道是哪个文件），但那不是秘密。
#[tauri::command]
pub fn credentials_ssh_inventory(state: tauri::State<'_, AppState>) -> AppResult<SshInventory> {
    let agent = match state.engines.write().probe_ssh_agent() {
        Ok(output) => parse_agent_listing(&output.stdout, output.exit_code),
        // "问不了 agent"（没装 ssh-add、超时）与"agent 没运行"必须分开：
        // 前者是我们查不到，后者是用户需要去启动 agent。理由里带上引擎给的原因，
        // 让界面能如实展示而不是笼统地说"SSH 有问题"。
        Err(error) => AgentStatus::Unknown {
            reason: error.detail.unwrap_or(error.message),
        },
    };
    ssh_inventory(default_ssh_dir().as_deref(), agent)
}

/// "测试连接"的结果。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteProbeDto {
    /// 远端引用条数（空仓库为 0）。
    pub refs: usize,
}

/// 测试连接：`git ls-remote`（只读，5s 超时）。
///
/// 入参二选一：`url`（手填地址）或 `repoId`（+ 可选 `remote`，缺省当前分支的上游
/// 远端 → `origin`）。失败时错误码来自 stderr 分类：SSH 主机指纹未信任、
/// SSH 公钥被拒、证书校验失败、代理失败各自成码，界面据此给出不同的建议。
#[tauri::command]
pub fn credential_test_remote(
    state: tauri::State<'_, AppState>,
    repo_id: Option<i64>,
    remote: Option<String>,
    url: Option<String>,
) -> AppResult<RemoteProbeDto> {
    let refs = state
        .sync_service()
        .probe_remote(repo_id, remote.as_deref(), url.as_deref())?;
    Ok(RemoteProbeDto { refs })
}

/// 凭据状态的 IPC 形状。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsStatusDto {
    /// 密文实际存在哪里（`systemKeyring` / `encryptedVault` / `memory`）。
    pub backend: BackendKind,
    /// 当前形态：`systemKeyring` / `vaultUnlocked` / `vaultLocked`。
    pub mode: CredentialMode,
    /// 已保存的凭据数量；保险库未解锁时为 `null`（**不是 0**）。
    pub count: Option<usize>,
    /// 索引文件路径（keyring 不可用时用户需要知道回退文件在哪）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_path: Option<String>,
    /// 加密保险库文件路径（已存在时）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
    /// 保险库文件是否已存在（界面据此决定展示"新建"还是"解锁"）。
    pub vault_exists: bool,
    /// 系统凭据库是否可用；不可用时给出平台原因（数据，不是建议）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyring_unavailable_reason: Option<String>,
}

/// 凭据状态：存在哪里、处于哪种形态、有多少条、系统凭据库能不能用。
///
/// 为什么要主动探测凭据库：Linux 上没有 Secret Service 时，用户要等**第一次保存**
/// 才知道不可用——那时他已经填完表单了。这里提前告诉他，并给出回退方向。
#[tauri::command]
pub fn credentials_status(state: tauri::State<'_, AppState>) -> AppResult<CredentialsStatusDto> {
    let service = state.credentials_service();
    let status = service.status()?;
    let availability = forgedesk_credentials::probe_default();

    Ok(CredentialsStatusDto {
        backend: status.backend,
        mode: status.mode,
        count: status.count,
        index_path: status.index_path,
        vault_path: status.vault_path,
        vault_exists: status.vault_exists,
        keyring_unavailable_reason: availability.reason().map(str::to_owned),
    })
}

/// 新建加密保险库（系统凭据库不可用时的回退方案）并切到它。
///
/// 口令只以 [`Secret`] 形式在内存里存在，落盘的是 Argon2id 派生的密钥加密后的密文；
/// 口令本身**不**保存——忘了口令等于忘了里面的凭据（这是加密存储的固有代价，界面必须说清）。
#[tauri::command]
pub fn credentials_vault_create(
    state: tauri::State<'_, AppState>,
    passphrase: String,
) -> AppResult<()> {
    let secret = Secret::new(passphrase);
    state.credentials_service().create_vault(&secret)?;
    remember_backend(&state, BACKEND_ENCRYPTED_VAULT)
}

/// 解锁已有加密保险库并切到它。
#[tauri::command]
pub fn credentials_vault_unlock(
    state: tauri::State<'_, AppState>,
    passphrase: String,
) -> AppResult<()> {
    let secret = Secret::new(passphrase);
    state.credentials_service().unlock_vault(&secret)?;
    remember_backend(&state, BACKEND_ENCRYPTED_VAULT)
}

/// 记住"用户选了哪个凭据后端"，供下次启动时直接进入该形态。
fn remember_backend(state: &AppState, backend: &str) -> AppResult<()> {
    SettingsRepository::new(&state.database).set(&Scope::Global, CREDENTIALS_BACKEND_KEY, backend)
}

/// 读"用户上次选择的凭据后端"。
///
/// 为什么放在命令层：启动时（`AppState` 还没建好）只有数据库可用，
/// 而"选择"是命令写进去的——读写必须用同一个键，因此定义也留在这里。
pub fn preferred_backend(database: &Database) -> Option<String> {
    SettingsRepository::new(database)
        .get(&Scope::Global, CREDENTIALS_BACKEND_KEY)
        .ok()
        .flatten()
}

/// 凭据后端的选择键（全局设置）。
pub const CREDENTIALS_BACKEND_KEY: &str = "credentials.backend";
/// 取值：系统凭据库。
pub const BACKEND_SYSTEM_KEYRING: &str = "systemKeyring";
/// 取值：加密文件（回退方案）。
pub const BACKEND_ENCRYPTED_VAULT: &str = "encryptedVault";

// 校验全部落在 `forgedesk_credentials::CredentialRef::new` 与存储层（空令牌 → VALIDATION），
// 本模块不做第二套规则：两套规则迟早会不一致，而不一致的那一套通常更松。
