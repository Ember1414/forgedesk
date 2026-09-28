//! 最底层的凭据读写接口，以及内存实现。
//!
//! # 为什么要多这一层
//!
//! 上层（索引、元数据、错误映射、`store/get/delete/list` 的语义）不该关心密文存在
//! 系统 keyring 还是加密文件里。把差异收敛到本 trait 之后：
//!
//! - 系统 keyring 与加密保险库只是两个实现；
//! - 测试可以注入内存实现，不需要碰真实的用户凭据库
//!   （docs/CODING_STYLE.md §2.4：禁止在单测里访问用户主目录）；
//! - 将来 T6.9 的平台适配层要换实现时，改动被限制在本 trait 之内。

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::error::CredentialsError;
use crate::secret::Secret;

/// 后端种类（日志与设置页展示用；不含任何密文）。
///
/// 可序列化：设置页要如实告诉用户"凭据存在系统凭据库还是加密文件里"，
/// 这是把安全性差异讲清楚的前提。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackendKind {
    /// 系统凭据库（Windows Credential Manager / macOS Keychain / Linux Secret Service）。
    SystemKeyring,
    /// 加密文件回退（Argon2id + AES-256-GCM）。
    EncryptedVault,
    /// 仅进程内存（测试与"本次会话临时凭据"）。
    Memory,
}

impl BackendKind {
    /// 稳定字符串（写日志、进设置项的取值）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SystemKeyring => "systemKeyring",
            Self::EncryptedVault => "encryptedVault",
            Self::Memory => "memory",
        }
    }
}

/// 按 `(service, account)` 存取一条密文。
///
/// 实现方**必须**保证：任何错误里都不含明文（见 [`CredentialsError`] 的说明）。
pub trait CredentialBackend: Send + Sync {
    /// 写入（覆盖已存在的同名条目）。
    fn set(&self, service: &str, account: &str, secret: &Secret) -> Result<(), CredentialsError>;

    /// 读取；不存在时返回 [`CredentialsError::NotFound`]。
    fn get(&self, service: &str, account: &str) -> Result<Secret, CredentialsError>;

    /// 删除；对不存在的条目**必须**返回 `Ok`（幂等）：
    /// 用户点"删除凭据"两次不该看到错误。
    fn delete(&self, service: &str, account: &str) -> Result<(), CredentialsError>;

    /// 自述种类。
    fn kind(&self) -> BackendKind;
}

/// 内存后端：进程内保存明文。
///
/// 只用于测试与"本次会话临时凭据"。**不要**把它当成"没有 keyring 时的回退方案"：
/// 明文在堆上、进程退出即丢失，用户重启应用会发现凭据没了，
/// 那是比提示"系统凭据库不可用"更糟的体验。真正的回退是加密文件（[`crate::vault`]）。
#[derive(Debug, Default)]
pub struct MemoryBackend {
    entries: Mutex<BTreeMap<(String, String), String>>,
}

impl MemoryBackend {
    /// 新建空后端。
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialBackend for MemoryBackend {
    fn set(&self, service: &str, account: &str, secret: &Secret) -> Result<(), CredentialsError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CredentialsError::Io("backend lock poisoned".to_owned()))?;
        entries.insert(
            (service.to_owned(), account.to_owned()),
            secret.expose().to_owned(),
        );
        Ok(())
    }

    fn get(&self, service: &str, account: &str) -> Result<Secret, CredentialsError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| CredentialsError::Io("backend lock poisoned".to_owned()))?;
        entries
            .get(&(service.to_owned(), account.to_owned()))
            .map(Secret::new)
            .ok_or_else(|| CredentialsError::NotFound(account.to_owned()))
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), CredentialsError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CredentialsError::Io("backend lock poisoned".to_owned()))?;
        entries.remove(&(service.to_owned(), account.to_owned()));
        Ok(())
    }

    fn kind(&self) -> BackendKind {
        BackendKind::Memory
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_backend_round_trips_a_secret() {
        let backend = MemoryBackend::new();

        backend
            .set(
                "org.forgedesk.app",
                "github:github.com:octocat",
                &Secret::new("tok"),
            )
            .expect("set");

        assert_eq!(
            backend
                .get("org.forgedesk.app", "github:github.com:octocat")
                .expect("get")
                .expose(),
            "tok"
        );
    }

    #[test]
    fn entries_are_scoped_by_service_so_two_apps_do_not_collide() {
        let backend = MemoryBackend::new();
        backend.set("a", "k", &Secret::new("first")).expect("set");
        backend.set("b", "k", &Secret::new("second")).expect("set");

        assert_eq!(backend.get("a", "k").expect("get").expose(), "first");
        assert_eq!(backend.get("b", "k").expect("get").expose(), "second");
    }

    #[test]
    fn reading_an_unknown_entry_reports_not_found_with_the_account() {
        let backend = MemoryBackend::new();

        match backend.get("org.forgedesk.app", "github:github.com:octocat") {
            Err(CredentialsError::NotFound(account)) => {
                assert_eq!(account, "github:github.com:octocat");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn deleting_is_idempotent_so_double_clicking_delete_is_not_an_error() {
        let backend = MemoryBackend::new();
        backend.set("s", "k", &Secret::new("v")).expect("set");

        backend.delete("s", "k").expect("first delete");
        backend.delete("s", "k").expect("second delete");

        assert!(backend.get("s", "k").is_err());
    }

    #[test]
    fn overwriting_replaces_the_previous_value() {
        let backend = MemoryBackend::new();
        backend.set("s", "k", &Secret::new("old")).expect("set");
        backend.set("s", "k", &Secret::new("new")).expect("set");

        assert_eq!(backend.get("s", "k").expect("get").expose(), "new");
    }

    #[test]
    fn backend_kinds_are_reported_for_diagnostics() {
        assert_eq!(MemoryBackend::new().kind(), BackendKind::Memory);
        assert_eq!(BackendKind::SystemKeyring.as_str(), "systemKeyring");
        assert_eq!(BackendKind::EncryptedVault.as_str(), "encryptedVault");
    }
}
