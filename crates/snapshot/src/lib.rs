//! 操作快照与回滚：破坏性操作前的状态打点与一致性校验。
//!
//! # 红线 R7 的最后一环（M1 / T1.9）
//!
//! 完整链条是"计划预览 → 快照 → 执行 → 可回滚"。T1.7 之前实际的安全网只有前两步
//! （快照位由 [`NoopSnapshotManager`] 如实标注"没有"）；现在 [`RefSnapshotManager`]
//! 补上了后两步：每个快照是一组**可独立校验的 git 事实**（HEAD oid、索引树、
//! 自定义 ref 锚点），而不是一份需要解释的备份文件。
//!
//! # 为什么是"自定义 ref"而不是备份目录或 reflog
//!
//! - **备份目录**（把整个工作区拷走）是 T3.8 快照 v2 的事：v1 的破坏性操作路径
//!   （提交、amend）不会删除或覆盖未跟踪文件，记录它们的路径就够定位问题；
//! - **reflog / `HEAD@{n}`** 会被外部操作改写或清空（别的工具、`gc --prune=now`、
//!   clone），不能作为唯一依据——任务定义明确禁止。
//!   自定义 ref `refs/forgedesk/snapshots/<id>` 指向快照时刻的 HEAD 提交，
//!   git 因此**不会**把它当垃圾回收，这是"锚点"二字的含义。
//!
//! # 校验纪律
//!
//! 回滚之后必须**用 git 自己的事实**核对（HEAD oid、`write-tree` 的树 oid），
//! 对不上就返回 [`SnapshotError::RestoreVerify`]，并且**自动恢复到回滚前快照**——
//! 用户要的是"回到过去"，不是"落到一个既不是过去也不是现在的状态"。

#![forbid(unsafe_code)]

use std::path::Path;

use forgedesk_domain::ErrorCode;

/// 快照 id（`snapshots` 表主键）。
pub type SnapshotId = i64;

mod backup;
mod ref_manager;

pub use ref_manager::{
    RefSnapshotManager, DEFAULT_MAX_AGE_DAYS, DEFAULT_MAX_COUNT, SNAPSHOT_REF_PREFIX,
};

/// 单份快照的未跟踪内容备份上限（200 MiB；`0` = 不限制）。
///
/// 这个默认值是任务书给定、**待人类确认**的：未跟踪文件往往是构建产物、
/// 依赖目录（`node_modules/` 动辄数百 MiB），按仓库全量备份会把磁盘吃光；
/// 而超过上限时我们选择"整体不备份 + 明确告警"，而不是悄悄漏掉一部分。
pub const DEFAULT_MAX_SNAPSHOT_BYTES: u64 = 200 * 1024 * 1024;

/// 单个仓库的快照备份总占用上限（2 GiB；`0` = 不限制），超出时按 LRU 清理。
pub const DEFAULT_MAX_REPO_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 快照锚点 ref 的前缀（完整形如 `refs/forgedesk/snapshots/<id>`）。
///
/// 挂在 `refs/forgedesk/` 下而不是 `refs/heads/`：它是**应用的管理数据**，
/// 不该出现在分支列表里，也不该被 `git push --mirror` 之类当成普通分支带走。
pub const SNAPSHOT_REF_NAMESPACE: &str = "refs/forgedesk/snapshots";

/// 创建快照的场景。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    /// 用户手动创建。
    Manual,
    /// 提交之前。
    PreCommit,
    /// 回滚之前——先给"现在"打一个点，回滚错了还能回来。
    PreRestore,
    /// 同步（拉取 / 合并 / 变基）之前。
    PreSync,
    /// 会移动 HEAD 的操作（重置、切换分支）之前。
    PreHeadMove,
    /// 只改动工作区/索引、不动 HEAD 的操作（储藏、应用储藏）之前。
    ///
    /// 与 [`Self::PreHeadMove`] 分开：回滚一份"只动了工作区"的快照不需要移动 HEAD，
    /// 界面上的说明也完全不同（"恢复我的未提交改动" vs "回到某个提交"）。
    PreWorktreeChange,
}

impl SnapshotKind {
    /// 稳定的短名（写进 `snapshots.kind` 与审计；前端据此走 i18n）。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::PreCommit => "pre-commit",
            Self::PreRestore => "pre-restore",
            Self::PreSync => "pre-sync",
            Self::PreHeadMove => "pre-head-move",
            Self::PreWorktreeChange => "pre-worktree-change",
        }
    }
}

/// 创建快照的请求。
#[derive(Debug, Clone, Copy)]
pub struct SnapshotRequest<'a> {
    /// 存储层记录 id（`repositories.id`）。
    pub repo_id: i64,
    /// 仓库工作区（快照要读 HEAD、索引与未跟踪清单）。
    pub workdir: &'a Path,
    /// 展示用的标签（例如 `pre-commit`）。
    pub label: &'a str,
    /// 场景。
    pub kind: SnapshotKind,
}

/// 快照的列表行（给界面看的摘要；恢复细节用 [`SnapshotDiff`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta {
    /// 主键。
    pub id: SnapshotId,
    /// 展示标签。
    pub label: String,
    /// 场景短名。
    pub kind: String,
    /// 快照时刻的 HEAD oid（全量；界面自行截短显示）。
    pub head_oid: String,
    /// 当时的分支名。
    pub branch: Option<String>,
    /// 是否游离 HEAD。
    pub detached: bool,
    /// 创建时间（Unix 毫秒）。
    pub created_at: i64,
}

/// 快照与当前状态的差异摘要。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotDiff {
    /// HEAD 已经不是快照时的位置。
    pub head_changed: bool,
    /// 索引已经不是快照时的树。
    pub index_changed: bool,
    /// 当前的 HEAD oid（空仓库为 `None`）。
    pub current_head_oid: Option<String>,
    /// 当前的索引树；索引里有未合并条目时 `write-tree` 会失败，此时为 `None`
    /// （并保留 `index_changed = true`：读不出树本身就说明状态已经不同）。
    pub current_index_tree_oid: Option<String>,
    /// 锚点 ref 已丢失——这个快照**不可恢复**，只能作为历史记录查看。
    pub ref_missing: bool,
    /// 回滚会写回的未跟踪文件（有内容备份，且当前缺失或内容不同）。
    pub untracked_restorable: Vec<String>,
    /// 快照里记录过、但没有内容备份的未跟踪文件——回滚**找不回来**。
    pub untracked_missing: Vec<String>,
    /// 当前存在、快照里没有的未跟踪文件——回滚**不会删除**它们。
    pub untracked_extra: Vec<String>,
}

/// 快照的内容备份与磁盘策略（T3.8）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotLimits {
    /// 单份快照的未跟踪内容上限（字节；`0` = 不限制）。
    pub max_snapshot_bytes: u64,
    /// 单个仓库的备份总占用上限（字节；`0` = 不限制）。
    pub max_repo_bytes: u64,
    /// 是否连 gitignore 覆盖的文件一起备份（默认否：它们通常体积大且可再生）。
    pub include_ignored: bool,
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            max_snapshot_bytes: DEFAULT_MAX_SNAPSHOT_BYTES,
            max_repo_bytes: DEFAULT_MAX_REPO_BYTES,
            include_ignored: false,
        }
    }
}

/// 备份清单里的一条未跟踪文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupEntry {
    /// 相对仓库根的正斜杠路径（恢复时的落点）。
    pub path: String,
    /// 字节数（恢复前的快速比对；内容一致性另行逐字节校验）。
    pub bytes: u64,
    /// 是否来自 gitignore 覆盖范围（清单里的来源标记，便于用户理解）。
    pub ignored: bool,
}

/// 快照时刻栈上的一条 stash（T3.11 第 7 条）。
///
/// 为什么需要它：`stash drop` / `stash clear` 只删掉**栈里的条目**（reflog），
/// stash 提交本身仍在对象库里——但没有任何引用指向它，`git gc` 之后就会真的消失。
/// 所以快照必须记下 oid 与原文，回滚时用 `git stash store` 重新登记。
///
/// 与 [`BackupEntry`] 的区别：那些是**内容备份**（字节复制到备份目录），
/// 这些只是**引用**（对象库里的提交，不复制字节）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StashedRef {
    /// stash 提交的 oid。
    pub oid: String,
    /// 栈里显示的描述信息（`WIP on main: 1a2b3c4 subject` 之类）。
    pub message: String,
}

/// 快照时刻的一个本地分支（T3.11 第 9 条）。
///
/// 为什么需要它：删掉一条未合并分支（`branch -D`）**不动 HEAD、也不动工作区**，
/// 因此"reset --hard + read-tree"那套回滚对它什么都没做——快照看起来可回滚，
/// 点下去分支却还在原地消失（比"没有回滚点"更糟：那是**假承诺**）。
/// 记下分支引用，回滚时把**缺失的**分支重新创建出来。
///
/// 与 [`StashedRef`] 同一条纪律：只记引用，不复制字节（提交就在对象库里）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BranchRef {
    /// 分支短名（`main`、`feature/x`）。
    pub name: String,
    /// 分支指向的提交 oid。
    pub oid: String,
}

/// 未跟踪内容备份的清单。
///
/// 同时存在于两处：`snapshots.manifest_json`（查询用）与备份目录里的
/// `manifest.json`（自描述——目录被单独拷走或数据库重建时仍能说清里面是什么）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BackupManifest {
    /// 备份的文件。
    pub entries: Vec<BackupEntry>,
    /// 备份内容总字节数（等于 `entries` 的字节和）。
    pub bytes: u64,
    /// 快照时刻栈上的 stash（从**新到旧**，与 `git stash list` 同一顺序）。
    pub stash: Vec<StashedRef>,
    /// 快照时刻的本地分支（按名字排序，便于比对）。
    pub branches: Vec<BranchRef>,
}

impl BackupManifest {
    /// 空清单（该快照没有内容备份、也没有 stash / 分支引用）。
    pub const fn empty() -> Self {
        Self {
            entries: Vec::new(),
            bytes: 0,
            stash: Vec::new(),
            branches: Vec::new(),
        }
    }

    /// 是否没有内容备份（v1 记录、没有未跟踪文件、或超限被整体跳过）。
    ///
    /// 只看 `entries`：stash 与分支不是"内容备份"，一份只带它们的清单
    /// 在这里仍然是"没有内容可写回"（回滚时另有各自的恢复步骤）。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 是否记着栈上的 stash。
    pub fn has_stash(&self) -> bool {
        !self.stash.is_empty()
    }

    /// 是否记着本地分支。
    pub fn has_branches(&self) -> bool {
        !self.branches.is_empty()
    }

    /// 序列化为 JSON（`{"entries":[…],"bytes":N,"stash":[…],"branches":[…]}`）。
    pub fn to_json(&self) -> String {
        let entries: Vec<serde_json::Value> = self
            .entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "path": entry.path,
                    "bytes": entry.bytes,
                    "ignored": entry.ignored,
                })
            })
            .collect();
        let stash: Vec<serde_json::Value> = self
            .stash
            .iter()
            .map(|entry| serde_json::json!({ "oid": entry.oid, "message": entry.message }))
            .collect();
        let branches: Vec<serde_json::Value> = self
            .branches
            .iter()
            .map(|branch| serde_json::json!({ "name": branch.name, "oid": branch.oid }))
            .collect();
        serde_json::json!({
            "entries": entries,
            "bytes": self.bytes,
            "stash": stash,
            "branches": branches,
        })
        .to_string()
    }

    /// 从 JSON 解析；无法解析时返回空清单。
    ///
    /// 宽容解析是有意的：清单损坏不应该让**回滚**失败（HEAD 与索引的恢复
    /// 与它无关），代价只是未跟踪内容恢复不了——而那会在报告里如实体现。
    pub fn from_json(text: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return Self::empty();
        };
        let bytes = value
            .get("bytes")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let entries = value
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let path = item.get("path")?.as_str()?.to_owned();
                        Some(BackupEntry {
                            path,
                            bytes: item
                                .get("bytes")
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0),
                            ignored: item
                                .get("ignored")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // `stash` / `branches` 缺失（T3.11 之前写的清单）时是空表，不是解析失败
        let stash = value
            .get("stash")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(StashedRef {
                            oid: item.get("oid")?.as_str()?.to_owned(),
                            message: item
                                .get("message")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let branches = value
            .get("branches")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(BranchRef {
                            name: item.get("name")?.as_str()?.to_owned(),
                            oid: item.get("oid")?.as_str()?.to_owned(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self {
            entries,
            bytes,
            stash,
            branches,
        }
    }
}

/// 创建快照时的**如实告警**（都不是失败：快照本身已经创建成功）。
///
/// 分类型而不是一句话的原因：前端要按类型给出不同的界面（超限要让用户在
/// 危险操作对话框里确认，空间回收是通知，孤儿清理是背景动作）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotWarning {
    /// 未跟踪内容超过单份上限，**整体未备份**（count 个文件、bytes 字节）。
    UntrackedBackupSkipped {
        /// 被跳过的文件数。
        count: usize,
        /// 它们的总字节数。
        bytes: u64,
        /// 触发跳过的上限。
        limit: u64,
    },
    /// 个别文件复制失败（权限、被占用等），其余仍然备份成功。
    UntrackedBackupPartial {
        /// 失败的相对路径。
        paths: Vec<String>,
        /// 第一个失败的描述（足够定位，不必把每个都写一遍）。
        detail: String,
    },
    /// 备份目录不可用（未配置 / 不可写）：快照不含内容备份。
    BackupDirUnavailable {
        /// 具体原因。
        detail: String,
    },
    /// 为满足仓库总占用上限，按 LRU 清理了旧快照。
    SpaceReclaimed {
        /// 被清理的快照。
        removed: Vec<SnapshotId>,
        /// 释放的字节数。
        freed_bytes: u64,
    },
    /// 上次崩溃留下的孤儿备份目录已清理。
    OrphansRemoved {
        /// 清理的目录数。
        count: usize,
    },
}

impl SnapshotWarning {
    /// 稳定的类型短名（命令层转 DTO；前端据此走 i18n）。
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::UntrackedBackupSkipped { .. } => "untrackedBackupSkipped",
            Self::UntrackedBackupPartial { .. } => "untrackedBackupPartial",
            Self::BackupDirUnavailable { .. } => "backupDirUnavailable",
            Self::SpaceReclaimed { .. } => "spaceReclaimed",
            Self::OrphansRemoved { .. } => "orphansRemoved",
        }
    }
}

/// 创建快照的结果（v2：内容备份体积、跳过清单与告警）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotOutcome {
    /// 新快照的 id。
    pub id: SnapshotId,
    /// 备份内容的字节总数。
    pub backup_bytes: u64,
    /// 备份成功的文件数。
    pub backed_up: usize,
    /// 快照时刻的未跟踪文件总数（含未备份的）。
    pub untracked_total: usize,
    /// 因超限或复制失败而**没有**进备份的未跟踪路径。
    pub skipped: Vec<String>,
    /// 如实告警。
    pub warnings: Vec<SnapshotWarning>,
    /// 本次顺手清理掉的旧快照。
    pub pruned: Vec<SnapshotId>,
}

impl SnapshotOutcome {
    /// 只有 id 的最小结果（"没有内容备份"的实现与测试替身用）。
    pub const fn bare(id: SnapshotId) -> Self {
        Self {
            id,
            backup_bytes: 0,
            backed_up: 0,
            untracked_total: 0,
            skipped: Vec::new(),
            warnings: Vec::new(),
            pruned: Vec::new(),
        }
    }

    /// 是否有需要用户看见的告警。
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// 快照的磁盘占用（列表页 / 设置页显示）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotUsage {
    /// 仓库 id。
    pub repo_id: i64,
    /// 快照条数。
    pub snapshot_count: i64,
    /// 备份内容占用的字节数（来自记录，不扫盘）。
    pub backup_bytes: u64,
    /// 单份上限（`0` = 不限制）。
    pub max_snapshot_bytes: u64,
    /// 总占用上限（`0` = 不限制）。
    pub max_repo_bytes: u64,
    /// 磁盘上存在、但数据库里没有对应快照的备份目录（等待清理）。
    pub orphan_dirs: Vec<String>,
}

/// 手动清理的结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CleanupOutcome {
    /// 清理掉的孤儿目录数。
    pub orphans_removed: usize,
    /// 为满足上限而清理掉的快照（LRU）。
    pub reclaimed: Vec<SnapshotId>,
    /// 释放的字节数（孤儿 + 回收的快照）。
    pub freed_bytes: u64,
    /// 清理后该仓库的备份总占用。
    pub remaining_bytes: u64,
}

/// 快照体积预估（危险操作对话框在动手前展示）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotEstimate {
    /// 将被备份的未跟踪文件数（超限时为 0）。
    pub untracked_count: usize,
    /// 它们的字节数。
    pub untracked_bytes: u64,
    /// 被忽略文件的数量（`include_ignored = false` 时不计入备份）。
    pub ignored_count: usize,
    /// 被忽略文件的字节数。
    pub ignored_bytes: u64,
    /// 当前策略是否包含被忽略文件。
    pub include_ignored: bool,
    /// 单份上限（`0` = 不限制）。
    pub limit_bytes: u64,
    /// 本次是否会完整备份未跟踪内容。
    pub within_limit: bool,
    /// 超限时会被跳过（不备份）的文件数。
    pub would_skip: usize,
}

/// 一次回滚的结果。
///
/// # 失败也用它表达（T3.9）
///
/// "回滚没成功"分两种，用户能做的动作完全不同：
///
/// - **自动回退成功**（`outcome = RolledBack`）：仓库已经回到动手之前的样子，
///   用户什么都不用做——报告里说清"失败在哪一步、已经回到哪里"；
/// - **回退也失败**（`outcome = Emergency`）：仓库停在中间态，
///   报告必须带**可执行的恢复指引**（`emergency`），否则用户真的无从下手。
///
/// 只有"连第一步都没开始"（快照不存在、锚点丢失）才返回 `Err`：
/// 那种情况下没有任何阶段性事实可说。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// 被恢复的快照。
    pub restored_snapshot_id: SnapshotId,
    /// 恢复后的 HEAD oid（与快照记录一致，由恢复后的校验保证）。
    pub head_oid: String,
    /// 恢复后的索引树 oid。
    pub index_tree_oid: String,
    /// 回滚前自动打的"保护点"——用户对回滚结果不满意时可以再回到这里。
    pub pre_restore_snapshot_id: Option<SnapshotId>,
    /// 快照时刻的未跟踪文件路径。v1 只记录不恢复（内容备份属 T3.8），
    /// 列在这里是让用户知道"当时有这些文件"。
    pub untracked_paths: Vec<String>,
    /// 从内容备份写回工作区的未跟踪文件数（v1 快照或未备份时为 0）。
    pub untracked_restored: usize,
    /// 没能恢复的未跟踪文件（备份缺失、写不进去）。
    ///
    /// 这类失败**不触发**回退到回滚前快照：HEAD 与索引才是回滚的主体，
    /// 个别文件被占用不该把整次回滚推倒重来——如实列出来即可。
    pub untracked_failed: Vec<String>,
    /// 当前存在、快照里没有的未跟踪文件——**不会被删除**，列出来交给用户决定。
    pub untracked_extra: Vec<String>,
    /// 重新登记回栈上的 stash 条数（T3.11 第 7 条）。
    ///
    /// "放回栈"= `git stash store`：它只登记引用，**不会**把内容解包到工作区，
    /// 因此它与 [`Self::untracked_restored`] 是两个互不相干的事实。
    pub stash_restored: usize,
    /// 没能放回栈的 stash（对象已被 `gc` 回收、oid 认不出）。
    ///
    /// 与未跟踪文件一样**不触发**回退：HEAD 与索引才是回滚的主体。
    pub stash_failed: Vec<String>,
    /// 重新创建出来的本地分支数（T3.11 第 9 条）。
    ///
    /// 只算**当时不存在**的分支：已存在的分支一律不动（移动别人的分支比不恢复更危险）。
    pub branches_restored: usize,
    /// 没能重新创建的分支（`name: 失败原因`）。
    pub branches_failed: Vec<String>,
    /// 恢复后的完整校验是否通过（HEAD / 索引 / 备份内容逐字节）。
    pub verified: bool,
    /// 本次回滚的结局（T3.9）。
    pub outcome: RestoreOutcomeKind,
    /// 各阶段的结果（按执行顺序；未执行到的阶段不出现）。
    pub stages: Vec<StageResult>,
    /// 人话报告行：UI 直接展示，也是写进日志的内容（同一条事实只写一次）。
    pub report_lines: Vec<String>,
    /// 紧急模式的恢复指引；仅在 [`RestoreOutcomeKind::Emergency`] 时出现。
    pub emergency: Option<EmergencyGuidance>,
}

/// 回滚的结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcomeKind {
    /// 全部阶段成功，仓库已回到快照时刻。
    Completed,
    /// 中途失败，但已经自动回退到"回滚前快照"——仓库回到了动手之前。
    RolledBack,
    /// 中途失败且回退也失败：**需要用户介入**，报告里带可执行的恢复指引。
    Emergency,
}

impl RestoreOutcomeKind {
    /// 稳定短名（IPC 契约 + i18n key 后缀）。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::RolledBack => "rolledBack",
            Self::Emergency => "emergency",
        }
    }

    /// 是否算成功（`RolledBack` 不算：包回到动手之前 ≠ 用户要的回滚成功）。
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// 回滚的一个阶段（顺序即执行顺序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreStage {
    /// 给当前状态打"回滚前保护点"。
    Protection,
    /// `git reset --hard`：HEAD 与工作区。
    Head,
    /// `git read-tree`：索引（"已暂存未提交"的内容在这里）。
    Index,
    /// 写回未跟踪内容。
    Untracked,
    /// 用读引擎核对 git 事实。
    Verify,
}

impl RestoreStage {
    /// 全部阶段，按执行顺序。
    pub const ALL: [Self; 5] = [
        Self::Protection,
        Self::Head,
        Self::Index,
        Self::Untracked,
        Self::Verify,
    ];

    /// 稳定短名（IPC 契约 + i18n key 后缀 + `restore_stage` 落库值）。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Protection => "protection",
            Self::Head => "head",
            Self::Index => "index",
            Self::Untracked => "untracked",
            Self::Verify => "verify",
        }
    }

    /// 从落库的短名还原（认不出返回 `None`：损坏的标记不该让启动流程崩掉）。
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stage| stage.key() == key)
    }
}

/// 一个阶段的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageResult {
    /// 阶段。
    pub stage: RestoreStage,
    /// 是否成功。
    pub ok: bool,
    /// 失败原因（人话；成功为 `None`）。
    pub detail: Option<String>,
    /// 该阶段耗时（毫秒）。
    pub duration_ms: i64,
}

/// 紧急模式的恢复指引：**可复制、可执行**。
///
/// 任务书要求"输出可执行的恢复指引（测试中真的执行指引命令并断言仓库可用）"，
/// 所以 `commands` 是纯粹逐条可跑的 `git ...` 命令，不含占位符、不含解释；
/// 解释放在 `notes` 里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmergencyGuidance {
    /// 用户**原本想回到**的快照（报告里显式展示：那是他点下按钮时的心愿）。
    pub snapshot_id: SnapshotId,
    /// 指引命令的目标——通常是"回滚前保护点"。
    ///
    /// 为什么指引退到保护点而不是继续冲原快照：保护点是**动手之前**的状态，
    /// 是最安全的落点；而原快照此刻已经被证明"恢复不动"。先把仓库放回干净状态，
    /// 之后可以再来一次有保护点的回滚。
    pub target_snapshot_id: SnapshotId,
    /// 目标快照的内容备份目录（可能已被清理，缺失时也如实给出路径）。
    pub backup_dir: Option<String>,
    /// 按顺序执行的 git 命令。
    pub commands: Vec<String>,
    /// 说明：这些命令做什么、为什么、做完之后怎么办。
    pub notes: Vec<String>,
}

/// 一次未完成回滚的标记（崩溃恢复）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRestore {
    /// 当时正在回滚的快照。
    pub snapshot_id: SnapshotId,
    /// 落库的阶段短名（认不出时为 `None`）。
    pub stage: Option<RestoreStage>,
    /// 开始时间（Unix 毫秒）。
    pub started_at_ms: Option<i64>,
}

/// 快照的保留策略：**两个条件先到者生效**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// 每个仓库最多保留的条数。
    pub max_count: u32,
    /// 超过这个天数（按创建时间）的快照可被清理。
    pub max_age_days: u32,
}

impl Default for RetentionPolicy {
    /// 任务定义的默认值：每仓库 50 条或 30 天，先到者生效。
    fn default() -> Self {
        Self {
            max_count: DEFAULT_MAX_COUNT,
            max_age_days: DEFAULT_MAX_AGE_DAYS,
        }
    }
}

impl RetentionPolicy {
    /// 截止时间（Unix 毫秒）：早于它的快照可被清理。
    pub fn cutoff_ms(&self, now_ms: i64) -> i64 {
        now_ms - i64::from(self.max_age_days) * 24 * 60 * 60 * 1000
    }
}

/// 快照相关失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// 创建快照失败（读状态、写对象库、落库）。
    Failed(String),
    /// 快照记录不存在，或不属于指定的仓库。
    NotFound(SnapshotId),
    /// 锚点 ref 丢失——快照不可恢复。
    RefMissing {
        /// 快照 id。
        id: SnapshotId,
        /// 本应存在的 ref 名。
        name: String,
    },
    /// 恢复后的校验不通过。**发生时已自动回到回滚前快照**，仓库不会停在中间态。
    RestoreVerify(String),
    /// 恢复过程失败（git 报错等）。发生时同样先尝试回到回滚前快照。
    RestoreFailed(String),
    /// 数据库读写失败。
    Storage(String),
}

impl SnapshotError {
    /// 开发者可读的描述（英文；用户可见文案由前端按错误码走 i18n）。
    pub fn message(&self) -> String {
        match self {
            Self::Failed(detail) => format!("snapshot creation failed: {detail}"),
            Self::NotFound(id) => format!("snapshot {id} does not exist"),
            Self::RefMissing { id, name } => {
                format!("snapshot {id} is not restorable: anchor ref {name} is missing")
            }
            Self::RestoreVerify(detail) => {
                format!("restore verification failed: {detail}")
            }
            Self::RestoreFailed(detail) => format!("restore failed: {detail}"),
            Self::Storage(detail) => format!("snapshot storage failed: {detail}"),
        }
    }

    /// 稳定错误码（命令层转 `AppError` 时用；用户可见文案由前端给）。
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::NotFound(_) | Self::RefMissing { .. } => ErrorCode::NotFound,
            Self::RestoreVerify(_) => ErrorCode::RestoreVerifyFailed,
            Self::Failed(_) | Self::RestoreFailed(_) | Self::Storage(_) => ErrorCode::Internal,
        }
    }
}

/// 快照管理器。
///
/// 调用方（`services` 层）只依赖这个 trait；`create` 的调用点在 T1.7 就已就位
/// （提交执行前），换实现不需要动那条链路。
///
/// 要求 `Debug`：命令层的 `AppState` 持有 `Arc<dyn SnapshotManager>` 并派生 `Debug`
/// （启动与排查时能一眼看清状态对象的组成）。实现者多写一个 `#[derive(Debug)]`
/// 远比让状态结构丢掉可调试性划算。
pub trait SnapshotManager: Send + Sync + std::fmt::Debug {
    /// 创建一个快照，返回它的 id 与内容备份的实情。
    ///
    /// 空仓库（还没有 HEAD 提交）会失败：没有提交就没有可锚定的对象，
    /// 这种状态下"快照"是空洞的——调用方应当继续原操作并自行降级提示。
    ///
    /// 未跟踪内容备份**不是**失败点：目录不可写、体积超限、个别文件被占用
    /// 都在 [`SnapshotOutcome::warnings`] 里如实报告，快照本身照常创建——
    /// "没有把某个文件备上"远好于"因为某个文件而让这次危险操作裸奔"。
    fn create(&self, request: &SnapshotRequest<'_>) -> Result<SnapshotOutcome, SnapshotError>;

    /// 某个仓库的快照列表（新的在前）。
    fn list(&self, repo_id: i64, limit: i64) -> Result<Vec<SnapshotMeta>, SnapshotError>;

    /// 预估下一次快照会备份多少未跟踪内容（危险操作对话框在动手前展示）。
    fn estimate(&self, repo_id: i64) -> Result<SnapshotEstimate, SnapshotError>;

    /// 快照的磁盘占用（列表页 / 设置页显示）。
    fn usage(&self, repo_id: i64) -> Result<SnapshotUsage, SnapshotError>;

    /// 立即清理：孤儿目录 + 保留策略 + 总占用上限（设置页的"清理"按钮）。
    fn cleanup(&self, repo_id: i64) -> Result<CleanupOutcome, SnapshotError>;

    /// 回滚到指定快照，返回恢复报告。
    ///
    /// 流程（顺序即语义）：校验锚点 → 给当前状态打"回滚前快照" → `reset --hard`
    /// → `read-tree` 恢复索引 → 用 git 的事实核对 → 对不上就回到回滚前快照。
    fn restore(
        &self,
        repo_id: i64,
        snapshot_id: SnapshotId,
    ) -> Result<RestoreReport, SnapshotError>;

    /// 快照与当前状态的差异摘要（列表页的"这个快照还有意义吗"）。
    fn diff(&self, repo_id: i64, snapshot_id: SnapshotId) -> Result<SnapshotDiff, SnapshotError>;

    /// 按保留策略清理，返回被清理的快照 id。
    fn prune(
        &self,
        repo_id: i64,
        policy: &RetentionPolicy,
    ) -> Result<Vec<SnapshotId>, SnapshotError>;

    /// 是否有未完成的回滚（T3.9 崩溃恢复）。
    ///
    /// 宿主启动或打开仓库时查询：`Ok(Some(_))` = 上次回滚没走完，
    /// 界面据此提示"继续 / 查看详情 / 放弃"。
    ///
    /// **有默认实现**（返回"没有"）：快照能力之外的替身（测试夹具、
    /// 将来的远端快照实现）不该被迫写一个空方法；对它们而言"没有未完成回滚"
    /// 也正是事实。
    fn pending_restore(&self, _repo_id: i64) -> Result<Option<PendingRestore>, SnapshotError> {
        Ok(None)
    }

    /// 放弃未完成的回滚标记（用户选择"放弃"），返回清掉的标记数。
    ///
    /// **不回退任何东西**：它只是把"上次没走完"标记为已处理——
    /// 用户之所以能选它，是因为界面已经把当时的快照 ID 与阶段摆在他面前了。
    fn abandon_restore(&self, _repo_id: i64) -> Result<usize, SnapshotError> {
        Ok(0)
    }

    /// 这些快照现在**还能不能回滚**（锚点 ref 是否仍在），返回仍可回滚的子集。
    ///
    /// 锚点会消失（外部 clone、`git gc`、手工删 ref），而记录还在：操作历史
    /// 若不核对就给出一排"回滚"按钮，用户点下去只会收到一个到不了的目标。
    ///
    /// 默认实现返回**空**：没有快照能力的实现如实回答"一个都回滚不了"，
    /// 比乐观地说"都可以"安全得多——后者会让界面给出一堆必然失败的按钮。
    fn restorable(
        &self,
        _repo_id: i64,
        _snapshot_ids: &[SnapshotId],
    ) -> Result<Vec<SnapshotId>, SnapshotError> {
        Ok(Vec::new())
    }
}

/// 未启用快照的实现。
///
/// 现在只出现在测试里（真实现随 T1.9 落地、由宿主注入）。它**不会**假装成功：
/// `Err(NotFound)` 与"创建了快照"在调用方是可区分的。保留它是因为
/// "需要注入一个必然失败的快照管理器来测降级路径"这件事永远存在。
#[derive(Debug, Default)]
pub struct NoopSnapshotManager;

impl SnapshotManager for NoopSnapshotManager {
    fn create(&self, _request: &SnapshotRequest<'_>) -> Result<SnapshotOutcome, SnapshotError> {
        Err(SnapshotError::Failed(
            "snapshotting is disabled by configuration".to_owned(),
        ))
    }

    fn list(&self, _repo_id: i64, _limit: i64) -> Result<Vec<SnapshotMeta>, SnapshotError> {
        Ok(Vec::new())
    }

    /// 只读探测不失败：没有快照 = 没有内容要备，界面照常显示"0 字节"。
    fn estimate(&self, _repo_id: i64) -> Result<SnapshotEstimate, SnapshotError> {
        Ok(SnapshotEstimate {
            untracked_count: 0,
            untracked_bytes: 0,
            ignored_count: 0,
            ignored_bytes: 0,
            include_ignored: false,
            limit_bytes: 0,
            within_limit: true,
            would_skip: 0,
        })
    }

    fn usage(&self, repo_id: i64) -> Result<SnapshotUsage, SnapshotError> {
        Ok(SnapshotUsage {
            repo_id,
            snapshot_count: 0,
            backup_bytes: 0,
            max_snapshot_bytes: 0,
            max_repo_bytes: 0,
            orphan_dirs: Vec::new(),
        })
    }

    fn cleanup(&self, _repo_id: i64) -> Result<CleanupOutcome, SnapshotError> {
        Ok(CleanupOutcome {
            orphans_removed: 0,
            reclaimed: Vec::new(),
            freed_bytes: 0,
            remaining_bytes: 0,
        })
    }

    fn restore(
        &self,
        _repo_id: i64,
        snapshot_id: SnapshotId,
    ) -> Result<RestoreReport, SnapshotError> {
        Err(SnapshotError::NotFound(snapshot_id))
    }

    fn diff(&self, _repo_id: i64, snapshot_id: SnapshotId) -> Result<SnapshotDiff, SnapshotError> {
        Err(SnapshotError::NotFound(snapshot_id))
    }

    fn prune(
        &self,
        _repo_id: i64,
        _policy: &RetentionPolicy,
    ) -> Result<Vec<SnapshotId>, SnapshotError> {
        Ok(Vec::new())
    }
}

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        BackupEntry, BackupManifest, BranchRef, NoopSnapshotManager, RetentionPolicy, SnapshotKind,
        SnapshotLimits, SnapshotManager, SnapshotRequest, SnapshotWarning, StashedRef,
    };
    use std::path::Path;

    #[test]
    fn the_noop_manager_fails_loudly_instead_of_pretending_to_succeed() {
        let manager = NoopSnapshotManager;
        let request = SnapshotRequest {
            repo_id: 1,
            workdir: Path::new("/tmp/repo"),
            label: "pre-commit",
            kind: SnapshotKind::PreCommit,
        };

        // "未启用"必须是显式的失败：调用方据此走降级提示，而不是以为有快照
        assert!(manager.create(&request).is_err());
        assert!(manager.list(1, 10).unwrap().is_empty());
        assert!(manager
            .prune(1, &RetentionPolicy::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn snapshot_kinds_have_stable_keys() {
        assert_eq!(SnapshotKind::PreCommit.key(), "pre-commit");
        assert_eq!(SnapshotKind::PreRestore.key(), "pre-restore");
        assert_eq!(SnapshotKind::Manual.key(), "manual");
        assert_eq!(SnapshotKind::PreSync.key(), "pre-sync");
        assert_eq!(SnapshotKind::PreHeadMove.key(), "pre-head-move");
    }

    #[test]
    fn the_default_retention_policy_matches_the_task_definition() {
        let policy = RetentionPolicy::default();
        assert_eq!(policy.max_count, 50);
        assert_eq!(policy.max_age_days, 30);

        // 截止时间的数学要经得起时区无关的检查：30 天前
        let cutoff = policy.cutoff_ms(1_700_000_000_000);
        assert_eq!(cutoff, 1_700_000_000_000 - 30 * 24 * 60 * 60 * 1000);
    }

    #[test]
    fn the_default_limits_match_the_task_definition() {
        let limits = SnapshotLimits::default();
        assert_eq!(limits.max_snapshot_bytes, 200 * 1024 * 1024);
        assert_eq!(limits.max_repo_bytes, 2 * 1024 * 1024 * 1024);
        assert!(!limits.include_ignored, "被忽略文件默认不备份");
    }

    #[test]
    fn a_backup_manifest_round_trips_and_survives_corruption() {
        let manifest = BackupManifest {
            entries: vec![BackupEntry {
                path: "scratch/a.txt".to_owned(),
                bytes: 12,
                ignored: true,
            }],
            bytes: 12,
            stash: vec![StashedRef {
                oid: "a".repeat(40),
                message: "WIP on main: 1a2b3c4 subject".to_owned(),
            }],
            branches: vec![BranchRef {
                name: "gone".to_owned(),
                oid: "b".repeat(40),
            }],
        };
        assert_eq!(BackupManifest::from_json(&manifest.to_json()), manifest);

        // 清单损坏不能让回滚失败（HEAD 与索引的恢复与它无关）：
        // 读成空清单，未跟踪内容恢复不了会在报告里如实体现
        assert!(BackupManifest::from_json("not json").is_empty());
        assert!(BackupManifest::from_json(r#"{"entries":[]}"#).is_empty());
        assert!(BackupManifest::empty().is_empty());
    }

    #[test]
    fn snapshot_warnings_have_stable_kinds() {
        assert_eq!(
            SnapshotWarning::UntrackedBackupSkipped {
                count: 1,
                bytes: 2,
                limit: 3
            }
            .kind(),
            "untrackedBackupSkipped"
        );
        assert_eq!(
            SnapshotWarning::UntrackedBackupPartial {
                paths: vec!["a".to_owned()],
                detail: "busy".to_owned()
            }
            .kind(),
            "untrackedBackupPartial"
        );
        assert_eq!(
            SnapshotWarning::SpaceReclaimed {
                removed: vec![1],
                freed_bytes: 2
            }
            .kind(),
            "spaceReclaimed"
        );
        assert_eq!(
            SnapshotWarning::OrphansRemoved { count: 1 }.kind(),
            "orphansRemoved"
        );
        assert_eq!(
            SnapshotWarning::BackupDirUnavailable {
                detail: "no root".to_owned()
            }
            .kind(),
            "backupDirUnavailable"
        );
    }
}
