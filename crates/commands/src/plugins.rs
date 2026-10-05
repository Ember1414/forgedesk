//! 插件命令族与宿主服务的组合根实现（T6.4）。
//!
//! # 两个职责，一个模块
//!
//! 1. [`AppHostServices`]：[`HostServices`] 的**真实实现**——把插件的宿主调用
//!    落到本应用的 git 引擎 / 设置存储 / HTTP 底座上。它必须作为 `Arc` 在
//!    `src-tauri` 构建期注入 [`WasmiEngine`]（引擎在插件执行线程里回调它），
//!    因此持有的一切都是 `Arc` 克隆，没有对 `AppState` 的借用。
//! 2. 插件管理的 IPC 命令族（`plugin_*`）：列表 / 安装 / 启用禁用 / 授权 /
//!    撤权 / 卸载 / 重载 / 日志 / 面板渲染 / 命令执行。
//!
//! # "当前仓库"的解析
//!
//! 插件的仓库范围宿主调用（get_repo_info / get_status / read_file…）没有
//! 仓库参数——它们作用于**用户当前打开的仓库**（前端聚焦的那个）。后端的
//! 对应事实是 [`OpenRepoRegistry::last_opened`]（最近打开且仍打开）；
//! 没有任何仓库打开时返回结构化 `NOT_FOUND`，插件按可预期结果处理。
//!
//! # 未接线的操作
//!
//! `git_commit` 需要走快照 + 提交计划 + 审计的两阶段提交链路（T1.7 的
//! prepare/execute 管线），本期返回结构化错误而不是绕过安全网（红线 R7）。
//! 接入随提交钩子扩展点一起完成。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use forgedesk_domain::{AppError, AppResult};
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_plugin_host::engine_wasmi::WasmiEngine;
use forgedesk_plugin_host::host::{self, HostServices, SharedServices};
use forgedesk_plugin_host::manager::{
    InstallReport, ManagedState, PersistedEntry, PluginManager, PluginSummary, RegistryStore,
};
use forgedesk_plugin_host::permission::Permission;
use forgedesk_plugin_host::runtime::HostError;
use forgedesk_provider::{ApiRequest, GitHubHttp, HttpConfig};
use forgedesk_services::history::{HistoryQuery, HistoryService};
use forgedesk_services::repository::OpenRepoRegistry;
use forgedesk_services::workspace::WorkspaceService;
use forgedesk_storage::{Database, RepositoryStore, Scope, SettingsRepository};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::Emitter;

/// 事件：插件请求 toast（前端转为应用内通知）。
pub const EVENT_PLUGIN_TOAST: &str = "plugin-toast";
/// 事件：插件的命令/面板注册表发生变化（前端刷新命令面板与面板挂载）。
pub const EVENT_PLUGIN_REGISTRATIONS: &str = "plugin-registrations-changed";

/// 已注册贡献点（命令面板 / 面板挂载消费）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationDto {
    /// 注册它的插件 id。
    pub plugin_id: String,
    /// `command` 或 `panel`。
    pub kind: String,
    /// 全名（`<plugin-id>.<id>`）。
    pub id: String,
    pub title: String,
    /// 仅面板有：sidebar / bottom / repo-tab。
    pub location: Option<String>,
}

/// 插件宿主服务的真实实现（组合根）。
///
/// 手动 Debug：AppState 派生 Debug，这里只暴露可诊断计数，不把注册内容
/// 或插件输出打进日志（红线 R8）。
impl std::fmt::Debug for AppHostServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppHostServices")
            .field("registrations", &self.registrations.lock().map(|r| r.len()))
            .field(
                "subscribed_plugins",
                &self.subscriptions.lock().map(|s| s.len()),
            )
            .finish()
    }
}

pub struct AppHostServices {
    database: Arc<Database>,
    engines: Arc<GitEngines>,
    open: Arc<OpenRepoRegistry>,
    app: tauri::AppHandle,
    /// GitHub API 客户端（限流与连接池与账号服务共享同一套配置形状；
    /// 代理从设置读取在 T6.8 落地后接入）。
    http: GitHubHttp,
    registrations: Mutex<Vec<RegistrationDto>>,
    /// 插件 id → 订阅的事件名。
    subscriptions: Mutex<BTreeMap<String, Vec<String>>>,
}

impl AppHostServices {
    /// 构建组合根服务。
    pub fn new(
        database: Arc<Database>,
        engines: Arc<GitEngines>,
        open: Arc<OpenRepoRegistry>,
        app: tauri::AppHandle,
    ) -> Result<Self, AppError> {
        let http = GitHubHttp::new(HttpConfig::default())?;
        Ok(Self {
            database,
            engines,
            open,
            app,
            http,
            registrations: Mutex::new(Vec::new()),
            subscriptions: Mutex::new(BTreeMap::new()),
        })
    }

    /// 当前打开仓库的 id；没有则返回结构化 NOT_FOUND。
    fn current_repo(&self) -> AppResult<i64> {
        self.open.last_opened().ok_or_else(|| {
            AppError::new(
                forgedesk_domain::ErrorCode::NotFound,
                "no repository is open",
            )
        })
    }

    /// 构造工作区用例服务（与 AppState::workspace_service 同一批依赖）。
    fn workspace(&self) -> WorkspaceService<'_> {
        WorkspaceService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            &self.open,
        )
    }

    /// 解析相对路径并做落盘包含校验（canonicalize 后必须在仓库根内）。
    fn contained_path(&self, rel: &str) -> AppResult<PathBuf> {
        let repo_id = self.current_repo()?;
        let workdir = self.workspace().resolve_workdir(repo_id)?;
        let target = workdir.join(rel);
        let canonical = target.canonicalize().map_err(|error| {
            AppError::new(
                forgedesk_domain::ErrorCode::NotFound,
                format!("path does not exist: {error}"),
            )
        })?;
        let workdir_canonical = workdir.canonicalize().map_err(|error| {
            AppError::new(
                forgedesk_domain::ErrorCode::Internal,
                format!("could not canonicalize the workdir: {error}"),
            )
        })?;
        if !canonical.starts_with(&workdir_canonical) {
            // 宿主形状校验挡住了 `..`，符号链接逃逸在这里兜底
            return Err(AppError::new(
                forgedesk_domain::ErrorCode::Validation,
                "path escapes the repository root via a symlink",
            ));
        }
        Ok(canonical)
    }

    /// 已注册的贡献点（命令面板 / 面板挂载查询）。
    pub fn registrations(&self) -> Vec<RegistrationDto> {
        self.registrations
            .lock()
            .expect("registry poisoned")
            .clone()
    }
}

impl HostServices for AppHostServices {
    fn repo_info(&self, _plugin_id: &str) -> Result<Value, HostError> {
        let to_host = |error: AppError| {
            if error.code == forgedesk_domain::ErrorCode::NotFound {
                HostError::NotFound(error.message)
            } else {
                HostError::Engine(error.message)
            }
        };
        let repo_id = self.current_repo().map_err(to_host)?;
        let service = self.workspace();
        let report = service.status(repo_id, false).map_err(to_host)?;
        let workdir = service.resolve_workdir(repo_id).map_err(to_host)?;
        let dirty = !report.entries.is_empty();
        Ok(json!({
            "path": workdir.display().to_string(),
            "name": workdir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            "currentBranch": report.branch.head,
            "isDirty": dirty,
        }))
    }

    fn status(&self, _plugin_id: &str, filter: Option<String>) -> Result<Value, HostError> {
        let to_host = |error: AppError| HostError::Engine(error.message);
        let repo_id = self.current_repo().map_err(to_host)?;
        let entries = self
            .workspace()
            .status(repo_id, false)
            .map_err(to_host)?
            .entries
            .into_iter()
            .filter(|change| {
                filter.as_ref().is_none_or(|needle| {
                    change.path.to_string_lossy().contains(needle.as_str())
                })
            })
            .map(|change| {
                json!({
                    "path": change.path.to_string_lossy(),
                    "status": format!("{}{}", change.index_status.as_char(), change.worktree_status.as_char()),
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({ "entries": entries }))
    }

    fn read_file(&self, _plugin_id: &str, rel_path: &str) -> Result<Value, HostError> {
        let to_host = |error: AppError| HostError::Engine(error.message);
        let canonical = self.contained_path(rel_path).map_err(to_host)?;
        let bytes = std::fs::read(&canonical)
            .map_err(|error| HostError::NotFound(format!("{}: {error}", canonical.display())))?;
        // 文本契约：二进制内容经 lossy 转换会被破坏，直接报错让插件改走窄接口
        let text = String::from_utf8(bytes).map_err(|_| {
            HostError::InvalidArgument("path", "file is not valid UTF-8".to_owned())
        })?;
        Ok(json!({ "content": text }))
    }

    fn list_dir(&self, _plugin_id: &str, rel_path: &str) -> Result<Value, HostError> {
        let to_host = |error: AppError| HostError::Engine(error.message);
        let canonical = self.contained_path(rel_path).map_err(to_host)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&canonical)
            .map_err(|error| HostError::Engine(format!("read_dir failed: {error}")))?
        {
            let entry =
                entry.map_err(|error| HostError::Engine(format!("readdir entry: {error}")))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                "dir"
            } else {
                "file"
            };
            entries.push(json!({ "name": name, "kind": kind }));
        }
        Ok(json!({ "entries": entries }))
    }

    fn write_file(
        &self,
        plugin_id: &str,
        rel_path: &str,
        content: &str,
    ) -> Result<Value, HostError> {
        let to_host = |error: AppError| HostError::Engine(error.message);
        let canonical = self.contained_path(rel_path).map_err(to_host)?;
        std::fs::write(&canonical, content)
            .map_err(|error| HostError::Engine(format!("write failed: {error}")))?;
        // 来源=插件 的痕迹先落 tracing（结构化审计随提交钩子扩展点接
        // SnapshotManager 一起完成）
        tracing::info!(target: "audit", plugin_id, path = rel_path, "plugin wrote a file");
        Ok(json!({}))
    }

    fn http_get_json(
        &self,
        _plugin_id: &str,
        url: &str,
        headers: Value,
    ) -> Result<Value, HostError> {
        let mut request = ApiRequest::get(url);
        if let Value::Object(map) = &headers {
            for (key, value) in map {
                if let Some(text) = value.as_str() {
                    request = request
                        .with_header(key, text)
                        .map_err(|error| HostError::Engine(error.message))?;
                }
            }
        }
        // 宿主函数运行在普通线程（非 tokio worker）：用一个专用小运行时
        // 驱动 async 客户端；调用频率低且被 5s 墙钟上限约束，成本可接受
        let (status, body): (u16, Value) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| HostError::Engine(format!("runtime: {error}")))?
            .block_on(async {
                let response = self
                    .http
                    .send(&request)
                    .await
                    .map_err(|error| HostError::Engine(error.message))?;
                let status = response.status().as_u16();
                let body: Value = response
                    .json()
                    .await
                    .map_err(|error| HostError::Engine(format!("body is not JSON: {error}")))?;
                Ok((status, body))
            })?;
        Ok(json!({ "status": status, "body": body }))
    }

    fn get_setting(&self, _plugin_id: &str, key: &str) -> Result<Value, HostError> {
        // 命名空间前缀由派发器生成，这里直接按完整 key 读
        let value = SettingsRepository::new(&self.database)
            .get(&Scope::Global, key)
            .map_err(|error| HostError::Engine(error.message))?;
        Ok(
            json!({ "value": value.and_then(|raw: String| serde_json::from_str::<Value>(&raw).ok()) }),
        )
    }

    fn set_setting(&self, _plugin_id: &str, key: &str, value: Value) -> Result<Value, HostError> {
        SettingsRepository::new(&self.database)
            .set(&Scope::Global, key, &value.to_string())
            .map_err(|error| HostError::Engine(error.message))?;
        Ok(json!({}))
    }

    fn git_log(
        &self,
        _plugin_id: &str,
        limit: u32,
        path: Option<String>,
    ) -> Result<Value, HostError> {
        let to_host = |error: AppError| HostError::Engine(error.message);
        let repo_id = self.current_repo().map_err(to_host)?;
        let query = HistoryQuery {
            paths: path
                .map(forgedesk_domain::git::RepoPath::from)
                .map(|p| vec![p])
                .unwrap_or_default(),
            page_size: limit.min(100) as usize,
            ..HistoryQuery::default()
        };
        let page = HistoryService::new(&self.engines, RepositoryStore::new(&self.database))
            .page(repo_id, &query)
            .map_err(to_host)?;
        let commits = page
            .commits
            .iter()
            .map(|commit| {
                json!({
                    "id": commit.oid,
                    "summary": commit.subject,
                    "author": commit.author.name,
                    "time": commit.author.time,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({ "commits": commits }))
    }

    fn git_stage(&self, _plugin_id: &str, paths: Vec<String>) -> Result<Value, HostError> {
        let to_host = |error: AppError| HostError::Engine(error.message);
        let repo_id = self.current_repo().map_err(to_host)?;
        let repo_paths: Vec<forgedesk_domain::git::RepoPath> = paths
            .into_iter()
            .map(forgedesk_domain::git::RepoPath::from)
            .collect();
        self.workspace()
            .stage(repo_id, &repo_paths)
            .map_err(to_host)?;
        Ok(json!({}))
    }

    fn git_commit(&self, _plugin_id: &str, _message: &str) -> Result<Value, HostError> {
        // 红线 R7：改仓库状态必须走快照 + 审计；提交的两阶段管线接入前
        // 明确拒绝而不是绕过安全网
        Err(HostError::Engine(
            "git_commit is not wired to the snapshot pipeline yet".to_owned(),
        ))
    }

    fn register_command(
        &self,
        plugin_id: &str,
        id: &str,
        title: &str,
        keybinding: Option<String>,
    ) -> Result<Value, HostError> {
        let mut registry = self.registrations.lock().expect("registry poisoned");
        registry.retain(|r| !(r.plugin_id == plugin_id && r.id == id));
        registry.push(RegistrationDto {
            plugin_id: plugin_id.to_owned(),
            kind: "command".to_owned(),
            id: id.to_owned(),
            title: title.to_owned(),
            // command 没有面板位置；keybinding 走设置页改绑，不进 location
            location: None,
        });
        drop(registry);
        let _ = self.app.emit(EVENT_PLUGIN_REGISTRATIONS, ());
        Ok(json!({}))
    }

    fn register_panel(
        &self,
        plugin_id: &str,
        id: &str,
        title: &str,
        location: &str,
    ) -> Result<Value, HostError> {
        let mut registry = self.registrations.lock().expect("registry poisoned");
        registry.retain(|r| !(r.plugin_id == plugin_id && r.id == id));
        registry.push(RegistrationDto {
            plugin_id: plugin_id.to_owned(),
            kind: "panel".to_owned(),
            id: id.to_owned(),
            title: title.to_owned(),
            location: Some(location.to_owned()),
        });
        drop(registry);
        let _ = self.app.emit(EVENT_PLUGIN_REGISTRATIONS, ());
        Ok(json!({}))
    }

    fn show_toast(&self, plugin_id: &str, level: &str, message: &str) -> Result<Value, HostError> {
        let _ = self.app.emit(
            EVENT_PLUGIN_TOAST,
            json!({ "pluginId": plugin_id, "level": level, "message": message }),
        );
        Ok(json!({}))
    }

    fn subscribe_events(&self, plugin_id: &str, events: Vec<String>) -> Result<Value, HostError> {
        self.subscriptions
            .lock()
            .expect("subscriptions poisoned")
            .insert(plugin_id.to_owned(), events);
        Ok(json!({}))
    }

    fn event_interest(&self, plugin_id: &str, event: &str) -> bool {
        self.subscriptions
            .lock()
            .expect("subscriptions poisoned")
            .get(plugin_id)
            .is_some_and(|events| events.iter().any(|e| e == event))
    }
}

// ---------- 注册表持久化（SQLite 设置 KV） ----------

/// 设置 KV 里保存注册表 JSON 的键。
pub const PLUGIN_REGISTRY_KEY: &str = "plugins.registry";

/// 组合根的注册表持久化：JSON 存全局设置 KV（T6.1 允许 settings 方案）。
pub struct SettingsRegistryStore {
    database: Arc<Database>,
}

impl SettingsRegistryStore {
    pub fn new(database: Arc<Database>) -> Self {
        Self { database }
    }
}

impl RegistryStore for SettingsRegistryStore {
    fn save(&self, entries: &[PersistedEntry]) -> Result<(), HostError> {
        let json = serde_json::to_string(entries)
            .map_err(|error| HostError::Engine(format!("serialize registry: {error}")))?;
        SettingsRepository::new(&self.database)
            .set(&Scope::Global, PLUGIN_REGISTRY_KEY, &json)
            .map_err(|error| HostError::Engine(error.message))
    }

    fn load(&self) -> Vec<PersistedEntry> {
        SettingsRepository::new(&self.database)
            .get(&Scope::Global, PLUGIN_REGISTRY_KEY)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }
}

// ---------- IPC 命令 ----------

fn to_app(error: HostError) -> AppError {
    let code = match &error {
        HostError::NotFound(_) => forgedesk_domain::ErrorCode::NotFound,
        HostError::InvalidArgument(_, _) => forgedesk_domain::ErrorCode::Validation,
        HostError::PermissionDenied { .. } => forgedesk_domain::ErrorCode::PermissionDenied,
        _ => forgedesk_domain::ErrorCode::Internal,
    };
    AppError::new(code, error.to_string())
}

fn parse_permission(raw: &str) -> Result<Permission, AppError> {
    Permission::try_from(raw)
        .map_err(|error| AppError::new(forgedesk_domain::ErrorCode::Validation, error.to_string()))
}

/// 列出已安装插件。
#[tauri::command]
pub fn plugin_list(state: tauri::State<'_, PluginManager>) -> AppResult<Vec<PluginSummary>> {
    Ok(state.list())
}

/// 开发者模式：从本地目录安装插件（需用户在 UI 确认警告）。
#[tauri::command]
pub fn plugin_install_from_dir(
    state: tauri::State<'_, PluginManager>,
    dir: String,
) -> AppResult<InstallReport> {
    state
        .install_from_dir(std::path::Path::new(&dir))
        .map_err(to_app)
}

/// 启用 / 禁用插件。
#[tauri::command]
pub fn plugin_set_enabled(
    state: tauri::State<'_, PluginManager>,
    id: String,
    enabled: bool,
) -> AppResult<()> {
    state.set_enabled(&id, enabled).map_err(to_app)
}

/// 授予权限（逐项；扩权在插件重启后生效——UI 提示）。
#[tauri::command]
pub fn plugin_grant(
    state: tauri::State<'_, PluginManager>,
    id: String,
    permissions: Vec<String>,
) -> AppResult<()> {
    let perms: Vec<Permission> = permissions
        .iter()
        .map(|raw| parse_permission(raw))
        .collect::<AppResult<_>>()?;
    state.grant(&id, &perms).map_err(to_app)
}

/// 撤销一项权限（立即生效，插件的下一次调用即失败）。
#[tauri::command]
pub fn plugin_revoke(
    state: tauri::State<'_, PluginManager>,
    id: String,
    permission: String,
) -> AppResult<()> {
    let permission = parse_permission(&permission)?;
    state.revoke(&id, permission).map_err(to_app)
}

/// 卸载插件；返回是否删除了其目录（root 外的开发者目录保留）。
#[tauri::command]
pub fn plugin_uninstall(state: tauri::State<'_, PluginManager>, id: String) -> AppResult<bool> {
    state.uninstall(&id).map_err(to_app)
}

/// 热重载（开发者模式）。
#[tauri::command]
pub fn plugin_reload(state: tauri::State<'_, PluginManager>, id: String) -> AppResult<()> {
    state.reload(&id).map_err(to_app)
}

/// 插件日志（运行中的实例）。
#[tauri::command]
pub fn plugin_logs(
    state: tauri::State<'_, PluginManager>,
    id: String,
    limit: u32,
) -> AppResult<Vec<forgedesk_plugin_host::engine_wasmi::PluginLogEntry>> {
    state.logs(&id, limit as usize).map_err(to_app)
}

/// 渲染插件面板（返回已校验的 DSL JSON）。
#[tauri::command]
pub fn plugin_render_panel(
    state: tauri::State<'_, PluginManager>,
    id: String,
    panel_id: String,
) -> AppResult<String> {
    state.render_panel(&id, &panel_id).map_err(to_app)
}

/// 执行插件命令（命令面板与面板按钮的执行入口）。
#[tauri::command]
pub fn plugin_invoke_command(
    state: tauri::State<'_, PluginManager>,
    id: String,
    command: String,
    arg_json: String,
) -> AppResult<String> {
    state
        .invoke_command(&id, &command, &arg_json)
        .map_err(to_app)
}

/// 已注册的贡献点（命令面板 / 面板挂载）。
#[tauri::command]
pub fn plugin_registrations(
    state: tauri::State<'_, AppHostServices>,
) -> AppResult<Vec<RegistrationDto>> {
    Ok(state.registrations())
}

/// 构建插件宿主（组合根的装配函数，`src-tauri` 启动时调用）。
///
/// 返回（管理器, 宿主服务）——两者都要进 `AppState`：管理器是命令的执行面，
/// 服务持有注册表（`plugin_registrations` 查询它）。
pub fn build_plugin_host(
    database: Arc<Database>,
    engines: Arc<GitEngines>,
    open: Arc<OpenRepoRegistry>,
    plugins_root: PathBuf,
    app: tauri::AppHandle,
) -> AppResult<(PluginManager, Arc<AppHostServices>)> {
    let services_impl = Arc::new(AppHostServices::new(
        Arc::clone(&database),
        Arc::clone(&engines),
        Arc::clone(&open),
        app,
    )?);
    let services: SharedServices = services_impl.clone();
    let engine = Arc::new(WasmiEngine::new(
        forgedesk_plugin_host::runtime::RuntimeLimits::default(),
        services,
    ));
    let store = SettingsRegistryStore::new(database);
    let manager = PluginManager::new(engine, plugins_root, Arc::new(store))
        .map_err(|error| AppError::new(forgedesk_domain::ErrorCode::Internal, error.to_string()))?;
    Ok((manager, services_impl))
}
