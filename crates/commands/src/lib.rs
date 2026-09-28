//! Tauri IPC 命令层。
//!
//! 归属里程碑：M0 / T0.2（建立最小 IPC 通路）、T0.6（统一错误转换层）、
//! T0.7（设置读写）、T0.8（日志查看）、后续里程碑持续扩充。
//!
//! # 职责边界
//!
//! 本层**只做四件事**，不得包含业务逻辑：
//!
//! 1. 接收并校验参数（所有外部输入都被视为不可信）。
//! 2. 声明并校验能力等级（ReadOnly / Mutating / Network / Dangerous，见 docs/PLAN.md §5.12）。
//! 3. 把 `services` 层的结果转换为前端 DTO。
//! 4. 把错误转换为 [`forgedesk_domain::AppError`]（统一走 [`error::to_app_error`]）。
//!
//! 真实的业务编排在 `forgedesk-services`，纯逻辑在 `forgedesk-domain`，
//! 数据存取在 `forgedesk-storage`，与操作系统打交道在 `forgedesk-platform`。
//!
//! # 三条硬约定
//!
//! - **命令必须定义在子模块中**，由本文件重导出。原因是 `#[tauri::command]`
//!   会生成落在 crate 根的导出宏，若命令函数也在 crate 根会触发 E0255 命名冲突。
//! - 命令命名：`<domain>_<action>`（例如 `repo_open`、`settings_get`、`logs_tail`）；
//!   返回值一律 `AppResult<T>`；每个新增命令都必须在 `docs/API.md` 中登记
//!   （能力等级、参数、返回、错误码）。
//! - 错误一律经 [`error::to_app_error`] 转换：分类在领域层、脱敏在 diagnostics，
//!   命令层不得自行拼装用户可见文案。

#![forbid(unsafe_code)]

pub mod audit;
pub mod commit;
pub mod commit_detail;
pub mod debug;
pub mod error;
pub mod history;
pub mod jobs;
pub mod logs;
pub mod repository;
pub mod settings;
pub mod snapshots;
pub mod state;
pub mod system;
pub mod watch;
pub mod workspace;

pub use audit::{
    audit_export, audit_list, audit_prune, AuditEntryDto, AuditExportDto, AuditPageDto,
    AuditPruneDto,
};
pub use commit::{
    commit_amend_context, commit_execute, commit_hooks_list, commit_message_hint, commit_prepare,
    AmendContextDto, CommitOutcomeDto, CommitPlanDto, HookEntryDto, IdentityDto, IdentityRequest,
    MessageHintDto, PlannedFileDto, PrepareCommitRequest,
};
pub use commit_detail::git_commit_detail;
pub use debug::{debug_panic, debug_throw_error};
pub use error::{to_app_error, Fallible};
pub use history::git_log_page;
pub use jobs::{job_cancel, TauriJobReporter};
pub use logs::{logs_open, logs_tail};
pub use repository::{
    repo_clone, repo_close, repo_discover, repo_forget, repo_init, repo_open, repo_recent_list,
    AuditFindingDto, BranchLabelDto, CloneRequest, InitRequest, JobIdDto, OpenedRepositoryDto,
    RecentRepositoryDto, RepoAuditDto, RepositoryDto, WorktreeDto,
};
pub use settings::{settings_all, settings_get, settings_set};
pub use snapshots::{
    snapshot_diff, snapshot_list, snapshot_prune, snapshot_restore, RestoreReportDto,
    SnapshotDiffDto, SnapshotMetaDto,
};
pub use state::AppState;
pub use system::log_frontend_error;
pub use system::{app_version, AppVersion};
pub use watch::{emit_watch_event, WatchSettings, WatcherRegistry, AUTO_REFRESH_KEY, DEBOUNCE_KEY};
pub use workspace::{
    workspace_diff, workspace_diff_patch, workspace_discard, workspace_reveal, workspace_stage,
    workspace_status, workspace_unstage, StatusReportDto, EVENT_REPO_CHANGED,
};
