//! Git 领域模型：与 IO 无关的纯数据结构。
//!
//! 谁产出、谁消费：
//!
//! - `crates/git-engine` 的解析器把 git 的机器可读输出**解析成这里的类型**；
//! - `crates/services` 基于这些类型编排用例；
//! - `crates/commands` 把它们映射成 IPC DTO。
//!
//! # 两个刻意为之的取舍
//!
//! 1. **路径保真**：Git 的路径在 POSIX 上是任意字节（只要不含 NUL），
//!    中文/日文仓库里"文件名不是合法 UTF-8"很常见。因此路径一律用 [`RepoPath`]
//!    保存原始字节，只在展示时 lossy。若在解析阶段就 lossy，后续的文件系统操作
//!    会拿着一个被替换成 `U+FFFD` 的假路径去执行，而这类错误极难定位。
//! 2. **提交元数据 lossy**：作者名、邮箱、subject 允许任意编码（Git 有 `encoding` 头），
//!    界面无法渲染任意编码，所以这些字段统一 lossy —— 与路径相反，它们不参与
//!    文件系统操作，丢掉真值不会造成数据损坏。
//!
//! # 规格（Spec）与结果（Outcome）成对出现
//!
//! 写操作的参数与返回值都建模在 [`spec`] 里。理由见该模块头：这些结构体会被
//! 整体脱敏后写进审计日志与快照标签（红线 R7），散装参数做不到这一点。
//!
//! # 本模块不定义 IPC 序列化契约
//!
//! 这些类型暂不派生 `Serialize`：字节路径的线上表示需要与前端一起定
//! （lossy 字符串 + "是否合法 UTF-8"标记），那是 T1.4 的 DTO 层工作。
//! 现在随便定一个形状，只会在前端接上时返工。

pub mod audit;
pub mod commit;
pub mod commit_plan;
pub mod diff;
pub mod index;
pub mod path;
pub mod query;
pub mod refs;
pub mod repository;
pub mod spec;
pub mod staging;
pub mod stash;
pub mod status;
pub mod version;

pub use audit::{
    audit_config, AuditFinding, AuditFindingId, AuditSeverity, ConfigEntry, ConfigScope,
    RepoAuditReport,
};
pub use commit::{Commit, Signature, SignatureStatus};
pub use commit_plan::{
    compose_message, equivalent_command, review_message, CommitPlan, EquivalentCommandInput,
    MessageIssue, MessageReview, PlannedFile, SignMode, COMMIT_PLAN_TTL_MS, EMPTY_TREE_OID,
    EQUIVALENT_COMMAND_FILE_LIMIT, SUBJECT_RECOMMENDED_MAX_CHARS,
};
pub use diff::{
    DiffChangeKind, DiffHunk, DiffLine, DiffLineKind, DiffReport, DiffSpec, DiffTarget, FileDiff,
    FileStat, DEFAULT_CONTEXT_LINES, MAX_DIFF_BYTES_PER_FILE, MAX_DIFF_LINES_PER_FILE,
};
pub use index::{StageEntry, UnmergedEntry, UnmergedStage};
pub use path::RepoPath;
pub use query::{summarize_authors, AuthorSummary, LogQuery, Page};
pub use refs::{Branch, RefUpdate, RefUpdateKind, Remote, RemoteKind, Tag};
pub use repository::{BranchLabel, RepoId, RepositoryInfo, Worktree};
pub use spec::{
    validate_ref_name, AmendMode, ApplyDirection, ApplyPatchSpec, ApplyTarget, BranchCreateSpec,
    BranchDeleteSpec, BranchRenameSpec, BranchSetUpstreamSpec, CheckoutSpec, CloneSpec, CommitSpec,
    DiscardSpec, FetchOutcome, FetchSpec, InitSpec, MergeKind, MergeOutcome, MergeSpec,
    PullOutcome, PullSpec, PullStrategy, PushOutcome, PushRejection, PushSpec, ReflogEntry,
    ReorderAction, ReorderSpec, ReorderStep, ResetMode, ResetSpec, StageSpec, StashAction,
    StashSpec, SwitchStrategy, TagCreateSpec, TagDeleteSpec,
};
pub use staging::{trim_patch, LineSelection, PatchDirection, StageGranularity, StageScope};
pub use stash::StashEntry;
pub use status::{
    BranchInfo, ChangeKind, ConflictStages, EntryKind, FileChange, OperationState, StatusQuery,
    StatusReport, SubmoduleState,
};
pub use version::{GitVersion, MINIMUM_GIT_VERSION};
