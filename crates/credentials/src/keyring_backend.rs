//! 系统凭据库后端（Windows Credential Manager / macOS Keychain / Linux Secret Service）。
//!
//! # Linux 为什么需要额外的运行时条件
//!
//! 本 crate 用 keyring 的 `sync-secret-service`（走 D-Bus 的 Secret Service，
//! 编译期依赖 libdbus，运行期需要 gnome-keyring / KWallet 之类的提供者）。
//! 无桌面会话的机器（CI、SSH、精简桌面）上**没有**这个提供者，
//! 此时任何写入都会失败——这正是 [`probe`] 存在的理由：与其让用户在"保存凭据"
//! 时看到一句平台错误，不如在设置页就告诉他"系统凭据库不可用，可以改用加密文件"。
//!
//! # 测试策略
//!
//! 单元测试**不碰**真实的系统凭据库（docs/CODING_STYLE.md §2.4：禁止在单测里
//! 访问用户主目录/系统状态）。这里只测纯逻辑；真机连通性用 `#[ignore]` 标记的
//! 用例手动跑（`cargo test -p forgedesk-credentials -- --ignored`），
//! 或者在装了凭据服务的 CI 机器上跑。

use crate::backend::{BackendKind, CredentialBackend};
use crate::error::CredentialsError;
use crate::model::SERVICE_NAME;
use crate::secret::Secret;

/// 系统凭据库后端。
#[derive(Debug, Clone)]
pub struct KeyringBackend {
    service: String,
}

impl Default for KeyringBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyringBackend {
    /// 用生产 service 名（[`SERVICE_NAME`]）构造。
    pub fn new() -> Self {
        Self {
            service: SERVICE_NAME.to_owned(),
        }
    }

    /// 指定 service（测试与将来多实例隔离用）。
    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// 读取一条（不经过索引；供探测与诊断用）。
    pub fn read(&self, account: &str) -> Result<Secret, CredentialsError> {
        <Self as CredentialBackend>::get(self, &self.service, account)
    }
}

/// 把 keyring 的错误收敛成一句"可诊断但不含秘密"的原因。
///
/// 为什么保留平台的原文：用户拿它去搜到的第一条结果就是正确的解决办法
/// （例如 `The name org.freedesktop.secrets was not provided by any .service files`）。
/// 我们自己编一句"密钥环不可用"反而丢掉了这条线索。
fn describe(error: &keyring::Error) -> String {
    let platform = std::env::consts::OS;
    format!("platform={platform}; {error}")
}

impl CredentialBackend for KeyringBackend {
    fn set(&self, service: &str, account: &str, secret: &Secret) -> Result<(), CredentialsError> {
        let entry = keyring::Entry::new(service, account)
            .map_err(|error| CredentialsError::KeyringUnavailable(describe(&error)))?;
        entry
            .set_password(secret.expose())
            .map_err(|error| CredentialsError::KeyringUnavailable(describe(&error)))
    }

    fn get(&self, service: &str, account: &str) -> Result<Secret, CredentialsError> {
        let entry = keyring::Entry::new(service, account)
            .map_err(|error| CredentialsError::KeyringUnavailable(describe(&error)))?;
        match entry.get_password() {
            Ok(value) => Ok(Secret::new(value)),
            // "没有这条"与"库不可用"必须分开：前者用户没登录过，后者是环境坏了
            Err(keyring::Error::NoEntry) => Err(CredentialsError::NotFound(account.to_owned())),
            Err(error) => Err(CredentialsError::KeyringUnavailable(describe(&error))),
        }
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), CredentialsError> {
        let entry = keyring::Entry::new(service, account)
            .map_err(|error| CredentialsError::KeyringUnavailable(describe(&error)))?;
        match entry.delete_credential() {
            // 幂等：条目本来就不在，删除也算成功
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(CredentialsError::KeyringUnavailable(describe(&error))),
        }
    }

    fn kind(&self) -> BackendKind {
        BackendKind::SystemKeyring
    }
}

/// 系统凭据库的可用性。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyringAvailability {
    /// 可读写。
    Available,
    /// 不可用；`reason` 是平台原因（可直接作为错误的 `hint` 数据展示）。
    Unavailable {
        /// 平台原因（含目标平台与系统原文）。
        reason: String,
    },
}

impl KeyringAvailability {
    /// 是否可用。
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    /// 不可用时的原因。
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Available => None,
            Self::Unavailable { reason } => Some(reason),
        }
    }
}

/// 探测账号名（**临时**，探测结束即删除）。
///
/// 带上进程号与时间戳：同一台机器上两个实例同时启动探测时，
/// 它们不会互相读到对方的哨兵（否则会得出"写进去了但读出来是别人的值"的假结论）。
pub fn probe_account() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("probe:{}:{nanos}", std::process::id())
}

/// 探测值：写进去再读出来比对，确认"能写也能读"。
const PROBE_VALUE: &str = "forgedesk-probe-ok";

/// 探测系统凭据库是否可读写（写-读-删一条哨兵）。
///
/// 为什么必须"写-读-删"而不是只看能否构造 `Entry`：`Entry::new` 只做参数校验，
/// 永远成功；而 Linux 上没有 Secret Service 时，真正的失败发生在 `set_password`。
pub fn probe(service: &str) -> KeyringAvailability {
    let account = probe_account();
    let entry = match keyring::Entry::new(service, &account) {
        Ok(entry) => entry,
        Err(error) => {
            return KeyringAvailability::Unavailable {
                reason: describe(&error),
            };
        }
    };

    if let Err(error) = entry.set_password(PROBE_VALUE) {
        return KeyringAvailability::Unavailable {
            reason: describe(&error),
        };
    }

    let verdict = match entry.get_password() {
        Ok(value) if value == PROBE_VALUE => KeyringAvailability::Available,
        Ok(_) => KeyringAvailability::Unavailable {
            reason:
                "platform=unknown; sentinel mismatch (another process wrote the same probe key)"
                    .to_owned(),
        },
        Err(error) => KeyringAvailability::Unavailable {
            reason: describe(&error),
        },
    };

    // 无论结果如何都要清理：探测不该在用户的凭据管理器里留下垃圾条目
    let _ = entry.delete_credential();
    verdict
}

/// 用生产 service 名探测。
pub fn probe_default() -> KeyringAvailability {
    probe(SERVICE_NAME)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn the_backend_reports_itself_as_the_system_keyring() {
        assert_eq!(KeyringBackend::new().kind(), BackendKind::SystemKeyring);
        assert_eq!(KeyringBackend::default().kind(), BackendKind::SystemKeyring);
    }

    #[test]
    fn the_keyring_backend_uses_the_documented_service_name() {
        // service 名写错等同于"让所有已保存凭据失效"，这里把它钉死
        assert_eq!(SERVICE_NAME, "org.forgedesk.app");
    }

    #[test]
    fn probe_accounts_are_unique_per_call_so_concurrent_probes_do_not_collide() {
        let first = probe_account();
        let second = probe_account();

        assert!(first.starts_with("probe:"));
        assert_ne!(first, second);
        // 探测条目不能长得像真实账号，否则会被 list() 的解析当成一条真凭据
        assert!(crate::model::CredentialRef::parse(&first).is_some());
    }

    #[test]
    fn availability_helpers_agree_with_the_variant() {
        assert!(KeyringAvailability::Available.is_available());
        assert_eq!(KeyringAvailability::Available.reason(), None);

        let unavailable = KeyringAvailability::Unavailable {
            reason: "platform=linux; dbus".to_owned(),
        };
        assert!(!unavailable.is_available());
        assert_eq!(unavailable.reason(), Some("platform=linux; dbus"));
    }

    /// 真机连通性：验证"这台机器的系统凭据库确实能存能取能删"。
    ///
    /// 手动跑：`cargo test -p forgedesk-credentials -- --ignored`
    #[test]
    #[ignore = "touches the real system credential store; run manually with --ignored"]
    fn the_real_credential_store_round_trips_a_probe_entry() {
        let backend = KeyringBackend::new();
        let account = probe_account();

        backend
            .set(SERVICE_NAME, &account, &Secret::new(PROBE_VALUE))
            .expect("set on the real store");
        assert_eq!(
            backend
                .get(SERVICE_NAME, &account)
                .expect("get from the real store")
                .expose(),
            PROBE_VALUE
        );
        backend.delete(SERVICE_NAME, &account).expect("delete");
        assert!(matches!(
            backend.get(SERVICE_NAME, &account),
            Err(CredentialsError::NotFound(_))
        ));
    }

    /// 真机探测：验证 [`probe`] 的结论与"直接读写"一致。
    #[test]
    #[ignore = "touches the real system credential store; run manually with --ignored"]
    fn probing_the_real_store_agrees_with_a_direct_write() {
        let availability = probe_default();
        let backend = KeyringBackend::new();
        let account = probe_account();
        let direct = backend.set(SERVICE_NAME, &account, &Secret::new(PROBE_VALUE));

        assert_eq!(
            availability.is_available(),
            direct.is_ok(),
            "probe={availability:?} direct={direct:?}"
        );

        let _ = backend.delete(SERVICE_NAME, &account);
    }
}
