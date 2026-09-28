//! 加密文件回退：没有系统凭据库时，把凭据加密存到本地文件。
//!
//! # 取舍（必须如实告诉用户）
//!
//! 系统凭据库不可用（典型场景：Linux 上没有 Secret Service、精简桌面、CI 容器）时，
//! 只有两条路：**不保存凭据**（每次都重新输入，等于不可用），或者
//! **用口令加密存到文件**。后者是一个真实的降级：
//!
//! | | 系统凭据库 | 加密文件 |
//! | --- | --- | --- |
//! | 密文位置 | 系统守护进程/内核保护 | 应用数据目录下的文件 |
//! | 威胁模型 | 同用户进程一般无法直接读取 | **能读到文件的人可以离线爆破口令** |
//! | 用户负担 | 无 | 每次启动要输入口令（除非本次会话已解锁） |
//!
//! 因此默认始终优先用系统凭据库；只有探测失败后才建议回退，并在设置页说明差异
//! （界面文案在 `errors.KEYRING_UNAVAILABLE.hint` 与设置页的 `sync`/`credentials` 分组）。
//!
//! # 格式
//!
//! ```text
//! 偏移  长度  内容
//! 0     6     magic "FDVLT\x01"（版本写在 magic 里：读到别的值就是"不是我们的文件"）
//! 6     16    salt（Argon2id 的盐，创建时随机；只在该文件重建时更换）
//! 22    4     m_cost（KiB，大端）
//! 26    4     t_cost（大端）
//! 30    4     p_cost（大端）
//! 34    12    nonce（每次写入重新随机）
//! 46    ..    AES-256-GCM 密文（含认证标签）
//! ```
//!
//! 头部（magic..nonce 之前）作为 **AAD** 参与认证：攻击者改不动 KDF 参数来悄悄
//! 降级我们的抗爆破强度——改了参数，解密就会失败。
//!
//! 载荷是 JSON：`{"version":1,"entries":{"<account>":"<base64 明文>"}}`。
//! 明文只在内存里存在（[`Secret`]），落盘前一定经过加密。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::backend::{BackendKind, CredentialBackend};
use crate::error::CredentialsError;
use crate::secret::Secret;

/// 文件头 magic（含格式版本）。
const MAGIC: &[u8; 6] = b"FDVLT\x01";
/// Argon2id 盐长度。
const SALT_LEN: usize = 16;
/// AES-GCM nonce 长度。
const NONCE_LEN: usize = 12;
/// 派生密钥长度（AES-256）。
const KEY_LEN: usize = 32;
/// 头部（AAD）长度：magic + salt + 三个参数 + nonce。
const HEADER_LEN: usize = MAGIC.len() + SALT_LEN + 4 + 4 + 4 + NONCE_LEN;
/// 载荷格式版本。
const PAYLOAD_VERSION: u32 = 1;

/// Argon2id 参数。
///
/// 默认值的选择：64 MiB 内存、3 次迭代。目标是"单次解锁对用户无感（几十毫秒），
/// 但对拿到文件的攻击者每次尝试都要付出 64 MiB 的内存带宽"。p=1 是因为
/// 桌面解锁是单次串行操作，并行度对用户没有收益，反而抬高内存峰值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultParams {
    /// 内存开销（KiB）。
    pub m_cost_kib: u32,
    /// 迭代次数。
    pub t_cost: u32,
    /// 并行度。
    pub p_cost: u32,
}

impl Default for VaultParams {
    fn default() -> Self {
        Self {
            m_cost_kib: 64 * 1024,
            t_cost: 3,
            p_cost: 1,
        }
    }
}

impl VaultParams {
    /// 参数下界（低于此值等于没有抗爆破强度；创建时据此拒绝）。
    pub const MIN_M_COST_KIB: u32 = 8 * 1024;
    /// 迭代次数下界。
    pub const MIN_T_COST: u32 = 1;
    /// 内存开销上界（512 MiB）。
    ///
    /// 为什么必须有**上界**：参数是从文件里读出来的。一个被改坏（或被恶意构造）的文件
    /// 只要把 m_cost 写成 4 GiB，应用就会在解锁时尝试分配 4 GiB —— 拒绝服务，
    /// 而用户看到的只是"打开设置页卡死了"。上界取 512 MiB：是默认值（64 MiB）的 8 倍，
    /// 给将来调参留足余量，又不至于让一次解锁吃掉整台机器。
    pub const MAX_M_COST_KIB: u32 = 512 * 1024;
    /// 迭代次数上界（防止"解锁要 10 分钟"）。
    pub const MAX_T_COST: u32 = 32;
    /// 并行度上界。
    pub const MAX_P_COST: u32 = 16;

    /// 校验参数区间。
    pub fn validate(&self) -> Result<(), CredentialsError> {
        if self.m_cost_kib < Self::MIN_M_COST_KIB || self.m_cost_kib > Self::MAX_M_COST_KIB {
            return Err(CredentialsError::VaultMalformed(format!(
                "m_cost {} KiB is outside [{}, {}] KiB",
                self.m_cost_kib,
                Self::MIN_M_COST_KIB,
                Self::MAX_M_COST_KIB
            )));
        }
        if self.t_cost < Self::MIN_T_COST || self.t_cost > Self::MAX_T_COST {
            return Err(CredentialsError::VaultMalformed(format!(
                "t_cost {} is outside [{}, {}]",
                self.t_cost,
                Self::MIN_T_COST,
                Self::MAX_T_COST
            )));
        }
        if self.p_cost == 0 || self.p_cost > Self::MAX_P_COST {
            return Err(CredentialsError::VaultMalformed(format!(
                "p_cost {} is outside [1, {}]",
                self.p_cost,
                Self::MAX_P_COST
            )));
        }
        Ok(())
    }
}

/// 载荷（加密后的 JSON）。
///
/// 名字带 `Vault` 前缀是为了不与 `aes_gcm::aead::Payload`（AAD 载体）同名——
/// 两者都会在本模块出现，同名会直接编译失败。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultPayload {
    version: u32,
    /// account → base64(明文)。
    entries: BTreeMap<String, String>,
}

/// 32 字节对称密钥（Drop 时清零）。
#[derive(Clone)]
struct VaultKey(Zeroizing<[u8; KEY_LEN]>);

impl VaultKey {
    fn derive(
        passphrase: &Secret,
        salt: &[u8; SALT_LEN],
        params: VaultParams,
    ) -> Result<Self, CredentialsError> {
        params.validate()?;
        let argon_params = Params::new(
            params.m_cost_kib,
            params.t_cost,
            params.p_cost,
            Some(KEY_LEN),
        )
        .map_err(|error| CredentialsError::VaultMalformed(format!("kdf params: {error}")))?;
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

        let mut key = Zeroizing::new([0_u8; KEY_LEN]);
        argon
            .hash_password_into(passphrase.expose().as_bytes(), salt, key.as_mut())
            .map_err(|error| CredentialsError::VaultMalformed(format!("kdf: {error}")))?;
        Ok(Self(key))
    }

    fn cipher(&self) -> Result<Aes256Gcm, CredentialsError> {
        Aes256Gcm::new_from_slice(self.0.as_ref())
            .map_err(|error| CredentialsError::VaultMalformed(format!("cipher: {error}")))
    }
}

impl std::fmt::Debug for VaultKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 密钥也不许进日志
        formatter.write_str("VaultKey(<redacted>)")
    }
}

/// 一个已解锁的加密保险库。
///
/// 生命周期：`open`/`create` 解锁 → 读改（每次改完立即落盘）→ Drop 时内存清零。
/// **不缓存解锁状态**到磁盘：没有"记住口令"这种事，那等于把保险库降级成明文。
pub struct Vault {
    path: PathBuf,
    key: VaultKey,
    salt: [u8; SALT_LEN],
    params: VaultParams,
    entries: BTreeMap<String, String>,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Vault")
            .field("path", &self.path)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl Vault {
    /// 保险库文件是否已存在（用于决定"让用户设置口令"还是"让用户输入口令"）。
    pub fn is_initialised(path: &Path) -> bool {
        path.is_file()
    }

    /// 新建（覆盖）保险库。
    pub fn create(path: &Path, passphrase: &Secret) -> Result<Self, CredentialsError> {
        Self::create_with(path, passphrase, VaultParams::default())
    }

    /// 用指定参数新建（测试用更小的开销；生产用默认）。
    pub fn create_with(
        path: &Path,
        passphrase: &Secret,
        params: VaultParams,
    ) -> Result<Self, CredentialsError> {
        if passphrase.is_empty() {
            return Err(CredentialsError::Invalid(
                "vault passphrase must not be empty".to_owned(),
            ));
        }
        let mut salt = [0_u8; SALT_LEN];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        let key = VaultKey::derive(passphrase, &salt, params)?;

        let mut vault = Self {
            path: path.to_path_buf(),
            key,
            salt,
            params,
            entries: BTreeMap::new(),
        };
        vault.flush()?;
        Ok(vault)
    }

    /// 解锁已存在的保险库。
    pub fn open(path: &Path, passphrase: &Secret) -> Result<Self, CredentialsError> {
        if !path.is_file() {
            return Err(CredentialsError::VaultMissing(path.display().to_string()));
        }
        let raw = fs::read(path)
            .map_err(|error| CredentialsError::Io(format!("read {}: {error}", path.display())))?;
        if raw.len() < HEADER_LEN {
            return Err(CredentialsError::VaultMalformed(format!(
                "{} is shorter than the fixed header ({HEADER_LEN} bytes)",
                path.display()
            )));
        }
        if &raw[..MAGIC.len()] != MAGIC {
            return Err(CredentialsError::VaultMalformed(format!(
                "{} is not a ForgeDesk credential vault",
                path.display()
            )));
        }

        let mut cursor = MAGIC.len();
        let mut salt = [0_u8; SALT_LEN];
        salt.copy_from_slice(&raw[cursor..cursor + SALT_LEN]);
        cursor += SALT_LEN;

        let read_u32 = |offset: usize| -> u32 {
            u32::from_be_bytes([
                raw[offset],
                raw[offset + 1],
                raw[offset + 2],
                raw[offset + 3],
            ])
        };
        let params = VaultParams {
            m_cost_kib: read_u32(cursor),
            t_cost: read_u32(cursor + 4),
            p_cost: read_u32(cursor + 8),
        };
        cursor += 12;

        let mut nonce_bytes = [0_u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(&raw[cursor..cursor + NONCE_LEN]);
        let header_end = cursor + NONCE_LEN;

        // 参数来自文件，必须先校验范围：否则一个被改成 m_cost = 4 GiB 的文件
        // 会让应用在解锁时直接吃光内存（拒绝服务）
        params.validate()?;

        let key = VaultKey::derive(passphrase, &salt, params)?;
        let cipher = key.cipher()?;
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &raw[header_end..],
                    // 头部参与认证：KDF 参数与盐都改不得
                    aad: &raw[..header_end],
                },
            )
            // 认证失败无法区分"口令错"与"文件被改"——如实按"打不开"报告
            .map_err(|_| CredentialsError::VaultLocked)?;

        let payload: VaultPayload = serde_json::from_slice(&plaintext)
            .map_err(|error| CredentialsError::VaultMalformed(format!("payload: {error}")))?;
        if payload.version != PAYLOAD_VERSION {
            return Err(CredentialsError::VaultMalformed(format!(
                "unsupported payload version {}",
                payload.version
            )));
        }

        Ok(Self {
            path: path.to_path_buf(),
            key,
            salt,
            params,
            entries: payload.entries,
        })
    }

    /// 保险库文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 条目数（诊断用，不暴露内容）。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 写入一条并立即落盘。
    pub fn set(&mut self, account: &str, secret: &Secret) -> Result<(), CredentialsError> {
        self.entries.insert(
            account.to_owned(),
            base64::engine::general_purpose::STANDARD.encode(secret.expose().as_bytes()),
        );
        self.flush()
    }

    /// 读取一条。
    pub fn get(&self, account: &str) -> Result<Secret, CredentialsError> {
        let encoded = self
            .entries
            .get(account)
            .ok_or_else(|| CredentialsError::NotFound(account.to_owned()))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| {
                CredentialsError::VaultMalformed(format!("entry {account}: {error}"))
            })?;
        let text = String::from_utf8(bytes)
            .map_err(|_| CredentialsError::VaultMalformed(format!("entry {account}: not UTF-8")))?;
        Ok(Secret::new(text))
    }

    /// 删除一条并立即落盘（不存在时也返回 Ok：幂等）。
    pub fn delete(&mut self, account: &str) -> Result<(), CredentialsError> {
        if self.entries.remove(account).is_none() {
            return Ok(());
        }
        self.flush()
    }

    /// 加密并原子替换文件。
    fn flush(&mut self) -> Result<(), CredentialsError> {
        let payload = serde_json::to_vec(&VaultPayload {
            version: PAYLOAD_VERSION,
            entries: self.entries.clone(),
        })
        .map_err(|error| CredentialsError::Io(format!("serialise vault: {error}")))?;

        // 每次写入换一个 nonce：同一把密钥重复使用同一个 nonce 会直接破坏 GCM 的安全性
        let mut nonce_bytes = [0_u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&self.salt);
        header.extend_from_slice(&self.params.m_cost_kib.to_be_bytes());
        header.extend_from_slice(&self.params.t_cost.to_be_bytes());
        header.extend_from_slice(&self.params.p_cost.to_be_bytes());
        header.extend_from_slice(&nonce_bytes);

        let cipher = self.key.cipher()?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: payload.as_slice(),
                    aad: header.as_slice(),
                },
            )
            .map_err(|_| CredentialsError::Io("vault encryption failed".to_owned()))?;

        let mut file_bytes = header;
        file_bytes.extend_from_slice(&ciphertext);

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                CredentialsError::Io(format!("create {}: {error}", parent.display()))
            })?;
        }
        write_atomic(&self.path, &file_bytes)
    }
}

/// 原子写：临时文件 + 轮转（Windows 的 rename 不覆盖已存在的目标）。
///
/// 崩溃在任意一步，磁盘上都还留着一份**完整**的保险库；最坏情况是留下一个
/// `.bak`（下次写入时会覆盖它）。
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CredentialsError> {
    let temp = path.with_extension("tmp");
    let backup = path.with_extension("bak");

    fs::write(&temp, bytes)
        .map_err(|error| CredentialsError::Io(format!("write {}: {error}", temp.display())))?;

    if path.exists() {
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup)
            .map_err(|error| CredentialsError::Io(format!("rotate {}: {error}", path.display())))?;
    }
    fs::rename(&temp, path)
        .map_err(|error| CredentialsError::Io(format!("install {}: {error}", path.display())))?;
    let _ = fs::remove_file(&backup);
    Ok(())
}

/// 把已解锁的保险库接到 [`CredentialBackend`] 上。
pub struct VaultBackend {
    vault: std::sync::Mutex<Vault>,
}

impl VaultBackend {
    /// 用已解锁的保险库构造。
    pub fn new(vault: Vault) -> Self {
        Self {
            vault: std::sync::Mutex::new(vault),
        }
    }

    /// 新建保险库并包装。
    pub fn create(path: &Path, passphrase: &Secret) -> Result<Self, CredentialsError> {
        Ok(Self::new(Vault::create(path, passphrase)?))
    }

    /// 解锁已有保险库并包装。
    pub fn open(path: &Path, passphrase: &Secret) -> Result<Self, CredentialsError> {
        Ok(Self::new(Vault::open(path, passphrase)?))
    }

    /// 条目数（诊断用）。
    pub fn len(&self) -> Result<usize, CredentialsError> {
        let vault = self
            .vault
            .lock()
            .map_err(|_| CredentialsError::Io("vault lock poisoned".to_owned()))?;
        Ok(vault.len())
    }

    /// 是否为空。
    pub fn is_empty(&self) -> Result<bool, CredentialsError> {
        Ok(self.len()? == 0)
    }
}

impl CredentialBackend for VaultBackend {
    fn set(&self, _service: &str, account: &str, secret: &Secret) -> Result<(), CredentialsError> {
        let mut vault = self
            .vault
            .lock()
            .map_err(|_| CredentialsError::Io("vault lock poisoned".to_owned()))?;
        vault.set(account, secret)
    }

    fn get(&self, _service: &str, account: &str) -> Result<Secret, CredentialsError> {
        let vault = self
            .vault
            .lock()
            .map_err(|_| CredentialsError::Io("vault lock poisoned".to_owned()))?;
        vault.get(account)
    }

    fn delete(&self, _service: &str, account: &str) -> Result<(), CredentialsError> {
        let mut vault = self
            .vault
            .lock()
            .map_err(|_| CredentialsError::Io("vault lock poisoned".to_owned()))?;
        vault.delete(account)
    }

    fn kind(&self) -> BackendKind {
        BackendKind::EncryptedVault
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// 测试用的低开销参数：8 MiB × 1 次。真实默认值另有专门用例覆盖。
    fn fast() -> VaultParams {
        VaultParams {
            m_cost_kib: 8 * 1024,
            t_cost: 1,
            p_cost: 1,
        }
    }

    fn temp_path(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("forgedesk-vault-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir.join("credentials.vault")
    }

    #[test]
    fn a_vault_round_trips_entries_across_reopen() {
        let path = temp_path("roundtrip");
        let passphrase = Secret::new("correct horse battery staple");

        {
            let mut vault = Vault::create_with(&path, &passphrase, fast()).expect("create");
            vault
                .set("github:github.com:octocat", &Secret::new("ghp_x"))
                .expect("set");
            vault
                .set("generic:git.internal:alice", &Secret::new("p@ss"))
                .expect("set");
        }

        let vault = Vault::open(&path, &passphrase).expect("open");
        assert_eq!(vault.len(), 2);
        assert_eq!(
            vault
                .get("github:github.com:octocat")
                .expect("get")
                .expose(),
            "ghp_x"
        );
        assert_eq!(
            vault
                .get("generic:git.internal:alice")
                .expect("get")
                .expose(),
            "p@ss"
        );

        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn the_default_parameters_also_round_trip() {
        let path = temp_path("default-params");
        let passphrase = Secret::new("pw");

        {
            let mut vault = Vault::create(&path, &passphrase).expect("create");
            vault.set("a:b:c", &Secret::new("v")).expect("set");
        }

        let vault = Vault::open(&path, &passphrase).expect("open");
        assert_eq!(vault.get("a:b:c").expect("get").expose(), "v");

        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn the_plaintext_never_appears_in_the_file_on_disk() {
        let path = temp_path("plaintext");
        let passphrase = Secret::new("pw");
        let secret_text = "ghp_must_not_appear_in_the_file";

        {
            let mut vault = Vault::create_with(&path, &passphrase, fast()).expect("create");
            vault
                .set("github:github.com:octocat", &Secret::new(secret_text))
                .expect("set");
        }

        let raw = fs::read(&path).expect("read");
        let haystack = String::from_utf8_lossy(&raw);
        assert!(
            !haystack.contains(secret_text),
            "凭据明文出现在保险库文件里（红线 R8）"
        );
        // account 名是允许落盘的（它不是秘密），但也要确认文件确实被加密过：
        // base64 之后仍然不该直接出现原始明文
        assert!(!haystack.contains("ghp_"));

        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn a_wrong_passphrase_cannot_unlock_the_vault() {
        let path = temp_path("wrong-pass");
        {
            let mut vault =
                Vault::create_with(&path, &Secret::new("right"), fast()).expect("create");
            vault.set("a:b:c", &Secret::new("v")).expect("set");
        }

        let error = Vault::open(&path, &Secret::new("wrong")).expect_err("must refuse");

        assert_eq!(error, CredentialsError::VaultLocked);
        assert_eq!(error.code(), forgedesk_domain::ErrorCode::Storage);

        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn tampering_with_the_ciphertext_is_detected() {
        let path = temp_path("tamper-body");
        let passphrase = Secret::new("pw");
        {
            let mut vault = Vault::create_with(&path, &passphrase, fast()).expect("create");
            vault.set("a:b:c", &Secret::new("v")).expect("set");
        }

        let mut raw = fs::read(&path).expect("read");
        let last = raw.len() - 1;
        raw[last] ^= 0x01;
        fs::write(&path, &raw).expect("write");

        assert_eq!(
            Vault::open(&path, &passphrase).expect_err("must detect tampering"),
            CredentialsError::VaultLocked
        );

        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn downgrading_the_kdf_parameters_in_the_header_is_detected_because_the_header_is_authenticated(
    ) {
        let path = temp_path("tamper-header");
        let passphrase = Secret::new("pw");
        {
            let mut vault = Vault::create_with(&path, &passphrase, fast()).expect("create");
            vault.set("a:b:c", &Secret::new("v")).expect("set");
        }

        let mut raw = fs::read(&path).expect("read");
        // 把 t_cost 改大（仍在允许区间内）：若头部未参与认证，攻击者就能用这种方式
        // 改写 KDF 参数而不被发现。注意不能改成 m_cost 的"最小值"——那正是本用例
        // 创建时的取值，等于什么都没改（第一版就是这么写错的）
        let offset = MAGIC.len() + SALT_LEN + 4;
        raw[offset..offset + 4].copy_from_slice(&2_u32.to_be_bytes());
        fs::write(&path, &raw).expect("write");

        assert_eq!(
            Vault::open(&path, &passphrase).expect_err("参数改动必须被认证拦下"),
            CredentialsError::VaultLocked
        );

        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn an_absurd_memory_cost_in_the_header_is_refused_before_allocating() {
        let path = temp_path("absurd-cost");
        let passphrase = Secret::new("pw");
        {
            let mut vault = Vault::create_with(&path, &passphrase, fast()).expect("create");
            vault.set("a:b:c", &Secret::new("v")).expect("set");
        }

        let mut raw = fs::read(&path).expect("read");
        let offset = MAGIC.len() + SALT_LEN;
        raw[offset..offset + 4].copy_from_slice(&(4 * 1024 * 1024_u32).to_be_bytes());
        fs::write(&path, &raw).expect("write");

        match Vault::open(&path, &passphrase) {
            Err(CredentialsError::VaultMalformed(_)) => {}
            other => panic!("expected VaultMalformed, got {other:?}"),
        }

        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn a_file_that_is_not_a_vault_is_reported_as_malformed_not_as_a_wrong_passphrase() {
        let path = temp_path("not-a-vault");
        fs::write(
            &path,
            b"this is just a text file, not a vault at all........",
        )
        .expect("write");

        match Vault::open(&path, &Secret::new("pw")) {
            Err(CredentialsError::VaultMalformed(_)) => {}
            other => panic!("expected VaultMalformed, got {other:?}"),
        }

        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn opening_a_vault_that_was_never_created_says_so() {
        let path = temp_path("missing");

        match Vault::open(&path, &Secret::new("pw")) {
            Err(CredentialsError::VaultMissing(reported)) => {
                assert!(reported.ends_with("credentials.vault"), "{reported}");
            }
            other => panic!("expected VaultMissing, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_passphrase_is_refused_when_creating() {
        let path = temp_path("empty-pass");

        assert!(matches!(
            Vault::create_with(&path, &Secret::new(""), fast()),
            Err(CredentialsError::Invalid(_))
        ));
        assert!(!path.exists());
    }

    #[test]
    fn deleting_an_entry_persists_immediately() {
        let path = temp_path("delete");
        let passphrase = Secret::new("pw");
        {
            let mut vault = Vault::create_with(&path, &passphrase, fast()).expect("create");
            vault.set("a:b:c", &Secret::new("v")).expect("set");
            vault.delete("a:b:c").expect("delete");
            // 幂等
            vault.delete("a:b:c").expect("delete again");
        }

        let vault = Vault::open(&path, &passphrase).expect("open");
        assert!(vault.is_empty());
        assert!(matches!(
            vault.get("a:b:c"),
            Err(CredentialsError::NotFound(_))
        ));

        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn the_backend_adapter_behaves_like_any_other_credential_backend() {
        let path = temp_path("backend");
        let backend = VaultBackend::create(&path, &Secret::new("pw")).expect("create");

        backend
            .set("org.forgedesk.app", "a:b:c", &Secret::new("v"))
            .expect("set");
        assert_eq!(
            backend
                .get("org.forgedesk.app", "a:b:c")
                .expect("get")
                .expose(),
            "v"
        );
        assert_eq!(backend.kind(), BackendKind::EncryptedVault);
        assert_eq!(backend.len().expect("len"), 1);

        backend
            .delete("org.forgedesk.app", "a:b:c")
            .expect("delete");
        assert!(backend.is_empty().expect("empty"));
        assert!(matches!(
            backend.get("org.forgedesk.app", "a:b:c"),
            Err(CredentialsError::NotFound(_))
        ));

        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn vault_debug_reports_only_the_path_and_entry_count() {
        let path = temp_path("debug");
        let mut vault = Vault::create_with(&path, &Secret::new("pw"), fast()).expect("create");
        vault
            .set("a:b:c", &Secret::new("supersecret"))
            .expect("set");

        let text = format!("{vault:?}");

        assert!(text.contains("entries: 1"));
        assert!(!text.contains("supersecret"), "{text}");

        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn weak_parameters_are_refused_instead_of_writing_an_uncrackable_looking_file() {
        let path = temp_path("weak-params");
        let weak = VaultParams {
            m_cost_kib: 512,
            t_cost: 1,
            p_cost: 1,
        };

        assert!(matches!(
            Vault::create_with(&path, &Secret::new("pw"), weak),
            Err(CredentialsError::VaultMalformed(_))
        ));
        assert!(!path.exists());
    }
}
