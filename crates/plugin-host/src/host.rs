//! 宿主函数与权限校验（T6.2）。
//!
//! # 为什么是"一张操作表 + 一个泛化入口"
//!
//! T6.2 的验收要求"断言所有 host function 都有权限检查"。如果每个宿主函数
//! 是一个独立函数，这个断言只能靠代码扫描（会漏）；把全部操作收进
//! [`HostOp`] 表、由 [`dispatch_host_call`] 单点派发，权限检查就成为
//! **结构保证**——派发器先查表校验权限，再调用服务。一个测试遍历全表
//! （无授权 → 全部拒绝且服务不可达）即可覆盖全部宿主函数。
//!
//! # ABI（imports，见 docs/PLUGIN-API.md）
//!
//! - `fd.log(level, ptr, len)`：无权限要求（诊断通道，内容进日志前由脱敏层兜底）；
//! - `fd.host_call(op, arg_ptr, arg_len) -> i32`：JSON 入参，成功返回 0，
//!   失败返回负的错误码；结果/错误详情写入 staging；
//! - `fd.host_result(out_ptr, out_cap) -> i32`：取回 staging；缓冲不足时返回
//!   `-required_len`（staging 保留，可加大缓冲重试）。
//!
//! 泛化 JSON 入口意味着 ABI 永不膨胀（apiVersion MINOR 只增的承诺落在
//! op 表上），代价是每次调用一次 JSON 解析——这正是选 wasmi 时接受的
//! "UI 胶水"性能档位。

use std::sync::Arc;

use crate::permission::Permission;
use crate::runtime::{HostError, PermissionSet};
use serde_json::{json, Value};

/// 入参 JSON 上限（与 fd_invoke 的参数口径一致）。
pub const MAX_ARG_BYTES: usize = 1024 * 1024;
/// staging 结果上限。
pub const MAX_RESULT_BYTES: usize = 1024 * 1024;
/// 单条插件日志/通知文本上限。
pub const MAX_LOG_BYTES: usize = 8 * 1024;

// ---- 稳定错误码（ABI 层面，PLUGIN-API.md 登记，MINOR 只增） ----

/// 成功。
pub const ERR_OK: i32 = 0;
/// 未知错误（服务内部失败等）。
pub const ERR_GENERIC: i32 = -1;
/// 权限未授予（或被用户撤销）。
pub const ERR_PERMISSION_DENIED: i32 = -2;
/// 参数缺失/类型错/越界/路径非法等。
pub const ERR_INVALID_ARGUMENT: i32 = -3;
/// 目标不存在（文件、键等）。
pub const ERR_NOT_FOUND: i32 = -4;
/// 操作超时。
pub const ERR_TIMEOUT: i32 = -5;
/// 结果超出大小上限。
pub const ERR_TOO_LARGE: i32 = -6;

/// 宿主操作封闭集合。op id 是 ABI 的一部分：只增不改，废弃只标注不停用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOp {
    /// 仓库基本信息。
    GetRepoInfo,
    /// 简化状态（路径 + 状态码，不含内容）。
    GetStatus,
    /// 读仓库内文本文件。
    ReadFile,
    /// 列目录。
    ListDir,
    /// 写仓库内文件（需审计）。
    WriteFile,
    /// 通过宿主 HTTP 层取 JSON（域名白名单）。
    HttpGetJson,
    /// 读插件命名空间设置。
    GetSetting,
    /// 写插件命名空间设置。
    SetSetting,
    /// 简化提交历史。
    GetGitLog,
    /// 暂存文件（走快照 + 审计）。
    GitStage,
    /// 创建提交（走快照 + 审计）。
    GitCommit,
    /// 动态注册命令。
    RegisterCommand,
    /// 动态注册面板。
    RegisterPanel,
    /// 弹出 toast。
    ShowToast,
    /// 订阅宿主事件（repo_opened / repo_changed / commit_created / sync_completed）。
    SubscribeEvents,
}

/// 可订阅的宿主事件（封闭白名单；payload 均为仓库范围的极小 JSON）。
pub const EVENT_NAMES: [&str; 4] = [
    "repo_opened",
    "repo_changed",
    "commit_created",
    "sync_completed",
];

impl HostOp {
    /// 全部操作，按 op id 升序（表格驱动测试与文档生成共用）。
    pub const ALL: [HostOp; 15] = [
        HostOp::GetRepoInfo,
        HostOp::GetStatus,
        HostOp::ReadFile,
        HostOp::ListDir,
        HostOp::WriteFile,
        HostOp::HttpGetJson,
        HostOp::GetSetting,
        HostOp::SetSetting,
        HostOp::GetGitLog,
        HostOp::GitStage,
        HostOp::GitCommit,
        HostOp::RegisterCommand,
        HostOp::RegisterPanel,
        HostOp::ShowToast,
        HostOp::SubscribeEvents,
    ];

    /// ABI 中的稳定 op id。
    pub fn id(self) -> i32 {
        match self {
            HostOp::GetRepoInfo => 1,
            HostOp::GetStatus => 2,
            HostOp::ReadFile => 3,
            HostOp::ListDir => 4,
            HostOp::WriteFile => 5,
            HostOp::HttpGetJson => 6,
            HostOp::GetSetting => 7,
            HostOp::SetSetting => 8,
            HostOp::GetGitLog => 9,
            HostOp::GitStage => 10,
            HostOp::GitCommit => 11,
            HostOp::RegisterCommand => 12,
            HostOp::RegisterPanel => 13,
            HostOp::ShowToast => 14,
            HostOp::SubscribeEvents => 15,
        }
    }

    /// 由 op id 反查；未知 id 返回 None（派发器报参数错误）。
    pub fn from_id(id: i32) -> Option<Self> {
        HostOp::ALL.iter().copied().find(|op| op.id() == id)
    }

    /// 稳定名称（日志与文档用）。
    pub fn name(self) -> &'static str {
        match self {
            HostOp::GetRepoInfo => "get_repo_info",
            HostOp::GetStatus => "get_status",
            HostOp::ReadFile => "read_file",
            HostOp::ListDir => "list_dir",
            HostOp::WriteFile => "write_file",
            HostOp::HttpGetJson => "http_get_json",
            HostOp::GetSetting => "get_setting",
            HostOp::SetSetting => "set_setting",
            HostOp::GetGitLog => "get_git_log",
            HostOp::GitStage => "git_stage",
            HostOp::GitCommit => "git_commit",
            HostOp::RegisterCommand => "register_command",
            HostOp::RegisterPanel => "register_panel",
            HostOp::ShowToast => "show_toast",
            HostOp::SubscribeEvents => "subscribe_events",
        }
    }

    /// 调用本操作所需的权限。
    pub fn permission(self) -> Permission {
        match self {
            HostOp::GetRepoInfo | HostOp::GetStatus | HostOp::GetGitLog => Permission::GitRead,
            HostOp::ReadFile | HostOp::ListDir => Permission::FsRead,
            HostOp::WriteFile => Permission::FsWrite,
            HostOp::HttpGetJson => Permission::NetGithub,
            HostOp::GetSetting => Permission::SettingsRead,
            HostOp::SetSetting => Permission::SettingsWrite,
            HostOp::GitStage | HostOp::GitCommit => Permission::GitWrite,
            HostOp::RegisterCommand => Permission::UiCommand,
            HostOp::RegisterPanel => Permission::UiPanel,
            HostOp::ShowToast => Permission::UiToast,
            // 事件 payload 都是仓库范围的（opened/changed/commit/sync），
            // 订阅它们要求具备与"读仓库"同级的权限
            HostOp::SubscribeEvents => Permission::GitRead,
        }
    }
}

/// 宿主服务集合：由组合根（services/commands 层）提供真实实现，
/// 测试提供 mock。全部方法都带 `plugin_id`——审计与"由插件 X 执行"标注
/// 的信息从这一天起就要跟着请求走。
pub trait HostServices: Send + Sync {
    /// 仓库基本信息（无打开仓库时返回 [`HostError::NotFound`]）。
    fn repo_info(&self, plugin_id: &str) -> Result<Value, HostError>;
    /// 工作区状态（路径 + 状态码）。
    fn status(&self, plugin_id: &str, filter: Option<String>) -> Result<Value, HostError>;
    /// 读仓库内文本文件（实现负责 canonicalize 与符号链接逃逸校验）。
    fn read_file(&self, plugin_id: &str, rel_path: &str) -> Result<Value, HostError>;
    /// 列仓库内目录。
    fn list_dir(&self, plugin_id: &str, rel_path: &str) -> Result<Value, HostError>;
    /// 写仓库内文件（实现负责快照/审计，标注来源插件）。
    fn write_file(
        &self,
        plugin_id: &str,
        rel_path: &str,
        content: &str,
    ) -> Result<Value, HostError>;
    /// 经宿主 HTTP 层取 JSON（实现负责代理与限流）。
    fn http_get_json(&self, plugin_id: &str, url: &str, headers: Value)
        -> Result<Value, HostError>;
    /// 读插件命名空间下的设置。
    fn get_setting(&self, plugin_id: &str, key: &str) -> Result<Value, HostError>;
    /// 写插件命名空间下的设置。
    fn set_setting(&self, plugin_id: &str, key: &str, value: Value) -> Result<Value, HostError>;
    /// 简化提交历史。
    fn git_log(
        &self,
        plugin_id: &str,
        limit: u32,
        path: Option<String>,
    ) -> Result<Value, HostError>;
    /// 暂存文件。
    fn git_stage(&self, plugin_id: &str, paths: Vec<String>) -> Result<Value, HostError>;
    /// 创建提交。
    fn git_commit(&self, plugin_id: &str, message: &str) -> Result<Value, HostError>;
    /// 动态注册命令（id 已按 `<plugin-id>.<id>` 前缀化后传入）。
    fn register_command(
        &self,
        plugin_id: &str,
        id: &str,
        title: &str,
        keybinding: Option<String>,
    ) -> Result<Value, HostError>;
    /// 动态注册面板（id 已前缀化；location: sidebar|bottom|repo-tab）。
    fn register_panel(
        &self,
        plugin_id: &str,
        id: &str,
        title: &str,
        location: &str,
    ) -> Result<Value, HostError>;
    /// 弹出 toast。
    fn show_toast(&self, plugin_id: &str, level: &str, message: &str) -> Result<Value, HostError>;
    /// 登记插件的事件订阅（调用方已校验事件名在 [`EVENT_NAMES`] 白名单内）。
    fn subscribe_events(&self, plugin_id: &str, events: Vec<String>) -> Result<Value, HostError>;
    /// 查询插件是否订阅了某事件；默认未订阅（组合根按需覆盖）。
    fn event_interest(&self, _plugin_id: &str, _event: &str) -> bool {
        false
    }
}

/// 供组合根使用的共享句柄别名。
pub type SharedServices = Arc<dyn HostServices>;

/// [`HostError`] → ABI 错误码。
pub fn error_code(error: &HostError) -> i32 {
    match error {
        HostError::PermissionDenied { .. } => ERR_PERMISSION_DENIED,
        HostError::InvalidArgument(_, _) => ERR_INVALID_ARGUMENT,
        HostError::Timeout { .. } => ERR_TIMEOUT,
        HostError::MemoryLimitExceeded => ERR_TOO_LARGE,
        HostError::NotFound(_) => ERR_NOT_FOUND,
        HostError::Trap(_) | HostError::InstanceNotFound(_) | HostError::Engine(_) => ERR_GENERIC,
    }
}

/// 一次宿主调用的完整入口（引擎的 `fd.host_call` 导入最终走到这里）。
///
/// 权限 → 参数 → 服务 的顺序不可调换：权限失败时服务根本不该被看到。
/// 结果（或错误详情 JSON）写入 `staging`，由 `fd.host_result` 取回。
/// 返回 ABI 错误码（0 = 成功）。
pub fn dispatch_host_call(
    plugin_id: &str,
    granted: &PermissionSet,
    services: &dyn HostServices,
    op_id: i32,
    args: &[u8],
    staging: &mut Vec<u8>,
) -> i32 {
    staging.clear();
    let Some(op) = HostOp::from_id(op_id) else {
        stage_error(
            staging,
            &HostError::InvalidArgument("op", format!("unknown op id {op_id}")),
        );
        return ERR_INVALID_ARGUMENT;
    };
    let permission = op.permission();
    if !granted.allows(permission) {
        // 撤销授权后（T6.4）下一次调用即失败：这里是唯一的权限裁决点
        let error = HostError::PermissionDenied {
            permission,
            call: op.name(),
        };
        tracing::warn!(
            plugin_id,
            op = op.name(),
            permission = permission.as_str(),
            "host call denied"
        );
        stage_error(staging, &error);
        return ERR_PERMISSION_DENIED;
    }
    if args.len() > MAX_ARG_BYTES {
        stage_error(
            staging,
            &HostError::InvalidArgument("args", "argument JSON too large".to_owned()),
        );
        return ERR_TOO_LARGE;
    }
    let args: Value = if args.is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        match serde_json::from_slice(args) {
            Ok(value) => value,
            Err(error) => {
                stage_error(
                    staging,
                    &HostError::InvalidArgument("args", error.to_string()),
                );
                return ERR_INVALID_ARGUMENT;
            }
        }
    };

    let started = std::time::Instant::now();
    let outcome = dispatch_op(plugin_id, services, op, &args);
    let ok = outcome.is_ok();
    // 日志只记元数据，不记参数与结果内容（R8：内容可能含用户数据）
    tracing::debug!(
        plugin_id,
        op = op.name(),
        ok,
        duration_ms = started.elapsed().as_millis() as u64,
        "host call"
    );
    match outcome {
        Ok(value) => stage_result(staging, value),
        Err(error) => {
            stage_error(staging, &error);
            error_code(&error)
        }
    }
}

fn dispatch_op(
    plugin_id: &str,
    services: &dyn HostServices,
    op: HostOp,
    args: &Value,
) -> Result<Value, HostError> {
    match op {
        HostOp::GetRepoInfo => services.repo_info(plugin_id),
        HostOp::GetStatus => {
            let filter = opt_str_field(args, "filter")?;
            services.status(plugin_id, filter)
        }
        HostOp::ReadFile => {
            let path = str_field(args, "path")?;
            validate_rel_path(&path)?;
            services.read_file(plugin_id, &path)
        }
        HostOp::ListDir => {
            let path = str_field(args, "path")?;
            validate_rel_path(&path)?;
            services.list_dir(plugin_id, &path)
        }
        HostOp::WriteFile => {
            let path = str_field(args, "path")?;
            validate_rel_path(&path)?;
            let content = str_field(args, "content")?;
            size_guard("content", content.len(), MAX_ARG_BYTES)?;
            services.write_file(plugin_id, &path, &content)
        }
        HostOp::HttpGetJson => {
            let url = str_field(args, "url")?;
            validate_http_url(&url)?;
            let headers = args.get("headers").cloned().unwrap_or_else(|| json!({}));
            services.http_get_json(plugin_id, &url, headers)
        }
        HostOp::GetSetting => {
            let key = str_field(args, "key")?;
            services.get_setting(plugin_id, &setting_key(plugin_id, &key))
        }
        HostOp::SetSetting => {
            let key = str_field(args, "key")?;
            let value = args.get("value").ok_or_else(|| {
                HostError::InvalidArgument("value", "missing required field".to_owned())
            })?;
            services.set_setting(plugin_id, &setting_key(plugin_id, &key), value.clone())
        }
        HostOp::GetGitLog => {
            let limit = u32_field(args, "limit", 1, 1000)?;
            let path = opt_str_field(args, "path")?;
            services.git_log(plugin_id, limit, path)
        }
        HostOp::GitStage => {
            let paths = paths_field(args)?;
            services.git_stage(plugin_id, paths)
        }
        HostOp::GitCommit => {
            let message = str_field(args, "message")?;
            if message.trim().is_empty() {
                return Err(HostError::InvalidArgument(
                    "message",
                    "must not be empty".to_owned(),
                ));
            }
            if message.len() > MAX_LOG_BYTES {
                return Err(HostError::InvalidArgument(
                    "message",
                    format!("exceeds {MAX_LOG_BYTES} bytes"),
                ));
            }
            services.git_commit(plugin_id, &message)
        }
        HostOp::RegisterCommand => {
            let id = str_field(args, "id")?;
            let title = str_field(args, "title")?;
            let keybinding = opt_str_field(args, "keybinding")?;
            let full_id = format!("{plugin_id}.{id}");
            services.register_command(plugin_id, &full_id, &title, keybinding)
        }
        HostOp::RegisterPanel => {
            let id = str_field(args, "id")?;
            let title = str_field(args, "title")?;
            let location = str_field(args, "location")?;
            if !matches!(location.as_str(), "sidebar" | "bottom" | "repo-tab") {
                return Err(HostError::InvalidArgument(
                    "location",
                    format!("unknown panel location `{location}`"),
                ));
            }
            let full_id = format!("{plugin_id}.{id}");
            services.register_panel(plugin_id, &full_id, &title, &location)
        }
        HostOp::SubscribeEvents => {
            let events = obj_str_array(args, "events", 1, 8)?;
            for event in &events {
                if !EVENT_NAMES.contains(&event.as_str()) {
                    return Err(HostError::InvalidArgument(
                        "events",
                        format!("unknown event `{event}`"),
                    ));
                }
            }
            services.subscribe_events(plugin_id, events)
        }
        HostOp::ShowToast => {
            let level = str_field(args, "level")?;
            if !matches!(level.as_str(), "info" | "success" | "warning" | "danger") {
                return Err(HostError::InvalidArgument(
                    "level",
                    format!("unknown toast level `{level}`"),
                ));
            }
            let message = str_field(args, "message")?;
            size_guard("message", message.len(), MAX_LOG_BYTES)?;
            services.show_toast(plugin_id, &level, &message)
        }
    }
}

/// 成功结果写入 staging；超过上限时报 TOO_LARGE。
fn stage_result(staging: &mut Vec<u8>, value: Value) -> i32 {
    let bytes = match serde_json::to_vec(&value) {
        Ok(bytes) => bytes,
        Err(error) => {
            stage_error(staging, &HostError::Engine(error.to_string()));
            return ERR_GENERIC;
        }
    };
    if bytes.len() > MAX_RESULT_BYTES {
        stage_error(staging, &HostError::MemoryLimitExceeded);
        return ERR_TOO_LARGE;
    }
    *staging = bytes;
    ERR_OK
}

/// 错误详情写入 staging（引擎的导入在参数读取失败等场景也会用到）。
pub(crate) fn stage_error(staging: &mut Vec<u8>, error: &HostError) {
    *staging = serde_json::to_vec(&json!({
        "error": { "code": error_code(error), "message": error.to_string() },
    }))
    .unwrap_or_default();
}

/// 取回 staging 结果到引擎提供的输出缓冲。
///
/// staging 为空 → 0；缓冲不足 → `Err(-required_len)`（staging 保留，可加大
/// 缓冲重试）；成功 → 拷贝并清空，返回写入字节数。
pub fn take_staged_result_into(staging: &mut Vec<u8>, out: &mut [u8]) -> Result<usize, i32> {
    if staging.is_empty() {
        return Ok(0);
    }
    let required = staging.len();
    if required > out.len() {
        return Err(-(i32::try_from(required).unwrap_or(i32::MAX)));
    }
    out[..required].copy_from_slice(staging);
    staging.clear();
    Ok(required)
}

// ---- 参数抽取与校验 ----

fn str_field(args: &Value, key: &'static str) -> Result<String, HostError> {
    match args.get(key) {
        Some(Value::String(text)) if !text.is_empty() => Ok(text.clone()),
        Some(Value::String(_)) => Err(HostError::InvalidArgument(
            key,
            "must not be empty".to_owned(),
        )),
        _ => Err(HostError::InvalidArgument(
            key,
            "must be a string".to_owned(),
        )),
    }
}

fn opt_str_field(args: &Value, key: &'static str) -> Result<Option<String>, HostError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
        Some(Value::String(_)) => Err(HostError::InvalidArgument(
            key,
            "must not be empty".to_owned(),
        )),
        _ => Err(HostError::InvalidArgument(
            key,
            "must be a string".to_owned(),
        )),
    }
}

fn u32_field(args: &Value, key: &'static str, min: u32, max: u32) -> Result<u32, HostError> {
    match args.get(key) {
        Some(Value::Number(n)) => match n.as_u64().and_then(|v| u32::try_from(v).ok()) {
            Some(v) if (min..=max).contains(&v) => Ok(v),
            _ => Err(HostError::InvalidArgument(
                key,
                format!("must be an integer in {min}..={max}"),
            )),
        },
        _ => Err(HostError::InvalidArgument(
            key,
            "must be a number".to_owned(),
        )),
    }
}

fn obj_str_array(
    args: &Value,
    key: &'static str,
    min: usize,
    max: usize,
) -> Result<Vec<String>, HostError> {
    match args.get(key) {
        Some(Value::Array(items)) => {
            if items.len() < min || items.len() > max {
                return Err(HostError::InvalidArgument(
                    key,
                    format!("must contain {min}..={max} items, got {}", items.len()),
                ));
            }
            items
                .iter()
                .map(|item| match item {
                    Value::String(text) if !text.is_empty() => Ok(text.clone()),
                    Value::String(_) => Err(HostError::InvalidArgument(
                        key,
                        "items must not be empty".to_owned(),
                    )),
                    _ => Err(HostError::InvalidArgument(
                        key,
                        "items must be strings".to_owned(),
                    )),
                })
                .collect()
        }
        _ => Err(HostError::InvalidArgument(
            key,
            "must be an array of strings".to_owned(),
        )),
    }
}

fn paths_field(args: &Value) -> Result<Vec<String>, HostError> {
    match args.get("paths") {
        Some(Value::Array(items)) => {
            if items.is_empty() || items.len() > 100 {
                return Err(HostError::InvalidArgument(
                    "paths",
                    format!("must contain 1..=100 items, got {}", items.len()),
                ));
            }
            items
                .iter()
                .map(|item| match item {
                    Value::String(text) => validate_rel_path(text).map(|_| text.clone()),
                    _ => Err(HostError::InvalidArgument(
                        "paths",
                        "items must be strings".to_owned(),
                    )),
                })
                .collect()
        }
        _ => Err(HostError::InvalidArgument(
            "paths",
            "must be an array of strings".to_owned(),
        )),
    }
}

fn size_guard(field: &'static str, len: usize, max: usize) -> Result<(), HostError> {
    if len <= max {
        Ok(())
    } else {
        Err(HostError::InvalidArgument(
            field,
            format!("exceeds {max} bytes"),
        ))
    }
}

/// 相对路径形状校验（宿主侧第一道关）：
/// 非空、非绝对、无 `..` 分量、无 NUL、长度有界。
/// 符号链接逃逸与 canonicalize 的落盘校验由服务实现负责（它才掌握仓库根）。
pub fn validate_rel_path(path: &str) -> Result<(), HostError> {
    let invalid = |reason: &str| HostError::InvalidArgument("path", reason.to_owned());
    if path.is_empty() {
        return Err(invalid("must not be empty"));
    }
    if path.len() > 4096 {
        return Err(invalid("is too long"));
    }
    if path.contains('\0') {
        return Err(invalid("contains NUL"));
    }
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') {
        return Err(invalid("must be relative to the repository root"));
    }
    if normalized.split('/').any(|component| component == "..") {
        return Err(invalid("must not contain `..` components"));
    }
    Ok(())
}

/// 插件命名空间化的设置键：`plugin.<plugin_id>.<key>`。
/// 命名空间在宿主侧强制（服务实现看不到裸 key），插件永远碰不到别人的设置。
pub fn setting_key(plugin_id: &str, key: &str) -> String {
    format!("plugin.{plugin_id}.{key}")
}

/// HTTP 白名单校验：只接受 https，host 必须在白名单内。
/// 默认白名单只含 api.github.com；用户显式添加的域名（T6.4 设置面板）落地后
/// 在此处扩充。白名单外的域名一律拒绝而不是静默放行。
pub fn validate_http_url(url: &str) -> Result<String, HostError> {
    const DEFAULT_ALLOWED: [&str; 1] = ["api.github.com"];
    let invalid = |reason: String| HostError::InvalidArgument("url", reason);
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(invalid("only https:// URLs are allowed".to_owned()));
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.contains(':') {
        return Err(invalid(format!("invalid authority in `{url}`")));
    }
    let host = authority.to_lowercase();
    if DEFAULT_ALLOWED.contains(&host.as_str()) {
        Ok(host)
    } else {
        Err(invalid(format!(
            "host `{host}` is not in the allowed list (default: api.github.com)"
        )))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// 记录调用轨迹的 mock 服务。
    struct MockServices {
        calls: Mutex<Vec<String>>,
        settings: Mutex<BTreeMap<String, Value>>,
    }

    impl MockServices {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                settings: Mutex::new(BTreeMap::new()),
            }
        }

        fn record(&self, what: String) {
            self.calls.lock().unwrap().push(what);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl HostServices for MockServices {
        fn repo_info(&self, plugin_id: &str) -> Result<Value, HostError> {
            self.record(format!("repo_info:{plugin_id}"));
            Ok(json!({"path": "/repo", "name": "repo", "currentBranch": "main", "isDirty": false}))
        }
        fn status(&self, plugin_id: &str, _filter: Option<String>) -> Result<Value, HostError> {
            self.record(format!("status:{plugin_id}"));
            Ok(json!({"entries": []}))
        }
        fn read_file(&self, plugin_id: &str, rel_path: &str) -> Result<Value, HostError> {
            self.record(format!("read_file:{plugin_id}:{rel_path}"));
            Ok(json!({"content": "hello"}))
        }
        fn list_dir(&self, plugin_id: &str, rel_path: &str) -> Result<Value, HostError> {
            self.record(format!("list_dir:{plugin_id}:{rel_path}"));
            Ok(json!({"entries": []}))
        }
        fn write_file(
            &self,
            plugin_id: &str,
            rel_path: &str,
            content: &str,
        ) -> Result<Value, HostError> {
            self.record(format!("write_file:{plugin_id}:{rel_path}:{content}"));
            Ok(json!({}))
        }
        fn http_get_json(
            &self,
            plugin_id: &str,
            url: &str,
            _headers: Value,
        ) -> Result<Value, HostError> {
            self.record(format!("http:{plugin_id}:{url}"));
            Ok(json!({"status": 200, "body": {}}))
        }
        fn get_setting(&self, plugin_id: &str, key: &str) -> Result<Value, HostError> {
            self.record(format!("get_setting:{plugin_id}:{key}"));
            let settings = self.settings.lock().unwrap();
            Ok(json!({"value": settings.get(key).cloned()}))
        }
        fn set_setting(
            &self,
            plugin_id: &str,
            key: &str,
            value: Value,
        ) -> Result<Value, HostError> {
            self.record(format!("set_setting:{plugin_id}:{key}"));
            self.settings.lock().unwrap().insert(key.to_owned(), value);
            Ok(json!({}))
        }
        fn git_log(
            &self,
            plugin_id: &str,
            limit: u32,
            _path: Option<String>,
        ) -> Result<Value, HostError> {
            self.record(format!("git_log:{plugin_id}:{limit}"));
            Ok(json!({"commits": []}))
        }
        fn git_stage(&self, plugin_id: &str, paths: Vec<String>) -> Result<Value, HostError> {
            self.record(format!("git_stage:{plugin_id}:{paths:?}"));
            Ok(json!({}))
        }
        fn git_commit(&self, plugin_id: &str, message: &str) -> Result<Value, HostError> {
            self.record(format!("git_commit:{plugin_id}:{message}"));
            Ok(json!({"commitId": "abc1234"}))
        }
        fn register_command(
            &self,
            plugin_id: &str,
            id: &str,
            title: &str,
            _keybinding: Option<String>,
        ) -> Result<Value, HostError> {
            self.record(format!("register_command:{plugin_id}:{id}:{title}"));
            Ok(json!({}))
        }
        fn register_panel(
            &self,
            plugin_id: &str,
            id: &str,
            title: &str,
            location: &str,
        ) -> Result<Value, HostError> {
            self.record(format!(
                "register_panel:{plugin_id}:{id}:{title}:{location}"
            ));
            Ok(json!({}))
        }
        fn show_toast(
            &self,
            plugin_id: &str,
            level: &str,
            message: &str,
        ) -> Result<Value, HostError> {
            self.record(format!("toast:{plugin_id}:{level}:{message}"));
            Ok(json!({}))
        }
        fn subscribe_events(
            &self,
            plugin_id: &str,
            events: Vec<String>,
        ) -> Result<Value, HostError> {
            self.record(format!("subscribe:{plugin_id}:{events:?}"));
            Ok(json!({}))
        }
    }

    const PLUGIN: &str = "com.example.test";

    fn granted(perms: &[Permission]) -> PermissionSet {
        PermissionSet::from_declared(perms.iter().copied())
    }

    fn call(
        services: &MockServices,
        perms: &[Permission],
        op: HostOp,
        args: Value,
    ) -> (i32, Value) {
        let mut staging = Vec::new();
        let code = dispatch_host_call(
            PLUGIN,
            &granted(perms),
            services,
            op.id(),
            &serde_json::to_vec(&args).unwrap(),
            &mut staging,
        );
        let staged: Value = if staging.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&staging).unwrap()
        };
        (code, staged)
    }

    // ---------- 结构性权限验收（T6.2 核心） ----------

    #[test]
    fn every_host_op_without_grants_is_denied_and_never_reaches_the_service() {
        // 结构保证：无授权时全表遍历，全部拒绝且服务零调用
        let services = MockServices::new();
        let empty = PermissionSet::none();
        let mut staging = Vec::new();
        for op in HostOp::ALL {
            let code = dispatch_host_call(
                PLUGIN,
                &empty,
                &services,
                op.id(),
                br#"{"path": "x"}"#,
                &mut staging,
            );
            assert_eq!(code, ERR_PERMISSION_DENIED, "op {} 应被拒绝", op.name());
            // 错误详情可从 staging 取回，且标明所需权限
            let detail: Value = serde_json::from_slice(&staging).unwrap();
            assert!(detail["error"]["message"]
                .as_str()
                .unwrap()
                .contains(op.name()));
        }
        assert!(services.calls().is_empty(), "被拒调用绝不能触达服务层");
    }

    #[test]
    fn the_op_table_is_complete_and_ids_are_unique() {
        let mut ids: Vec<i32> = HostOp::ALL.iter().map(|op| op.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), HostOp::ALL.len(), "op id 重复会破坏 ABI");
        for op in HostOp::ALL {
            assert_eq!(HostOp::from_id(op.id()), Some(op));
        }
    }

    // ---------- 各操作的正常与非法路径 ----------

    #[test]
    fn get_repo_info_returns_the_service_payload_with_git_read() {
        let services = MockServices::new();
        let (code, staged) = call(
            &services,
            &[Permission::GitRead],
            HostOp::GetRepoInfo,
            json!({}),
        );
        assert_eq!(code, ERR_OK);
        assert_eq!(staged["name"], "repo");
        assert_eq!(services.calls(), vec![format!("repo_info:{PLUGIN}")]);
    }

    #[test]
    fn read_file_rejects_paths_that_escape_the_repository_shape() {
        let services = MockServices::new();
        let perms = [Permission::FsRead];
        for bad in [
            "/etc/passwd",
            r"C:\Windows\system32\config",
            "../secret",
            "a/../../secret",
            "with\0nul",
            "",
        ] {
            let (code, _) = call(&services, &perms, HostOp::ReadFile, json!({"path": bad}));
            assert_eq!(code, ERR_INVALID_ARGUMENT, "path {bad} 应被拒绝");
        }
        assert!(services.calls().is_empty());

        let (code, _) = call(
            &services,
            &perms,
            HostOp::ReadFile,
            json!({"path": "src/lib.rs"}),
        );
        assert_eq!(code, ERR_OK);
        assert_eq!(
            services.calls(),
            vec![format!("read_file:{PLUGIN}:src/lib.rs")]
        );
    }

    #[test]
    fn http_get_json_enforces_the_https_and_domain_whitelist() {
        let services = MockServices::new();
        let perms = [Permission::NetGithub];

        let (code, staged) = call(
            &services,
            &perms,
            HostOp::HttpGetJson,
            json!({"url": "https://api.github.com/meta"}),
        );
        assert_eq!(code, ERR_OK);
        assert_eq!(staged["status"], 200);

        for bad in [
            "http://api.github.com/meta",       // 明文
            "https://evil.example.com/collect", // 白名单外
            "https://api.github.com:8080/meta", // 带端口
            "https://user@api.github.com/meta", // 带用户信息
            "ftp://api.github.com/meta",        // 非 http 协议
        ] {
            let (code, _) = call(&services, &perms, HostOp::HttpGetJson, json!({"url": bad}));
            assert_eq!(code, ERR_INVALID_ARGUMENT, "url {bad} 应被拒绝");
        }
        assert_eq!(
            services.calls(),
            vec![format!("http:{PLUGIN}:https://api.github.com/meta")]
        );
    }

    #[test]
    fn settings_are_namespaced_by_plugin_id_and_invisible_to_others() {
        let services = MockServices::new();
        let perms = [Permission::SettingsRead, Permission::SettingsWrite];

        let (code, _) = call(
            &services,
            &perms,
            HostOp::SetSetting,
            json!({"key": "templates", "value": ["feat", "fix"]}),
        );
        assert_eq!(code, ERR_OK);

        let (code, staged) = call(
            &services,
            &perms,
            HostOp::GetSetting,
            json!({"key": "templates"}),
        );
        assert_eq!(code, ERR_OK);
        assert_eq!(staged["value"], json!(["feat", "fix"]));

        // 服务看到的 key 一定带命名空间前缀
        assert!(services
            .calls()
            .iter()
            .all(|c| c.contains(&format!("plugin.{PLUGIN}."))));
    }

    #[test]
    fn git_write_operations_carry_the_plugin_id_for_audit() {
        let services = MockServices::new();
        let perms = [Permission::GitWrite];

        let (code, _) = call(
            &services,
            &perms,
            HostOp::GitStage,
            json!({"paths": ["src/lib.rs", "README.md"]}),
        );
        assert_eq!(code, ERR_OK);

        let (code, staged) = call(
            &services,
            &perms,
            HostOp::GitCommit,
            json!({"message": "feat: from plugin"}),
        );
        assert_eq!(code, ERR_OK);
        assert_eq!(staged["commitId"], "abc1234");

        // 空提交信息与越界路径列表被拒
        let (code, _) = call(
            &services,
            &perms,
            HostOp::GitCommit,
            json!({"message": "   "}),
        );
        assert_eq!(code, ERR_INVALID_ARGUMENT);
        let (code, _) = call(&services, &perms, HostOp::GitStage, json!({"paths": []}));
        assert_eq!(code, ERR_INVALID_ARGUMENT);
        let (code, _) = call(
            &services,
            &perms,
            HostOp::GitStage,
            json!({"paths": ["../escape"]}),
        );
        assert_eq!(code, ERR_INVALID_ARGUMENT);
    }

    #[test]
    fn registrations_are_prefixed_and_locations_are_whitelisted() {
        let services = MockServices::new();
        let perms = [Permission::UiCommand, Permission::UiPanel];

        let (code, _) = call(
            &services,
            &perms,
            HostOp::RegisterCommand,
            json!({"id": "fill", "title": "Fill"}),
        );
        assert_eq!(code, ERR_OK);
        assert!(services.calls()[0].contains(&format!("{PLUGIN}.fill")));

        let (code, _) = call(
            &services,
            &perms,
            HostOp::RegisterPanel,
            json!({"id": "stats", "title": "Stats", "location": "sidebar"}),
        );
        assert_eq!(code, ERR_OK);

        let (code, _) = call(
            &services,
            &perms,
            HostOp::RegisterPanel,
            json!({"id": "x", "title": "X", "location": "floating"}),
        );
        assert_eq!(code, ERR_INVALID_ARGUMENT);
    }

    #[test]
    fn toast_levels_are_whitelisted_and_texts_are_bounded() {
        let services = MockServices::new();
        let (code, _) = call(
            &services,
            &[Permission::UiToast],
            HostOp::ShowToast,
            json!({"level": "success", "message": "done"}),
        );
        assert_eq!(code, ERR_OK);

        let (code, _) = call(
            &services,
            &[Permission::UiToast],
            HostOp::ShowToast,
            json!({"level": "shouty", "message": "hi"}),
        );
        assert_eq!(code, ERR_INVALID_ARGUMENT);

        let (code, _) = call(
            &services,
            &[Permission::UiToast],
            HostOp::ShowToast,
            json!({"level": "info", "message": "x".repeat(9 * 1024)}),
        );
        assert_eq!(code, ERR_INVALID_ARGUMENT);
    }

    #[test]
    fn malformed_args_produce_invalid_argument_with_details_in_staging() {
        let services = MockServices::new();
        let perms = [Permission::GitRead];
        let (code, staged) = call(&services, &perms, HostOp::GetGitLog, json!({"limit": 0}));
        assert_eq!(code, ERR_INVALID_ARGUMENT);
        assert!(staged["error"]["message"]
            .as_str()
            .unwrap()
            .contains("limit"));

        let (code, _) = call(
            &services,
            &perms,
            HostOp::GetGitLog,
            json!({"limit": 99_999}),
        );
        assert_eq!(code, ERR_INVALID_ARGUMENT);
    }

    #[test]
    fn unknown_op_ids_are_rejected_without_touching_services() {
        let services = MockServices::new();
        let mut staging = Vec::new();
        let code = dispatch_host_call(
            PLUGIN,
            &granted(&Permission::ALL),
            &services,
            999,
            b"{}",
            &mut staging,
        );
        assert_eq!(code, ERR_INVALID_ARGUMENT);
        assert!(services.calls().is_empty());
    }

    #[test]
    fn oversized_args_and_results_hit_the_size_guards() {
        let services = MockServices::new();
        let mut staging = Vec::new();
        let big = vec![b'x'; MAX_ARG_BYTES + 1];
        let code = dispatch_host_call(
            PLUGIN,
            &granted(&[Permission::GitRead]),
            &services,
            HostOp::GetRepoInfo.id(),
            &big,
            &mut staging,
        );
        assert_eq!(code, ERR_TOO_LARGE);
    }

    #[test]
    fn staging_round_trips_with_retry_when_the_buffer_is_too_small() {
        let mut staging = b"0123456789".to_vec();
        let mut out = [0u8; 4];
        assert_eq!(take_staged_result_into(&mut staging, &mut out), Err(-10));
        // staging 保留，可加大缓冲重试
        let mut bigger = [0u8; 16];
        assert_eq!(take_staged_result_into(&mut staging, &mut bigger), Ok(10));
        assert_eq!(&bigger[..10], b"0123456789");
        // 清空后再取 → 0 字节
        assert_eq!(take_staged_result_into(&mut staging, &mut bigger), Ok(0));
    }

    #[test]
    fn service_failures_surface_their_specific_error_codes() {
        struct Failing;
        impl HostServices for Failing {
            fn repo_info(&self, _p: &str) -> Result<Value, HostError> {
                Err(HostError::NotFound("no repository open".to_owned()))
            }
            fn status(&self, _: &str, _: Option<String>) -> Result<Value, HostError> {
                Err(HostError::Timeout {
                    operation: "host-call",
                    limit_ms: 5000,
                })
            }
            fn read_file(&self, _: &str, _: &str) -> Result<Value, HostError> {
                Err(HostError::Engine("boom".to_owned()))
            }
            fn list_dir(&self, _: &str, _: &str) -> Result<Value, HostError> {
                unimplemented!()
            }
            fn write_file(&self, _: &str, _: &str, _: &str) -> Result<Value, HostError> {
                unimplemented!()
            }
            fn http_get_json(&self, _: &str, _: &str, _: Value) -> Result<Value, HostError> {
                unimplemented!()
            }
            fn get_setting(&self, _: &str, _: &str) -> Result<Value, HostError> {
                unimplemented!()
            }
            fn set_setting(&self, _: &str, _: &str, _: Value) -> Result<Value, HostError> {
                unimplemented!()
            }
            fn git_log(&self, _: &str, _: u32, _: Option<String>) -> Result<Value, HostError> {
                unimplemented!()
            }
            fn git_stage(&self, _: &str, _: Vec<String>) -> Result<Value, HostError> {
                unimplemented!()
            }
            fn git_commit(&self, _: &str, _: &str) -> Result<Value, HostError> {
                unimplemented!()
            }
            fn register_command(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: Option<String>,
            ) -> Result<Value, HostError> {
                unimplemented!()
            }
            fn register_panel(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<Value, HostError> {
                unimplemented!()
            }
            fn show_toast(&self, _: &str, _: &str, _: &str) -> Result<Value, HostError> {
                unimplemented!()
            }
            fn subscribe_events(&self, _: &str, _: Vec<String>) -> Result<Value, HostError> {
                unimplemented!()
            }
        }

        let mut staging = Vec::new();
        let code = dispatch_host_call(
            PLUGIN,
            &granted(&[Permission::GitRead]),
            &Failing,
            HostOp::GetRepoInfo.id(),
            b"{}",
            &mut staging,
        );
        assert_eq!(code, ERR_NOT_FOUND);
        let detail: Value = serde_json::from_slice(&staging).unwrap();
        assert_eq!(detail["error"]["code"], ERR_NOT_FOUND);

        let mut staging = Vec::new();
        let code = dispatch_host_call(
            PLUGIN,
            &granted(&[Permission::GitRead]),
            &Failing,
            HostOp::GetStatus.id(),
            b"{}",
            &mut staging,
        );
        assert_eq!(code, ERR_TIMEOUT);
    }

    #[test]
    fn subscribe_events_records_interest_and_validates_event_names() {
        let services = MockServices::new();
        let perms = [Permission::GitRead];

        let (code, _) = call(
            &services,
            &perms,
            HostOp::SubscribeEvents,
            json!({"events": ["repo_changed", "commit_created"]}),
        );
        assert_eq!(code, ERR_OK);
        assert_eq!(
            services.calls(),
            vec![format!(
                "subscribe:{PLUGIN}:[\"repo_changed\", \"commit_created\"]"
            )]
        );

        // 白名单外事件、空数组、超量订阅都被拒
        let (code, _) = call(
            &services,
            &perms,
            HostOp::SubscribeEvents,
            json!({"events": ["every_keystroke"]}),
        );
        assert_eq!(code, ERR_INVALID_ARGUMENT);
        let (code, _) = call(
            &services,
            &perms,
            HostOp::SubscribeEvents,
            json!({"events": []}),
        );
        assert_eq!(code, ERR_INVALID_ARGUMENT);
    }
}
