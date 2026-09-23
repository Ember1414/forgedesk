//! Tauri IPC 命令层。
//!
//! 归属里程碑：M0 / T0.2（建立最小 IPC 通路）、后续里程碑持续扩充。
//!
//! # 职责边界
//!
//! 本层**只做四件事**，不得包含业务逻辑：
//!
//! 1. 接收并校验参数（所有外部输入都被视为不可信）。
//! 2. 声明并校验能力等级（ReadOnly / Mutating / Network / Dangerous，见 docs/PLAN.md §5.12）。
//! 3. 把 `services` 层的结果转换为前端 DTO。
//! 4. 把错误转换为 [`forgedesk_domain::AppError`]。
//!
//! 真实的业务编排在 `forgedesk-services`，纯逻辑在 `forgedesk-domain`。
//!
//! # 两条硬约定
//!
//! - **命令必须定义在子模块中**，由本文件重导出。原因是 `#[tauri::command]`
//!   会生成落在 crate 根的导出宏，若命令函数也在 crate 根会触发 E0255 命名冲突。
//! - 命令命名：`<domain>_<action>`（例如 `repo_open`、`git_commit_execute`）；
//!   返回值一律 `AppResult<T>`；每个新增命令都必须在 `docs/API.md` 中登记
//!   （能力等级、参数、返回、错误码）。

#![forbid(unsafe_code)]

pub mod system;

pub use system::{app_version, AppVersion};
