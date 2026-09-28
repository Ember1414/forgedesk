//! 凭据管理：系统 keyring、加密文件回退，以及"凭据存在哪里/能不能用"的事实来源。
//!
//! 归属里程碑：T2.7（凭据解析与认证失败诊断）；M4/T4.4 在其上加多账号模型。
//!
//! # 分层与职责
//!
//! ```text
//! store.rs            CredentialStore：store/get/delete/list（用例接口）
//!   ├─ index.rs       索引：我们存过哪些凭据（系统凭据库无法统一枚举）
//!   └─ backend.rs     CredentialBackend：密文本体的读写
//!        ├─ keyring_backend.rs  系统凭据库（+ probe 可用性探测）
//!        ├─ vault.rs            加密文件回退（Argon2id + AES-256-GCM）
//!        └─ MemoryBackend       内存（测试 / 本次会话临时凭据）
//! secret.rs           明文包装：Debug/Display 脱敏 + Drop 清零
//! model.rs            provider/host/login 与 keyring account 命名
//! error.rs            本层错误 → AppError（错误码与 hint 的映射）
//! ```
//!
//! # 红线 R8（凭据永不进日志）
//!
//! 本 crate 的所有公开类型都满足：`Debug` 不含明文/密文（见 [`Secret`]、[`Vault`]），
//! 错误文案里最多出现 keyring 的 account 名（`<provider>:<host>:<login>`），
//! 那是用户自己的账号标识而不是秘密。
//!
//! # 为什么不用系统凭据库的枚举能力
//!
//! Windows Credential Manager 能枚举、macOS Keychain 与 Linux Secret Service 的
//! 枚举语义各不相同，且需要在不同平台上写三套代码。索引（[`index`]）以我们自己的记录为准，
//! 密文本体仍在系统库里；索引丢失最多丢"列表"，不丢凭据。

#![forbid(unsafe_code)]

/// 本 crate 的实现模块（对外只通过下面的重导出使用）。
pub mod askpass;
pub mod backend;
/// 凭据层的错误类型与 IPC 映射。
pub mod error;
/// 凭据索引（文件 / 内存）。
pub mod index;
/// 系统凭据库后端与可用性探测。
pub mod keyring_backend;
/// 凭据标识模型（provider/host/login、account 命名、类型）。
pub mod model;
/// 明文凭据的内存包装（防泄漏）。
pub mod secret;
/// SSH 密钥盘点（不读私钥内容）。
pub mod ssh;
/// 用例层的凭据存储接口与"后端 + 索引"组合实现。
pub mod store;
/// 远端 URL 解析（取 host 与 provider）。
pub mod url;
/// 加密文件回退（Argon2id + AES-256-GCM）。
pub mod vault;

pub use askpass::{
    answer_for, classify_prompt, is_askpass_invocation, prompt_from_args, write_answer,
    AskpassPlan, AskpassPrompt, FLAG as ASKPASS_FLAG, SECRET_ENV as ASKPASS_SECRET_ENV,
    USERNAME_ENV as ASKPASS_USERNAME_ENV,
};
pub use backend::{BackendKind, CredentialBackend, MemoryBackend};
pub use error::CredentialsError;
pub use index::{CredentialIndex, FileIndex, IndexEntry, MemoryIndex};
pub use keyring_backend::{probe, probe_default, KeyringAvailability, KeyringBackend};
pub use model::{CredentialKind, CredentialMeta, CredentialRef, SERVICE_NAME};
pub use secret::Secret;
pub use ssh::{
    default_ssh_dir, parse_agent_listing, parse_public_key, scan_keys, AgentKey, AgentStatus,
    SshInventory, SshKey,
};
pub use store::{system_clock, Clock, CredentialStore, IndexedCredentialStore};
pub use url::{parse_remote_url, provider_for_host, RemoteEndpoint, RemoteScheme};
pub use vault::{Vault, VaultBackend, VaultParams};

/// "系统 keyring + 文件索引"的默认组合。
pub type KeyringStore = IndexedCredentialStore<KeyringBackend, FileIndex>;

/// "加密保险库 + 文件索引"的回退组合。
pub type VaultStore = IndexedCredentialStore<VaultBackend, FileIndex>;

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
