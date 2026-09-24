//! Git 机器可读输出的解析器。
//!
//! 三条硬规则（AGENTS.md §7 与 T1.1 提示词）：
//!
//! 1. **输入一律是字节切片**，不是 `&str`。Git 的路径是字节串，先 `lossy` 再解析
//!    等于把非 UTF-8 文件名换成一个**不存在**的路径，而且不会报错。
//! 2. **只解析机器可读格式**。每个解析器对应一条固定的命令与格式串
//!    （见各模块顶部），人类可读输出（`git status` 默认格式、`git log --oneline`）
//!    一律不解析——它们的排版会随版本与 locale 变化。
//! 3. **永不 panic**。畸形输入（被截断的管道、git 版本差异、编码错误）只应导致
//!    "该条记录被跳过"，不该让整个状态面板空白。因此解析器返回尽力而为的结果，
//!    并且每个解析器都有"随机字节不 panic"的测试（见 `tests/parsers_fixtures.rs`）。
//!
//! 解析器**不做 IO**：它们只把字节变成 [`forgedesk_domain::git`] 里的领域类型，
//! 因此可以用固定样本（`tests/fixtures/`）完整覆盖边界。

mod common;
pub mod config;
pub mod diff;
pub mod log;
pub mod ls_files;
pub mod status;
pub mod unified_diff;
pub mod worktree;

pub use config::parse_config_list;
pub use diff::parse_diff_numstat;
pub use log::{parse_log_format, parse_show_format, LOG_FORMAT, SHOW_FORMAT};
pub use ls_files::parse_ls_files_stage;
pub use status::parse_status_porcelain_v2;
pub use unified_diff::{parse_unified_diff, ParsedPatchSection, PatchLimits};
pub use worktree::parse_worktree_list;
