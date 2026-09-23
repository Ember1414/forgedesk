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
//! 本模块**不**定义 IPC 序列化契约（那是 T1.2/T1.4 的事），因此这里的类型暂不派生
//! `Serialize`：字节路径的线上表示需要与前端一起定（lossy 字符串 + "是否合法 UTF-8"标记）。

pub mod commit;
pub mod diff;
pub mod index;
pub mod path;
pub mod status;

pub use commit::{Commit, SignatureStatus};
pub use diff::FileStat;
pub use index::{StageEntry, UnmergedEntry, UnmergedStage};
pub use path::RepoPath;
pub use status::{
    BranchInfo, ChangeKind, ConflictStages, EntryKind, FileChange, StatusReport, SubmoduleState,
};
