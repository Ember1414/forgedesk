//! 用例编排层：把领域逻辑与基础设施组合成完整业务动作（打开仓库、提交、同步、冲突解决…）。
//!
//! # 分层位置
//!
//! ```text
//! commands (IPC)  →  services (本层)  →  domain (纯逻辑)
//!                                     →  infra crates (git-engine / storage / jobs / …)
//! ```
//!
//! 本层是**唯一同时持有多个 infra crate** 的地方，因此也是唯一需要做
//! "读走 libgit2、写走 CLI"这类编排决策的地方（见 [`engines::GitEngines`]）。
//!
//! # 本层不做什么
//!
//! - **不产出用户可见文案**：错误分类在 domain，脱敏在 diagnostics，
//!   中英文界面文案由前端按错误码走 i18n（`docs/API.md` §1.1）。
//! - **不做参数校验**：外部输入由 `commands` 层收敛到已知取值（IPC 边界），
//!   本层假定收到的领域类型已经是合法的。
//! - **不感知"任务"**：阻塞的用例（clone / init）由调用方放进
//!   [`forgedesk_jobs::JobRunner`]，本层只管"把这一步做对"。

#![forbid(unsafe_code)]

pub mod engines;
pub mod repository;
pub mod templates;
pub mod workspace;

pub use engines::GitEngines;
pub use repository::{
    InitExtras, LicenseSpec, MillisClock, OpenRepoRegistry, OpenedRepository, RecentRepository,
    RepositoryService,
};
pub use templates::{sanitize_holder, GitignoreTemplate, LicenseTemplate};
pub use workspace::WorkspaceService;

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
