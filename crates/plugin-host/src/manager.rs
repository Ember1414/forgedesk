//! 插件生命周期管理（T6.4）：安装、授权、启用/禁用、卸载、热重载。
//!
//! # 职责与边界
//!
//! [`PluginManager`] 是 IPC 层看到的唯一插件入口：它持有 [`WasmiEngine`]，
//! 管理安装目录与注册表，把"用户授权了什么"翻译成引擎加载时的权限交集。
//! 它不做策略：哪些权限该授予、何时弹出授权对话框，是前端（T6.4 管理页）
//! 的事；这里只忠实记录与执行。
//!
//! # 持久化
//!
//! 注册表经 [`RegistryStore`] 抽象保存（组合根接 SQLite 的设置 KV），
//! 只存"目录、sha256、授权集、启用状态"这类小标量；manifest 与 wasm 每次
//! 启动时从安装目录重读——目录被用户手删的插件在启动时跳过并告警，
//! 而不是让整个注册表失效。
//!
//! # 授权语义
//!
//! - 安装后授权集为空（授权对话框在启用前弹出，逐项授予）；
//! - **撤销立即生效**（运行中实例的权限集同步收缩，下一次调用即失败）；
//! - **扩权需重启实例**（安全方向：扩权走完整授权对话框后重新启用）；
//! - 生效权限 = 清单声明 ∩ 用户授权，交集在加载时收敛，运行中只能收缩。
//!
//! # 卸载的目录语义（T6.4 验收二选一）
//!
//! 插件目录在 `plugins_root` 之内（正常安装）→ 卸载即删除目录；
//! 开发者模式从任意目录加载（目录在 root 之外）→ 只移除注册条目并**保留
//! 目录**（那是开发者自己的工作副本，删掉等于毁人工作区）。返回值明确
//! 告知调用方是哪种，UI 据此提示。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine_wasmi::WasmiEngine;
use crate::manifest::{PluginManifest, ValidatedManifest};
use crate::permission::Permission;
use crate::runtime::{HostError, PluginEngine, PluginHandle};

/// 已安装插件的管理态（启用/禁用是用户操作；崩溃由引擎报告）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ManagedState {
    /// 已启用（实例已加载并激活）。
    Enabled,
    /// 已禁用（未加载或已卸载）。
    Disabled,
    /// 启用失败或运行中崩溃（引擎隔离，详情见插件日志）。
    Crashed,
}

/// 注册表持久化条目（小标量；wasm 与 manifest 不入库，从目录重读）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedEntry {
    /// 插件 id（唯一键）。
    pub id: String,
    /// 安装目录（绝对路径）。
    pub dir: String,
    /// 安装时 wasm 的 SHA256（hex）。
    pub sha256: String,
    /// 用户已授予的权限。
    pub granted: Vec<Permission>,
    /// 上次会话结束时是否处于启用状态。
    pub enabled: bool,
}

/// 注册表持久化抽象（组合根接 SQLite 设置 KV；测试用内存实现）。
pub trait RegistryStore: Send + Sync {
    /// 全量保存注册表。
    fn save(&self, entries: &[PersistedEntry]) -> Result<(), HostError>;
    /// 读取注册表（无记录时返回空）。
    fn load(&self) -> Vec<PersistedEntry>;
}

/// 内存中的已安装插件。
struct InstalledPlugin {
    manifest: ValidatedManifest,
    wasm: Vec<u8>,
    dir: PathBuf,
    sha256: String,
    /// 用户授权集（生效权限 = 清单 ∩ 本集合，加载时收敛）。
    granted: BTreeSet<Permission>,
    state: ManagedState,
    /// 运行中实例的句柄；Disabled/Crashed 时为 None。
    runtime: Option<PluginHandle>,
}

/// 列表页的条目摘要（IPC 返回形状，不含 wasm 字节）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSummary {
    /// 插件 id。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// 版本。
    pub version: String,
    /// 作者。
    pub author: String,
    /// 许可证。
    pub license: String,
    /// 描述。
    pub description: String,
    /// 管理态。
    pub state: ManagedState,
    /// 清单声明的权限。
    pub declared_permissions: Vec<Permission>,
    /// 用户已授予的权限。
    pub granted_permissions: Vec<Permission>,
    /// 成功的宿主调用计数（按权限聚合；未运行时为空）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub permission_usage: Vec<(Permission, u64)>,
}

/// 安装结果摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    /// 插件 id。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// 版本。
    pub version: String,
    /// wasm 的 SHA256（hex，UI 展示供用户核对）。
    pub sha256: String,
    /// 插件在 `plugins_root` 内（卸载会删目录）还是外部（开发者模式，保留目录）。
    pub inside_root: bool,
    /// 清单声明的权限。
    pub declared_permissions: Vec<Permission>,
}

/// 插件生命周期管理器。
///
/// 手动实现 Debug：内部持有 wasm 引擎与全部实例状态（非 Debug），
/// 对外只暴露可诊断的字段——排查问题够用，不把实例内容打进日志（红线 R8）。
impl std::fmt::Debug for PluginManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginManager")
            .field("plugins_root", &self.plugins_root)
            .field("installed", &self.plugins.read().len())
            .finish()
    }
}

pub struct PluginManager {
    engine: Arc<WasmiEngine>,
    plugins_root: PathBuf,
    store: Arc<dyn RegistryStore>,
    plugins: parking_lot::RwLock<BTreeMap<String, InstalledPlugin>>,
}

impl PluginManager {
    /// 构建并从持久层恢复注册表（损坏/缺失的条目跳过并告警，不 panic）。
    pub fn new(
        engine: Arc<WasmiEngine>,
        plugins_root: PathBuf,
        store: Arc<dyn RegistryStore>,
    ) -> Result<Self, HostError> {
        std::fs::create_dir_all(&plugins_root).map_err(|error| {
            HostError::Engine(format!("could not create the plugins directory: {error}"))
        })?;
        let manager = Self {
            engine,
            plugins_root,
            store,
            plugins: parking_lot::RwLock::new(BTreeMap::new()),
        };
        manager.restore()?;
        Ok(manager)
    }

    /// 从持久层恢复：重读目录中的 manifest 与 wasm，校验 sha256。
    fn restore(&self) -> Result<(), HostError> {
        for entry in self.store.load() {
            let dir = PathBuf::from(&entry.dir);
            let (manifest, wasm) = match read_plugin_dir(&dir) {
                Ok(pair) => pair,
                Err(error) => {
                    tracing::warn!(plugin_id = entry.id, error = %error, "skipping plugin directory");
                    continue;
                }
            };
            if entry.sha256 != sha256_hex(&wasm) {
                tracing::warn!(
                    plugin_id = entry.id,
                    "plugin wasm changed on disk; skipping"
                );
                continue;
            }
            let state = if entry.enabled {
                match self.load_and_activate(
                    &manifest,
                    &wasm,
                    &entry.granted.iter().copied().collect(),
                ) {
                    Ok(_) => ManagedState::Enabled,
                    Err(error) => {
                        tracing::warn!(plugin_id = entry.id, error = %error, "plugin failed to restart");
                        ManagedState::Crashed
                    }
                }
            } else {
                ManagedState::Disabled
            };
            self.plugins.write().insert(
                entry.id,
                InstalledPlugin {
                    manifest,
                    wasm,
                    dir,
                    sha256: entry.sha256,
                    granted: entry.granted.into_iter().collect(),
                    state,
                    runtime: None,
                },
            );
        }
        Ok(())
    }

    /// 开发者模式：从任意本地目录安装（T6.4）。
    ///
    /// 安装后处于禁用状态——授权对话框（前端）在启用前逐项授予权限。
    pub fn install_from_dir(&self, dir: &Path) -> Result<InstallReport, HostError> {
        let (manifest, wasm) = read_plugin_dir(dir)?;
        let id = manifest.id.clone();
        {
            let plugins = self.plugins.read();
            if plugins.contains_key(&id) {
                return Err(HostError::InvalidArgument(
                    "id",
                    format!("plugin `{id}` is already installed"),
                ));
            }
        }
        let sha256 = sha256_hex(&wasm);
        let declared: Vec<Permission> = manifest.permissions.clone();
        let dir = dir.to_path_buf();
        let inside_root = dir.starts_with(&self.plugins_root);
        {
            let mut plugins = self.plugins.write();
            plugins.insert(
                id.clone(),
                InstalledPlugin {
                    manifest,
                    wasm,
                    dir: dir.clone(),
                    sha256: sha256.clone(),
                    granted: BTreeSet::new(),
                    state: ManagedState::Disabled,
                    runtime: None,
                },
            );
        }
        self.persist()?;
        Ok(InstallReport {
            id: id.clone(),
            name: self.summary_name(&id),
            version: self.summary_version(&id),
            sha256,
            inside_root,
            declared_permissions: declared,
        })
    }

    /// 授予权限（启用前的授权对话框逐项调用；扩权在下次启用时生效）。
    pub fn grant(&self, id: &str, permissions: &[Permission]) -> Result<(), HostError> {
        let mut plugins = self.plugins.write();
        let plugin = Self::lookup_mut(&mut plugins, id)?;
        if matches!(plugin.state, ManagedState::Enabled) {
            return Err(HostError::InvalidArgument(
                "id",
                "stop the plugin before expanding its grants".to_owned(),
            ));
        }
        plugin.granted.extend(permissions.iter().copied());
        drop(plugins);
        self.persist()
    }

    /// 撤销一项权限：立即生效（运行中实例同步收缩），失败不影响注册表。
    pub fn revoke(&self, id: &str, permission: Permission) -> Result<(), HostError> {
        let runtime = {
            let mut plugins = self.plugins.write();
            let plugin = Self::lookup_mut(&mut plugins, id)?;
            plugin.granted.remove(&permission);
            plugin.runtime
        };
        if let Some(handle) = runtime {
            self.engine.revoke_permission(handle, permission)?;
        }
        self.persist()
    }

    /// 启用：加载实例（生效权限 = 清单 ∩ 授权）并激活。
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), HostError> {
        if enabled {
            let (manifest, wasm, granted) = {
                let plugins = self.plugins.read();
                let plugin = Self::lookup(&plugins, id)?;
                if matches!(plugin.state, ManagedState::Enabled) {
                    return Ok(()); // 幂等
                }
                (
                    plugin.manifest.clone(),
                    plugin.wasm.clone(),
                    plugin.granted.clone(),
                )
            };
            let effective: BTreeSet<Permission> = manifest
                .permissions
                .iter()
                .copied()
                .filter(|p| granted.contains(p))
                .collect();
            let handle = self
                .load_and_activate(&manifest, &wasm, &effective)
                .inspect_err(|error| {
                    let mut plugins = self.plugins.write();
                    if let Some(plugin) = plugins.get_mut(id) {
                        plugin.state = ManagedState::Crashed;
                        let _ = error;
                    }
                })?;
            let mut plugins = self.plugins.write();
            let plugin = Self::lookup_mut(&mut plugins, id)?;
            plugin.state = ManagedState::Enabled;
            plugin.runtime = Some(handle);
            Ok(())
        } else {
            let runtime = {
                let mut plugins = self.plugins.write();
                let plugin = Self::lookup_mut(&mut plugins, id)?;
                if !matches!(plugin.state, ManagedState::Enabled) {
                    return Ok(()); // 幂等
                }
                plugin.state = ManagedState::Disabled;
                plugin.runtime.take()
            };
            if let Some(handle) = runtime {
                self.engine.unload(handle)?;
            }
            Ok(())
        }
    }

    /// 重新加载（开发者模式热重载）：重读目录，保持授权与启用状态。
    pub fn reload(&self, id: &str) -> Result<(), HostError> {
        let was_enabled = {
            let mut plugins = self.plugins.write();
            let plugin = Self::lookup_mut(&mut plugins, id)?;
            if !matches!(plugin.state, ManagedState::Enabled) {
                return Err(HostError::InvalidArgument(
                    "id",
                    "plugin is not running".to_owned(),
                ));
            }
            plugin.state = ManagedState::Disabled;
            plugin.runtime.take().is_some()
        };
        // 重读目录（开发者可能改了代码），sha256 刷新
        let (manifest, wasm, granted) = {
            let mut plugins = self.plugins.write();
            let plugin = Self::lookup_mut(&mut plugins, id)?;
            let (manifest, wasm) = read_plugin_dir(&plugin.dir)?;
            plugin.sha256 = sha256_hex(&wasm);
            plugin.manifest = manifest.clone();
            plugin.wasm = wasm.clone();
            (manifest, wasm, plugin.granted.clone())
        };
        if was_enabled {
            let effective: BTreeSet<Permission> = manifest
                .permissions
                .iter()
                .copied()
                .filter(|p| granted.contains(p))
                .collect();
            let handle = self
                .load_and_activate(&manifest, &wasm, &effective)
                .inspect_err(|error| {
                    let mut plugins = self.plugins.write();
                    if let Some(plugin) = plugins.get_mut(id) {
                        plugin.state = ManagedState::Crashed;
                        let _ = error;
                    }
                })?;
            let mut plugins = self.plugins.write();
            let plugin = Self::lookup_mut(&mut plugins, id)?;
            plugin.state = ManagedState::Enabled;
            plugin.runtime = Some(handle);
        }
        Ok(())
    }

    /// 卸载：移除注册条目；`plugins_root` 内的目录一并删除，外部目录保留。
    /// 返回是否删除了目录（UI 据此提示）。
    pub fn uninstall(&self, id: &str) -> Result<bool, HostError> {
        let (runtime, dir, inside_root) = {
            let mut plugins = self.plugins.write();
            let plugin = Self::lookup_mut(&mut plugins, id)?;
            let runtime = plugin.runtime.take();
            let dir = plugin.dir.clone();
            let inside_root = dir.starts_with(&self.plugins_root);
            plugins.remove(id);
            (runtime, dir, inside_root)
        };
        if let Some(handle) = runtime {
            self.engine.unload(handle)?;
        }
        let removed = if inside_root {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => true,
                // 目录已被手动删除也算卸载成功
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => {
                    return Err(HostError::Engine(format!(
                        "could not delete the plugin directory: {error}"
                    )))
                }
            }
        } else {
            false
        };
        self.persist()?;
        Ok(removed)
    }

    /// 列表页摘要（权限使用计数仅运行中的插件有值）。
    pub fn list(&self) -> Vec<PluginSummary> {
        let plugins = self.plugins.read();
        plugins
            .values()
            .map(|plugin| {
                let usage = match plugin.runtime {
                    Some(handle) => self.engine.permission_usage(handle).unwrap_or_default(),
                    None => Vec::new(),
                };
                PluginSummary {
                    id: plugin.manifest.id.clone(),
                    name: plugin.manifest.name.clone(),
                    version: plugin.manifest.version.clone(),
                    author: plugin.manifest.author.clone(),
                    license: plugin.manifest.license.clone(),
                    description: plugin.manifest.description.clone(),
                    state: plugin.state,
                    declared_permissions: plugin.manifest.permissions.clone(),
                    granted_permissions: plugin.granted.iter().copied().collect(),
                    permission_usage: usage,
                }
            })
            .collect()
    }

    /// 插件日志（运行中的实例；T6.4 日志页）。
    pub fn logs(
        &self,
        id: &str,
        limit: usize,
    ) -> Result<Vec<crate::engine_wasmi::PluginLogEntry>, HostError> {
        let runtime = {
            let plugins = self.plugins.read();
            Self::lookup(&plugins, id)?.runtime
        };
        let Some(handle) = runtime else {
            // 未运行的插件没有实例内日志（历史日志在 tracing 文件里）
            return Ok(Vec::new());
        };
        self.engine.plugin_logs(handle, limit)
    }

    /// 渲染面板（透传引擎）。
    pub fn render_panel(&self, id: &str, panel_id: &str) -> Result<String, HostError> {
        let runtime = {
            let plugins = self.plugins.read();
            Self::lookup(&plugins, id)?.runtime
        };
        let Some(handle) = runtime else {
            return Err(HostError::InvalidArgument(
                "id",
                "plugin is not running".to_owned(),
            ));
        };
        self.engine.render_panel(handle, panel_id)
    }

    /// 执行插件命令（透传引擎；命令面板与面板按钮的执行入口）。
    pub fn invoke_command(
        &self,
        id: &str,
        command: &str,
        arg_json: &str,
    ) -> Result<String, HostError> {
        let runtime = {
            let plugins = self.plugins.read();
            Self::lookup(&plugins, id)?.runtime
        };
        let Some(handle) = runtime else {
            return Err(HostError::InvalidArgument(
                "id",
                "plugin is not running".to_owned(),
            ));
        };
        self.engine.invoke(handle, command, arg_json)
    }

    // ---------- 内部 ----------

    fn load_and_activate(
        &self,
        manifest: &ValidatedManifest,
        wasm: &[u8],
        effective: &BTreeSet<Permission>,
    ) -> Result<PluginHandle, HostError> {
        let handle = self.engine.load(manifest, wasm)?;
        // 生效权限在加载时收敛，运行中只能收缩（revoke）
        self.engine
            .set_effective_permissions(handle, effective.clone())?;
        self.engine.activate(handle).inspect_err(|_| {
            // 激活失败的实例已经泄漏：立刻回收
            let _ = self.engine.unload(handle);
        })?;
        Ok(handle)
    }

    fn persist(&self) -> Result<(), HostError> {
        let entries: Vec<PersistedEntry> = {
            let plugins = self.plugins.read();
            plugins
                .values()
                .map(|plugin| PersistedEntry {
                    id: plugin.manifest.id.clone(),
                    dir: plugin.dir.display().to_string(),
                    sha256: plugin.sha256.clone(),
                    granted: plugin.granted.iter().copied().collect(),
                    enabled: matches!(plugin.state, ManagedState::Enabled),
                })
                .collect()
        };
        self.store.save(&entries)
    }

    fn summary_name(&self, id: &str) -> String {
        self.plugins
            .read()
            .get(id)
            .map(|p| p.manifest.name.clone())
            .unwrap_or_default()
    }

    fn summary_version(&self, id: &str) -> String {
        self.plugins
            .read()
            .get(id)
            .map(|p| p.manifest.version.clone())
            .unwrap_or_default()
    }

    fn lookup<'a>(
        plugins: &'a BTreeMap<String, InstalledPlugin>,
        id: &str,
    ) -> Result<&'a InstalledPlugin, HostError> {
        plugins
            .get(id)
            .ok_or_else(|| HostError::NotFound(format!("plugin `{id}` is not installed")))
    }

    fn lookup_mut<'a>(
        plugins: &'a mut BTreeMap<String, InstalledPlugin>,
        id: &str,
    ) -> Result<&'a mut InstalledPlugin, HostError> {
        plugins
            .get_mut(id)
            .ok_or_else(|| HostError::NotFound(format!("plugin `{id}` is not installed")))
    }
}

/// 读取插件目录（manifest.json + wasm 入口）。
fn read_plugin_dir(dir: &Path) -> Result<(ValidatedManifest, Vec<u8>), HostError> {
    let manifest_path = dir.join("plugin.json");
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .map_err(|error| HostError::NotFound(format!("{}: {error}", manifest_path.display())))?;
    let manifest = PluginManifest::parse(&manifest_text)
        .map_err(|error| HostError::InvalidArgument("plugin.json", error.to_string()))?;
    let wasm = std::fs::read(dir.join(&manifest.main))
        .map_err(|error| HostError::NotFound(format!("plugin entry {}: {error}", manifest.main)))?;
    Ok((manifest, wasm))
}

/// wasm 字节的 SHA256（hex 小写）。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::engine_wasmi::fixture;
    use serde_json::json;
    use std::sync::Mutex;

    /// 内存注册表（记录保存次数供断言）。
    #[derive(Default)]
    struct MemStore {
        entries: Mutex<Vec<PersistedEntry>>,
        saves: std::sync::atomic::AtomicUsize,
    }

    impl RegistryStore for MemStore {
        fn save(&self, entries: &[PersistedEntry]) -> Result<(), HostError> {
            *self.entries.lock().unwrap() = entries.to_vec();
            self.saves.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn load(&self) -> Vec<PersistedEntry> {
            self.entries.lock().unwrap().clone()
        }
    }

    /// 最小 HostServices：管理器测试不触发宿主调用，只有 engine 构建需要它。
    #[derive(Default)]
    struct NoopServices;

    impl crate::host::HostServices for NoopServices {
        fn repo_info(&self, _: &str) -> Result<serde_json::Value, HostError> {
            Err(HostError::NotFound("no repository".to_owned()))
        }
        fn status(&self, _: &str, _: Option<String>) -> Result<serde_json::Value, HostError> {
            Err(HostError::NotFound("no repository".to_owned()))
        }
        fn read_file(&self, _: &str, _: &str) -> Result<serde_json::Value, HostError> {
            Err(HostError::NotFound("no repository".to_owned()))
        }
        fn list_dir(&self, _: &str, _: &str) -> Result<serde_json::Value, HostError> {
            Err(HostError::NotFound("no repository".to_owned()))
        }
        fn write_file(&self, _: &str, _: &str, _: &str) -> Result<serde_json::Value, HostError> {
            Err(HostError::NotFound("no repository".to_owned()))
        }
        fn http_get_json(
            &self,
            _: &str,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, HostError> {
            Err(HostError::NotFound("no network".to_owned()))
        }
        fn get_setting(&self, _: &str, _: &str) -> Result<serde_json::Value, HostError> {
            Ok(json!({"value": null}))
        }
        fn set_setting(
            &self,
            _: &str,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, HostError> {
            Ok(json!({}))
        }
        fn git_log(
            &self,
            _: &str,
            _: u32,
            _: Option<String>,
        ) -> Result<serde_json::Value, HostError> {
            Err(HostError::NotFound("no repository".to_owned()))
        }
        fn git_stage(&self, _: &str, _: Vec<String>) -> Result<serde_json::Value, HostError> {
            Err(HostError::NotFound("no repository".to_owned()))
        }
        fn git_commit(&self, _: &str, _: &str) -> Result<serde_json::Value, HostError> {
            Err(HostError::NotFound("no repository".to_owned()))
        }
        fn register_command(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: Option<String>,
        ) -> Result<serde_json::Value, HostError> {
            Ok(json!({}))
        }
        fn register_panel(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<serde_json::Value, HostError> {
            Ok(json!({}))
        }
        fn show_toast(&self, _: &str, _: &str, _: &str) -> Result<serde_json::Value, HostError> {
            Ok(json!({}))
        }
        fn subscribe_events(
            &self,
            _: &str,
            _: Vec<String>,
        ) -> Result<serde_json::Value, HostError> {
            Ok(json!({}))
        }
    }

    fn test_manager(tag: &str) -> (PluginManager, PathBuf, Arc<MemStore>) {
        let root = std::env::temp_dir().join(format!("forgedesk-mgr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let engine = Arc::new(WasmiEngine::with_default_limits(Arc::new(NoopServices)));
        let store = Arc::new(MemStore::default());
        let manager = PluginManager::new(
            engine,
            root.clone(),
            Arc::clone(&store) as Arc<dyn RegistryStore>,
        )
        .unwrap();
        (manager, root, store)
    }

    /// 在 `plugins_root` 内写出一个可安装的插件目录。
    fn write_plugin(root: &Path, dir_name: &str, wasm: &[u8]) -> PathBuf {
        let dir = root.join(dir_name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            r#"{"id": "com.example.installed", "name": "Installed", "version": "1.0.0",
                "apiVersion": "0.1", "author": "t", "license": "MIT",
                "description": "d", "main": "plugin.wasm", "permissions": ["git:read"]}"#,
        )
        .unwrap();
        std::fs::write(dir.join("plugin.wasm"), wasm).unwrap();
        dir
    }

    #[test]
    fn install_from_dir_starts_disabled_and_persists() {
        let (manager, root, store) = test_manager("install");
        let dir = write_plugin(&root, "installed", &fixture::normal_plugin());

        let report = manager.install_from_dir(&dir).unwrap();
        assert_eq!(report.id, "com.example.installed");
        assert!(report.inside_root);
        assert_eq!(report.sha256, sha256_hex(&fixture::normal_plugin()));
        assert_eq!(report.declared_permissions, vec![Permission::GitRead]);

        let summary = &manager.list()[0];
        assert_eq!(summary.state, ManagedState::Disabled);
        assert!(summary.granted_permissions.is_empty(), "安装后授权集为空");
        assert_eq!(store.entries.lock().unwrap().len(), 1);
    }

    #[test]
    fn enable_invoke_disable_round_trip() {
        let (manager, root, _store) = test_manager("roundtrip");
        let dir = write_plugin(&root, "p", &fixture::normal_plugin());
        manager.install_from_dir(&dir).unwrap();
        manager
            .grant("com.example.installed", &[Permission::GitRead])
            .unwrap();

        manager.set_enabled("com.example.installed", true).unwrap();
        assert_eq!(manager.list()[0].state, ManagedState::Enabled);

        let result = manager
            .invoke_command("com.example.installed", "greet", "{}")
            .unwrap();
        assert_eq!(result, "hello world");

        manager.set_enabled("com.example.installed", false).unwrap();
        assert_eq!(manager.list()[0].state, ManagedState::Disabled);
        let error = manager
            .invoke_command("com.example.installed", "greet", "{}")
            .unwrap_err();
        assert!(matches!(error, HostError::InvalidArgument(_, _)), "{error}");
    }

    #[test]
    fn revoking_a_grant_denies_the_next_host_call_immediately() {
        let (manager, root, _store) = test_manager("revoke");
        let dir = write_plugin(
            &root,
            "p",
            &fixture::host_call_plugin(crate::host::HostOp::GetRepoInfo, "{}"),
        );
        manager.install_from_dir(&dir).unwrap();
        manager
            .grant("com.example.installed", &[Permission::GitRead])
            .unwrap();
        manager.set_enabled("com.example.installed", true).unwrap();

        assert!(manager
            .invoke_command("com.example.installed", "repo", "{}")
            .is_ok());

        manager
            .revoke("com.example.installed", Permission::GitRead)
            .unwrap();
        let denied = manager
            .invoke_command("com.example.installed", "repo", "{}")
            .unwrap();
        assert!(denied.contains("-2"), "应携带 PERMISSION_DENIED: {denied}");
        // 授权集已收缩并持久化
        assert!(manager.list()[0].granted_permissions.is_empty());
    }

    #[test]
    fn uninstall_deletes_in_root_directories_and_keeps_external_ones() {
        let (manager, root, store) = test_manager("uninstall");
        // root 内
        let inside = write_plugin(&root, "inside", &fixture::normal_plugin());
        manager.install_from_dir(&inside).unwrap();
        assert!(manager.uninstall("com.example.installed").unwrap());
        assert!(!inside.exists(), "root 内的目录应被删除");
        assert!(manager.list().is_empty());

        // root 外（开发者模式的工作副本）
        let outside_root =
            std::env::temp_dir().join(format!("forgedesk-mgr-external-{}", std::process::id()));
        let outside = write_plugin(&outside_root, "external", &fixture::normal_plugin());
        manager.install_from_dir(&outside).unwrap();
        let removed = manager.uninstall("com.example.installed").unwrap();
        assert!(!removed, "root 外的目录必须保留");
        assert!(outside.exists(), "开发者的工作副本不能被删");
        assert!(manager.list().is_empty());
        assert_eq!(store.entries.lock().unwrap().len(), 0);

        let _ = std::fs::remove_dir_all(&outside_root);
    }

    #[test]
    fn registry_survives_manager_restart_with_grants() {
        let (manager, root, store) = test_manager("restart");
        let dir = write_plugin(&root, "p", &fixture::normal_plugin());
        manager.install_from_dir(&dir).unwrap();
        manager
            .grant("com.example.installed", &[Permission::GitRead])
            .unwrap();
        drop(manager);

        // 新 manager 从同一 store + 目录恢复
        let engine = Arc::new(WasmiEngine::with_default_limits(Arc::new(NoopServices)));
        let manager = PluginManager::new(
            engine,
            root.clone(),
            Arc::clone(&store) as Arc<dyn RegistryStore>,
        )
        .unwrap();
        let summary = &manager.list()[0];
        assert_eq!(summary.id, "com.example.installed");
        assert_eq!(
            summary.state,
            ManagedState::Disabled,
            "恢复后默认禁用，等用户启用"
        );
        assert_eq!(summary.granted_permissions, vec![Permission::GitRead]);
    }

    #[test]
    fn entries_with_missing_or_tampered_directories_are_skipped_on_restart() {
        let (manager, root, store) = test_manager("tamper");
        let dir = write_plugin(&root, "p", &fixture::normal_plugin());
        manager.install_from_dir(&dir).unwrap();
        drop(manager);

        // 篡改 wasm（sha 不再匹配）
        std::fs::write(root.join("p").join("plugin.wasm"), b"tampered").unwrap();

        let engine = Arc::new(WasmiEngine::with_default_limits(Arc::new(NoopServices)));
        let manager = PluginManager::new(
            engine,
            root.clone(),
            Arc::clone(&store) as Arc<dyn RegistryStore>,
        )
        .unwrap();
        assert!(
            manager.list().is_empty(),
            "sha 不匹配的条目必须跳过而不是加载"
        );
    }

    #[test]
    fn a_plugin_that_fails_to_activate_is_marked_crashed() {
        let (manager, root, _store) = test_manager("crash");
        let dir = write_plugin(&root, "p", &fixture::bomb_plugin());
        manager.install_from_dir(&dir).unwrap();
        manager
            .grant("com.example.installed", &[Permission::GitRead])
            .unwrap();

        manager
            .set_enabled("com.example.installed", true)
            .unwrap_err();
        assert_eq!(manager.list()[0].state, ManagedState::Crashed);

        // 崩溃的插件可以重试启用（重新加载新实例）
        // bomb 插件每次激活都会超时，这里只验证状态机允许重试路径编译为 Crashed
        manager
            .set_enabled("com.example.installed", true)
            .unwrap_err();
        assert_eq!(manager.list()[0].state, ManagedState::Crashed);
    }

    #[test]
    fn reload_rereads_the_directory_and_keeps_state() {
        let (manager, root, _store) = test_manager("reload");
        let dir = write_plugin(&root, "p", &fixture::normal_plugin());
        manager.install_from_dir(&dir).unwrap();
        manager
            .grant("com.example.installed", &[Permission::GitRead])
            .unwrap();
        manager.set_enabled("com.example.installed", true).unwrap();

        manager.reload("com.example.installed").unwrap();
        assert_eq!(manager.list()[0].state, ManagedState::Enabled);
        let result = manager
            .invoke_command("com.example.installed", "greet", "{}")
            .unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn duplicate_ids_are_refused_at_install() {
        let (manager, root, _store) = test_manager("dup");
        let dir = write_plugin(&root, "a", &fixture::normal_plugin());
        manager.install_from_dir(&dir).unwrap();
        // 同 id 第二份目录（内容不同也拒）
        let dir2 = write_plugin(&root, "b", &fixture::normal_plugin());
        let error = manager.install_from_dir(&dir2).unwrap_err();
        assert!(error.to_string().contains("already installed"), "{error}");
    }
}
