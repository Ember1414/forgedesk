//! 凭据索引：记录"我们存过哪些凭据"。
//!
//! # 为什么必须自己维护一份索引
//!
//! 三个平台的系统凭据库**没有统一的枚举接口**：Windows Credential Manager 能按
//! target 前缀枚举，macOS Keychain 的 `SecItemCopyMatching` 语义不同，Linux 的
//! Secret Service 需要遍历 collection 且各实现行为不一。而"我保存过哪些账号"是设置页
//! 的核心需求，不能靠后端"尽力而为"。
//!
//! 因此：**索引以我们自己为准，密文本体仍在系统凭据库里**（红线 R8）。
//! 索引里只有引用、类型与写入时间，丢了也不致命（最多丢"列表"，用户重新保存一次
//! 原来那条凭据即可，因为密文还在系统库里；这也解释了为什么索引可以做原子替换）。
//!
//! # 与 T4.4 的关系
//!
//! 计划的账号模型（`accounts` 表，`migrations/0001_init.sql`）落地后，
//! [`CredentialIndex`] 应改由那张表实现（T4.4），本层的 `store/get/delete/list`
//! 语义不变——这是本 trait 存在的意义。

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CredentialsError;
use crate::model::{CredentialKind, CredentialRef};

/// 索引文件的结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexFile {
    /// 格式版本：将来加字段时用它决定怎么迁移（未知版本按"读不出来"处理，
    /// 而不是猜——索引丢了可以重建，猜错了会让列表与实际凭据对不上）。
    version: u32,
    entries: Vec<IndexEntry>,
}

/// 索引里的一条。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    /// keyring 的 account 名（`<provider>:<host>:<login>`）。
    pub account: String,
    /// 凭据类型。
    pub kind: CredentialKind,
    /// 写入时间（Unix 毫秒）。
    pub created_at_ms: i64,
}

impl IndexEntry {
    /// 还原成引用（索引里坏掉的那条返回 `None`，由调用方跳过）。
    pub fn reference(&self) -> Option<CredentialRef> {
        CredentialRef::parse(&self.account)
    }
}

/// 索引当前支持的格式版本。
pub const INDEX_VERSION: u32 = 1;

/// 凭据索引的读写接口。
pub trait CredentialIndex: Send + Sync {
    /// 记录一条（同 account 覆盖）。
    fn upsert(&self, entry: IndexEntry) -> Result<(), CredentialsError>;
    /// 删除一条（不存在时返回 `Ok`）。
    fn remove(&self, account: &str) -> Result<(), CredentialsError>;
    /// 列出全部（顺序由实现决定，调用方自行排序）。
    fn list(&self) -> Result<Vec<IndexEntry>, CredentialsError>;
}

/// 纯内存索引（测试、以及"本次会话临时凭据"）。
#[derive(Debug, Default)]
pub struct MemoryIndex {
    entries: std::sync::Mutex<BTreeMap<String, IndexEntry>>,
}

impl MemoryIndex {
    /// 新建空索引。
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialIndex for MemoryIndex {
    fn upsert(&self, entry: IndexEntry) -> Result<(), CredentialsError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CredentialsError::Io("index lock poisoned".to_owned()))?;
        entries.insert(entry.account.clone(), entry);
        Ok(())
    }

    fn remove(&self, account: &str) -> Result<(), CredentialsError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CredentialsError::Io("index lock poisoned".to_owned()))?;
        entries.remove(account);
        Ok(())
    }

    fn list(&self) -> Result<Vec<IndexEntry>, CredentialsError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| CredentialsError::Io("index lock poisoned".to_owned()))?;
        Ok(entries.values().cloned().collect())
    }
}

/// 落盘的 JSON 索引。
///
/// 写入走"临时文件 + 原子替换"：直接覆写正在被读的文件，会在崩溃时留下半截 JSON，
/// 而半截 JSON 的后果是"账号列表突然空了"——用户会以为凭据丢了。
#[derive(Debug)]
pub struct FileIndex {
    path: PathBuf,
}

impl FileIndex {
    /// 指向某个索引文件（不存在时视为空索引，不报错）。
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 索引文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 读取全部条目；文件不存在 → 空；内容坏掉 → 报错（宁可提示，也不静默清空列表）。
    fn read(&self) -> Result<Vec<IndexEntry>, CredentialsError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&self.path).map_err(|error| {
            CredentialsError::Io(format!("read {}: {error}", self.path.display()))
        })?;
        if raw.trim().is_empty() {
            return Ok(Vec::new());
        }
        let file: IndexFile = serde_json::from_str(&raw).map_err(|error| {
            CredentialsError::Io(format!("parse {}: {error}", self.path.display()))
        })?;
        if file.version != INDEX_VERSION {
            return Err(CredentialsError::Io(format!(
                "unsupported index version {} in {}",
                file.version,
                self.path.display()
            )));
        }
        Ok(file.entries)
    }

    /// 写回全部条目（原子替换）。
    fn write(&self, entries: Vec<IndexEntry>) -> Result<(), CredentialsError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                CredentialsError::Io(format!("create {}: {error}", parent.display()))
            })?;
        }

        let payload = serde_json::to_string_pretty(&IndexFile {
            version: INDEX_VERSION,
            entries,
        })
        .map_err(|error| CredentialsError::Io(format!("serialise index: {error}")))?;

        // Windows 的 rename **不覆盖**已存在的目标（docs/CODING_STYLE.md §1 提到的事故类型），
        // 因此：先把旧文件改名成 .bak，再就位新文件，最后删掉 .bak。
        // 中间任何一步崩掉，磁盘上都还留着一份完整索引（新的或旧的）。
        let temp = self.path.with_extension("tmp");
        let backup = self.path.with_extension("bak");

        {
            let mut file = fs::File::create(&temp).map_err(|error| {
                CredentialsError::Io(format!("create {}: {error}", temp.display()))
            })?;
            file.write_all(payload.as_bytes()).map_err(|error| {
                CredentialsError::Io(format!("write {}: {error}", temp.display()))
            })?;
            // 先 flush 再 rename：否则可能 rename 出一个内容还在页缓存里的文件
            file.sync_all().map_err(|error| {
                CredentialsError::Io(format!("sync {}: {error}", temp.display()))
            })?;
        }

        if self.path.exists() {
            let _ = fs::remove_file(&backup);
            fs::rename(&self.path, &backup).map_err(|error| {
                CredentialsError::Io(format!("rotate {}: {error}", self.path.display()))
            })?;
        }
        fs::rename(&temp, &self.path).map_err(|error| {
            CredentialsError::Io(format!("install {}: {error}", self.path.display()))
        })?;
        let _ = fs::remove_file(&backup);
        Ok(())
    }

    /// 读-改-写。
    fn mutate<F>(&self, change: F) -> Result<(), CredentialsError>
    where
        F: FnOnce(&mut BTreeMap<String, IndexEntry>),
    {
        let mut map: BTreeMap<String, IndexEntry> = self
            .read()?
            .into_iter()
            .map(|entry| (entry.account.clone(), entry))
            .collect();
        change(&mut map);
        self.write(map.into_values().collect())
    }
}

impl CredentialIndex for FileIndex {
    fn upsert(&self, entry: IndexEntry) -> Result<(), CredentialsError> {
        self.mutate(|entries| {
            entries.insert(entry.account.clone(), entry);
        })
    }

    fn remove(&self, account: &str) -> Result<(), CredentialsError> {
        self.mutate(|entries| {
            entries.remove(account);
        })
    }

    fn list(&self) -> Result<Vec<IndexEntry>, CredentialsError> {
        self.read()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// 每个用例一个独立目录（用后清理），避免并行执行时互相踩。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("forgedesk-idx-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn entry(account: &str, kind: CredentialKind, created_at_ms: i64) -> IndexEntry {
        IndexEntry {
            account: account.to_owned(),
            kind,
            created_at_ms,
        }
    }

    #[test]
    fn a_file_index_round_trips_entries_through_disk() {
        let dir = temp_dir("roundtrip");
        let index = FileIndex::at(dir.join("credentials.json"));

        index
            .upsert(entry("github:github.com:octocat", CredentialKind::Pat, 11))
            .expect("upsert");

        // 重新构造一个实例，真正验证"落盘了"
        let reopened = FileIndex::at(dir.join("credentials.json"));
        let entries = reopened.list().expect("list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].account, "github:github.com:octocat");
        assert_eq!(entries[0].kind, CredentialKind::Pat);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_index_file_is_an_empty_index_not_an_error() {
        let dir = temp_dir("missing");
        let index = FileIndex::at(dir.join("nope.json"));

        assert_eq!(index.list().expect("list"), Vec::new());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn upserting_the_same_account_replaces_instead_of_duplicating() {
        let dir = temp_dir("upsert");
        let index = FileIndex::at(dir.join("credentials.json"));

        index
            .upsert(entry("g:h:a", CredentialKind::Pat, 1))
            .expect("first");
        index
            .upsert(entry("g:h:a", CredentialKind::Password, 2))
            .expect("second");

        let entries = index.list().expect("list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, CredentialKind::Password);
        assert_eq!(entries[0].created_at_ms, 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn removal_is_idempotent_and_leaves_no_temporary_files_behind() {
        let dir = temp_dir("remove");
        let path = dir.join("credentials.json");
        let index = FileIndex::at(&path);

        index
            .upsert(entry("g:h:a", CredentialKind::Pat, 1))
            .expect("upsert");
        index.remove("g:h:a").expect("remove");
        index.remove("g:h:a").expect("remove again");

        assert!(index.list().expect("list").is_empty());
        assert!(!path.with_extension("tmp").exists());
        assert!(!path.with_extension("bak").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupted_index_reports_an_error_instead_of_silently_looking_empty() {
        let dir = temp_dir("corrupt");
        let path = dir.join("credentials.json");
        fs::write(&path, b"{ this is not json").expect("write");

        let index = FileIndex::at(&path);
        // 静默返回空列表会让"账号列表空了"变成一个无法解释的现象
        assert!(index.list().is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_index_version_is_refused_rather_than_misread() {
        let dir = temp_dir("version");
        let path = dir.join("credentials.json");
        fs::write(&path, br#"{"version": 99, "entries": []}"#).expect("write");

        assert!(FileIndex::at(&path).list().is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn entries_can_be_turned_back_into_references() {
        assert_eq!(
            entry("github:github.com:octocat", CredentialKind::Pat, 1)
                .reference()
                .expect("parsable")
                .login,
            "octocat"
        );
        assert!(entry("broken", CredentialKind::Pat, 1)
            .reference()
            .is_none());
    }

    #[test]
    fn a_memory_index_behaves_like_the_file_one_for_the_same_operations() {
        let index = MemoryIndex::new();
        index
            .upsert(entry("g:h:a", CredentialKind::Oauth, 5))
            .expect("upsert");

        assert_eq!(index.list().expect("list").len(), 1);
        assert_eq!(index.list().expect("list")[0].kind, CredentialKind::Oauth);

        index.remove("g:h:a").expect("remove");
        assert!(index.list().expect("list").is_empty());
    }
}
