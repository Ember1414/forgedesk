//! 仓库指纹（T3.9）：用**少数几个稳定事实**描述"仓库现在长什么样"。
//!
//! # 为什么需要它
//!
//! 回滚要回答两个问题："回滚到底有没有成功"（校验）与"回滚之后仓库是不是
//! 真的回到了那一刻"（幂等与故障安全）。逐个字段比对（HEAD、分支、索引树、
//! 已跟踪内容、未跟踪集合、进行中的操作）比"看 git status 输出"可靠得多：
//! 输出是人读的，字段是机器读的。
//!
//! # 为什么指纹是"哈希 + 原始值"的混合
//!
//! - HEAD oid / 分支名 / 索引树 oid：**保留原始值**。它们短、可读，
//!   出错时报告里能直接写出"应该是 X，实际是 Y"，比一个哈希有用得多；
//! - 文件集合（已跟踪、未跟踪）：**只留哈希**。集合可能上万项，
//!   逐项比对既慢又没人看得完，而这里的用途只是"变了没有"。
//!
//! # 大仓库怎么办
//!
//! `tracked_files_hash` 是 `Option`：仓库超过阈值时**跳过**计算（`None`）。
//! 比较时两边只要有一边是 `None` 就**不断言**变化——"不知道"不是"不一样"，
//! 把未知当成变化会让校验随机失败。

/// 已跟踪文件数超过这个值就跳过内容哈希（大仓库回滚时不必为此扫全库）。
pub const TRACKED_HASH_LIMIT: usize = 20_000;

/// 仓库指纹：一次回滚前后的"同一性"判据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoFingerprint {
    /// HEAD 指向的提交（空仓库为 `None`）。
    pub head_oid: Option<String>,
    /// 当前分支全名（游离 HEAD 为 `None`）。
    pub head_ref: Option<String>,
    /// 索引树 oid（索引有未合并条目时为 `None`）。
    pub index_tree_oid: Option<String>,
    /// 已跟踪文件的哈希；`None` = 本次跳过（仓库太大）。
    pub tracked_files_hash: Option<String>,
    /// 未跟踪路径集合的哈希（排序后，与顺序无关）。
    pub untracked_paths_hash: String,
    /// 进行中的操作短名（`rebase` / `merge` / `cherry-pick` / `revert`）。
    pub operation_state: Option<String>,
}

impl RepoFingerprint {
    /// 与另一份指纹是否完全一致（含"两边都没算 tracked"的情形）。
    pub fn identical_to(&self, other: &Self) -> bool {
        compare(self, other).is_identical()
    }
}

/// 两个指纹之间的差异（逐字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FingerprintDiff {
    /// HEAD 指向的提交变了。
    pub head_changed: bool,
    /// 当前分支变了（含"从分支变成游离 HEAD"）。
    pub head_ref_changed: bool,
    /// 索引树变了（"已暂存未提交"的内容不同）。
    pub index_changed: bool,
    /// 已跟踪内容的变更集变了；两边有一边跳过计算时恒为 `false`（不知道 ≠ 不一样）。
    pub tracked_changed: bool,
    /// 未跟踪路径集合变了。
    pub untracked_changed: bool,
    /// 进行中的操作变了（例如回滚把一次合并中途的状态留了下来）。
    pub operation_changed: bool,
}

impl FingerprintDiff {
    /// 是否完全一致。
    pub fn is_identical(&self) -> bool {
        !self.any_changed()
    }

    /// 是否有任何一项不同。
    pub fn any_changed(&self) -> bool {
        self.head_changed
            || self.head_ref_changed
            || self.index_changed
            || self.tracked_changed
            || self.untracked_changed
            || self.operation_changed
    }

    /// 变化项的短名（报告里说清"到底是哪儿不一样"）。
    pub fn changed_fields(&self) -> Vec<&'static str> {
        let mut fields = Vec::new();
        if self.head_changed {
            fields.push("head");
        }
        if self.head_ref_changed {
            fields.push("headRef");
        }
        if self.index_changed {
            fields.push("index");
        }
        if self.tracked_changed {
            fields.push("tracked");
        }
        if self.untracked_changed {
            fields.push("untracked");
        }
        if self.operation_changed {
            fields.push("operation");
        }
        fields
    }
}

/// 比较两份指纹。
pub fn compare(before: &RepoFingerprint, after: &RepoFingerprint) -> FingerprintDiff {
    FingerprintDiff {
        head_changed: before.head_oid != after.head_oid,
        head_ref_changed: before.head_ref != after.head_ref,
        index_changed: before.index_tree_oid != after.index_tree_oid,
        tracked_changed: match (&before.tracked_files_hash, &after.tracked_files_hash) {
            (Some(left), Some(right)) => left != right,
            // 有一边跳过就算"不可比"：让调用方看到"没变化"而不是随机的失败
            _ => false,
        },
        untracked_changed: before.untracked_paths_hash != after.untracked_paths_hash,
        operation_changed: before.operation_state != after.operation_state,
    }
}

/// FNV-1a 的 64 位偏移基准。
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 的 64 位质数。
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// 一组**路径**的稳定哈希（16 位十六进制）。
///
/// 为什么是 FNV 而不是密码学哈希：这个值只用于比较，与防篡改无关；
/// 而 `domain` 不许引入任何外部依赖，FNV 是十几行纯函数。
///
/// 两个细节：
/// - **先排序**：`["a","b"]` 与 `["b","a"]` 是同一个集合，必须得到同一个哈希；
/// - **写入长度前缀**：否则 `["ab"]` 与 `["a","b"]` 会碰撞（分隔符可能出现在路径里）。
pub fn hash_paths<'a, I>(paths: I) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let mut sorted: Vec<&str> = paths.into_iter().collect();
    sorted.sort_unstable();

    let mut hash = FNV_OFFSET;
    for path in sorted {
        hash = fnv1a(hash, &(path.len() as u64).to_le_bytes());
        hash = fnv1a(hash, path.as_bytes());
    }
    format!("{hash:016x}")
}

/// 一个文件的字节内容的稳定哈希（`tracked_files_hash` 的组成单元）。
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    fnv1a(FNV_OFFSET, bytes)
}

/// 把多个"内容哈希"折叠成一个：`name → 哈希` 的有序列表。
///
/// 输入必须**已按路径排序**（调用方排序）：这样折叠结果与遍历顺序无关。
pub fn fold_content_hashes<'a, I>(entries: I) -> String
where
    I: IntoIterator<Item = (&'a str, u64)>,
{
    let mut hash = FNV_OFFSET;
    for (path, content) in entries {
        hash = fnv1a(hash, &(path.len() as u64).to_le_bytes());
        hash = fnv1a(hash, path.as_bytes());
        hash = fnv1a(hash, &content.to_le_bytes());
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::{compare, fold_content_hashes, hash_bytes, hash_paths, RepoFingerprint};

    fn fingerprint() -> RepoFingerprint {
        RepoFingerprint {
            head_oid: Some("a".repeat(40)),
            head_ref: Some("refs/heads/main".to_owned()),
            index_tree_oid: Some("t".repeat(40)),
            tracked_files_hash: Some("abc".to_owned()),
            untracked_paths_hash: hash_paths(["scratch.txt"]),
            operation_state: None,
        }
    }

    #[test]
    fn path_hash_is_order_independent() {
        // 同一集合的不同顺序必须是同一个哈希，否则指纹会随机不等
        assert_eq!(
            hash_paths(["a.txt", "b/c.txt", "d.txt"]),
            hash_paths(["d.txt", "a.txt", "b/c.txt"])
        );
    }

    #[test]
    fn path_hash_distinguishes_the_len_prefix_case() {
        // ["ab"] 与 ["a","b"] 必须是不同的集合
        assert_ne!(hash_paths(["ab"]), hash_paths(["a", "b"]));
        // 空集合也有确定的值（不是空串：空串看起来像"没算过"）
        assert_eq!(hash_paths(Vec::<&str>::new()).len(), 16);
    }

    #[test]
    fn identical_fingerprints_compare_equal() {
        let left = fingerprint();
        let right = fingerprint();
        assert!(left.identical_to(&right));
        assert!(compare(&left, &right).changed_fields().is_empty());
    }

    #[test]
    fn each_field_is_reported_separately() {
        let before = fingerprint();

        let mut head = before.clone();
        head.head_oid = Some("b".repeat(40));
        assert_eq!(compare(&before, &head).changed_fields(), vec!["head"]);

        let mut index = before.clone();
        index.index_tree_oid = None;
        assert_eq!(compare(&before, &index).changed_fields(), vec!["index"]);

        let mut untracked = before.clone();
        untracked.untracked_paths_hash = hash_paths(["other.txt"]);
        assert_eq!(
            compare(&before, &untracked).changed_fields(),
            vec!["untracked"]
        );

        let mut operation = before.clone();
        operation.operation_state = Some("rebase".to_owned());
        assert_eq!(
            compare(&before, &operation).changed_fields(),
            vec!["operation"]
        );

        let mut branch = before.clone();
        branch.head_ref = None;
        assert_eq!(compare(&before, &branch).changed_fields(), vec!["headRef"]);
    }

    #[test]
    fn skipped_tracked_hash_never_reports_a_change() {
        // 大仓库跳过内容哈希时，"不知道"不能被当成"不一样"
        let mut skipped = fingerprint();
        skipped.tracked_files_hash = None;
        let mut other = fingerprint();
        other.tracked_files_hash = None;

        let diff = compare(&skipped, &other);
        assert!(!diff.tracked_changed);
        assert!(diff.is_identical());
    }

    #[test]
    fn content_hash_folds_paths_and_bytes() {
        let left =
            fold_content_hashes([("a.txt", hash_bytes(b"one")), ("b.txt", hash_bytes(b"two"))]);
        let right =
            fold_content_hashes([("a.txt", hash_bytes(b"one")), ("b.txt", hash_bytes(b"two"))]);
        assert_eq!(left, right);

        // 内容变了必须能看出来
        let changed = fold_content_hashes([
            ("a.txt", hash_bytes(b"one!")),
            ("b.txt", hash_bytes(b"two")),
        ]);
        assert_ne!(left, changed);

        // 路径变了也必须能看出来（内容一样）
        let renamed =
            fold_content_hashes([("a.txt", hash_bytes(b"one")), ("c.txt", hash_bytes(b"two"))]);
        assert_ne!(left, renamed);
    }
}
