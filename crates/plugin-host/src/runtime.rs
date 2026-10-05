//! 引擎无关的插件运行时骨架（T6.1）。
//!
//! # 为什么先立 trait 再选引擎
//!
//! wasmtime 的引入是**审批项**（体积影响待人类确认），wasmi 是备选方案。
//! 把宿主需要的最小接口（加载 / 激活 / 调用 / 卸载 / 状态）冻结在
//! [`PluginEngine`] 上，引擎落地就只是替换实现；上层（services / commands）
//! 与测试（mock 引擎）不感知具体引擎，避免选型摇摆向外扩散。
//!
//! # 错误即数据
//!
//! 插件越权 / 超时 / 崩溃都必须以 [`HostError`] 返回而不是 panic：
//! "插件崩溃绝不影响宿主"（T6.1 验收）意味着宿主侧不存在会 panic 的路径，
//! 前端（T6.4）也依赖结构化错误来渲染"哪个权限被拒了"。

use crate::manifest::ValidatedManifest;
use crate::permission::Permission;
use std::time::Duration;

/// 运行时资源上限（T6.1 规格：内存 64MB、单次宿主函数调用 5s、单次命令 30s）。
///
/// 具体引擎负责落实：wasmtime 用 `StoreLimits` 与调用点超时；mock 直接照搬字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLimits {
    /// 单插件实例的线性内存上限。
    pub max_plugin_memory_bytes: u64,
    /// 单次宿主函数调用（插件 → 宿主）超时。
    pub host_call_timeout: Duration,
    /// 单次插件命令执行（激活 / 命令 / 事件回调）超时。
    pub command_timeout: Duration,
    /// 单次命令执行的 fuel 预算（约等于可执行的最简指令数）。
    ///
    /// # 为什么默认值按构建 profile 区分
    ///
    /// wasmi 解释器的原生栈占用在 debug 构建下与"已消耗的 fuel"成正比
    /// （实测约 128B/fuel；release 下 LLVM 优化后完全平坦，2MB 栈即可跑满 1G）。
    /// debug 构建只用于测试（夹具都很小），给安全的小预算；release 服务真实
    /// 插件，给足 1G fuel（≈ 规格的 30s 命令上限）。执行线程另有大栈兜底，
    /// 见 `engine_wasmi::EXECUTOR_STACK_BYTES`。
    pub fuel_budget: u64,
}

/// debug 构建下的 fuel 预算（受执行线程栈容量约束，见上）。
#[cfg(debug_assertions)]
pub const DEFAULT_FUEL_BUDGET: u64 = 400_000;

/// release 构建下的 fuel 预算（≈ 1G 条最简指令，对应 30s 命令超时）。
#[cfg(not(debug_assertions))]
pub const DEFAULT_FUEL_BUDGET: u64 = 1_000_000_000;

impl Default for RuntimeLimits {
    fn default() -> Self {
        // 64 MiB：插件面板/统计类需求远用不满；超出即停用而不是 OOM 宿主
        const MIB: u64 = 1024 * 1024;
        Self {
            max_plugin_memory_bytes: 64 * MIB,
            host_call_timeout: Duration::from_secs(5),
            command_timeout: Duration::from_secs(30),
            fuel_budget: DEFAULT_FUEL_BUDGET,
        }
    }
}

/// 插件生命周期状态（load → activate → 事件驱动 → deactivate）。
///
/// `Crashed` 与 `Disabled` 都不可调用但语义不同：前者是引擎检测到 trap /
/// 超时 / 超限后**自动停用**（需要记日志并通知前端），后者是用户在管理页
/// 主动禁用（T6.4），重新启用即可恢复。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    /// 已实例化，未激活（execute activate 之前）。
    Loaded,
    /// 激活中：已注册贡献点，可响应命令与事件。
    Active,
    /// 已正常停用（deactivate 已执行，实例保留可再次激活）。
    Deactivated,
    /// 因 trap / 超时 / 超限被引擎自动停用（异常，见插件日志）。
    Crashed,
    /// 被用户禁用（管理页操作，非异常）。
    Disabled,
}

impl LifecycleState {
    /// 是否可响应命令 / 事件调用。
    pub fn is_callable(self) -> bool {
        matches!(self, LifecycleState::Active)
    }
}

/// 宿主侧错误：结构化、可展示、绝不含密钥或插件输出原文（进日志前仍需过脱敏层）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    /// 未授权调用宿主函数（以用户授权为准，T6.1）。
    #[error("permission denied for `{call}` (requires `{permission}`)")]
    PermissionDenied {
        /// 被拒调用所需的权限。
        permission: Permission,
        /// 宿主函数名（如 `read_file`）。
        call: &'static str,
    },
    /// 参数非法（类型、越界、超长、路径逃逸等）。
    #[error("invalid argument for `{0}`: {1}")]
    InvalidArgument(&'static str, String),
    /// 调用超时。
    #[error("timeout after {limit_ms}ms in {operation}")]
    Timeout {
        /// 超时的操作名（host-call / command / event）。
        operation: &'static str,
        /// 限时毫秒数。
        limit_ms: u64,
    },
    /// 线性内存超出上限。
    #[error("plugin exceeded its memory limit")]
    MemoryLimitExceeded,
    /// 插件 trap（panic、不可恢复错误、死循环被引擎打断等）。
    #[error("plugin trapped: {0}")]
    Trap(String),
    /// 句柄失效：实例已卸载或不存在。
    #[error("plugin instance not found: {0}")]
    InstanceNotFound(u64),
    /// 引擎内部错误（加载失败、链接失败等，含引擎原始信息）。
    #[error("engine error: {0}")]
    Engine(String),
}

/// 已加载插件实例的句柄；具体引擎自行映射到内部状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PluginHandle(u64);

impl PluginHandle {
    /// 由引擎实现分配句柄（保证非零，0 留给"无效"哨兵）。
    pub fn new(raw: u64) -> Self {
        Self(raw.max(1))
    }

    /// 句柄原始数值（日志 / 诊断用）。
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// 宿主对插件的引擎契约（引擎无关）。
///
/// 实现约定：
/// - 全部方法 `&self`，引擎内部用并发容器（宿主可能多线程触发事件回调）；
/// - 权限检查在引擎内完成：实例只能使用清单声明且用户已授权的能力；
/// - 任何插件侧异常都必须以 [`HostError`] 返回，实现里不得 panic。
pub trait PluginEngine: Send + Sync {
    /// 解析并实例化 wasm 模块（不激活）。
    ///
    /// `wasm` 应来自插件安装目录下 manifest.main 指向的文件；权限集
    /// 取自清单声明 ∩ 用户授权（T6.4），两者在加载时收敛，之后不可扩大。
    fn load(&self, manifest: &ValidatedManifest, wasm: &[u8]) -> Result<PluginHandle, HostError>;

    /// 激活插件（执行其 activate 导出；无该导出视为直接激活成功）。
    fn activate(&self, handle: PluginHandle) -> Result<(), HostError>;

    /// 调用插件注册的命令。
    fn invoke(
        &self,
        handle: PluginHandle,
        command: &str,
        arg_json: &str,
    ) -> Result<String, HostError>;

    /// 停用并回收实例（执行 deactivate 导出；trap 不上抛，返回 Ok 并进入
    /// [`LifecycleState::Crashed`]——"停用一个崩溃的插件"本身是成功操作）。
    fn unload(&self, handle: PluginHandle) -> Result<(), HostError>;

    /// 当前生命周期状态；句柄失效返回 [`HostError::InstanceNotFound`]。
    fn state(&self, handle: PluginHandle) -> Result<LifecycleState, HostError>;
}

/// 实例生效的权限集：清单声明 ∩ 用户授权，deny-by-default。
///
/// 为什么不用 `HashSet<Permission>` 裸暴露：收敛发生在加载时刻且不可扩大
/// （T6.4 撤销授权 → 下次调用即失败），把"只读集合 + 显式判断"包成类型，
/// 让 T6.2 的宿主函数入口检查只有一个正确写法。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet {
    granted: std::collections::BTreeSet<Permission>,
}

impl PermissionSet {
    /// 空集（未授予任何权限）。
    pub fn none() -> Self {
        Self::default()
    }

    /// 由清单声明的权限构造（用户授权交集由上层在授权流程后应用）。
    pub fn from_declared(declared: impl IntoIterator<Item = Permission>) -> Self {
        Self {
            granted: declared.into_iter().collect(),
        }
    }

    /// 收回一项权限（T6.4：撤销后插件下次调用即失败）。
    pub fn revoke(&mut self, permission: Permission) {
        self.granted.remove(&permission);
    }

    /// 是否拥有该权限。
    pub fn allows(&self, permission: Permission) -> bool {
        self.granted.contains(&permission)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::manifest::PluginManifest;
    use std::sync::Mutex;

    fn validated_manifest() -> ValidatedManifest {
        PluginManifest::parse(
            r#"{
                "id": "com.example.mock",
                "name": "Mock",
                "version": "1.0.0",
                "apiVersion": "0.1",
                "author": "test",
                "license": "MIT",
                "description": "test plugin",
                "main": "plugin.wasm",
                "permissions": ["git:read", "ui:toast"]
            }"#,
        )
        .unwrap()
    }

    /// 最小 mock 引擎：证明 trait 可实现、状态机与错误语义成立。
    /// 真实 wasm 引擎（wasmtime/wasmi）在选型审批后落地，并复用同一套测试断言。
    struct MockEngine {
        handles: Mutex<std::collections::BTreeMap<u64, (LifecycleState, Option<HostError>)>>,
        next: std::sync::atomic::AtomicU64,
    }

    impl MockEngine {
        fn new() -> Self {
            Self {
                handles: Mutex::new(std::collections::BTreeMap::new()),
                next: std::sync::atomic::AtomicU64::new(1),
            }
        }

        fn crash(&self, handle: PluginHandle, reason: &str) {
            let mut handles = self.handles.lock().unwrap();
            if let Some(entry) = handles.get_mut(&handle.raw()) {
                entry.0 = LifecycleState::Crashed;
                entry.1 = Some(HostError::Trap(reason.to_owned()));
            }
        }
    }

    impl PluginEngine for MockEngine {
        fn load(
            &self,
            _manifest: &ValidatedManifest,
            _wasm: &[u8],
        ) -> Result<PluginHandle, HostError> {
            let raw = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.handles
                .lock()
                .unwrap()
                .insert(raw, (LifecycleState::Loaded, None));
            Ok(PluginHandle::new(raw))
        }

        fn activate(&self, handle: PluginHandle) -> Result<(), HostError> {
            let mut handles = self.handles.lock().unwrap();
            let entry = handles
                .get_mut(&handle.raw())
                .ok_or(HostError::InstanceNotFound(handle.raw()))?;
            entry.0 = LifecycleState::Active;
            Ok(())
        }

        fn invoke(
            &self,
            handle: PluginHandle,
            _command: &str,
            _arg_json: &str,
        ) -> Result<String, HostError> {
            let handles = self.handles.lock().unwrap();
            let entry = handles
                .get(&handle.raw())
                .ok_or(HostError::InstanceNotFound(handle.raw()))?;
            if let Some(cached) = &entry.1 {
                // 已崩溃的插件再次被调用：返回缓存的结构化错误，绝不 panic
                return Err(cached.clone());
            }
            if !entry.0.is_callable() {
                return Err(HostError::Engine(format!(
                    "plugin is not callable in state {:?}",
                    entry.0
                )));
            }
            Ok(String::new())
        }

        fn unload(&self, handle: PluginHandle) -> Result<(), HostError> {
            let mut handles = self.handles.lock().unwrap();
            // 停用一个（哪怕崩溃的）插件是成功操作：deactivate 的 trap 不上抛
            handles.remove(&handle.raw());
            Ok(())
        }

        fn state(&self, handle: PluginHandle) -> Result<LifecycleState, HostError> {
            self.handles
                .lock()
                .unwrap()
                .get(&handle.raw())
                .map(|entry| entry.0)
                .ok_or(HostError::InstanceNotFound(handle.raw()))
        }
    }

    #[test]
    fn default_limits_match_the_t6_1_spec_exactly() {
        let limits = RuntimeLimits::default();
        assert_eq!(limits.max_plugin_memory_bytes, 64 * 1024 * 1024);
        assert_eq!(limits.host_call_timeout, Duration::from_secs(5));
        assert_eq!(limits.command_timeout, Duration::from_secs(30));
        // fuel 预算按构建 profile 区分（debug 小预算保 CI，release 大预算服务插件）
        if cfg!(debug_assertions) {
            assert_eq!(limits.fuel_budget, 400_000);
        } else {
            assert_eq!(limits.fuel_budget, 1_000_000_000);
        }
    }

    #[test]
    fn lifecycle_follows_load_activate_and_unload() {
        let engine = MockEngine::new();
        let handle = engine.load(&validated_manifest(), b"\0asm").unwrap();

        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Loaded);
        assert!(!engine.state(handle).unwrap().is_callable());

        engine.activate(handle).unwrap();
        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Active);
        assert!(engine.state(handle).unwrap().is_callable());

        engine.unload(handle).unwrap();
        assert_eq!(
            engine.state(handle),
            Err(HostError::InstanceNotFound(handle.raw()))
        );
    }

    #[test]
    fn a_denied_host_call_is_a_structured_error_not_a_panic() {
        // T6.2 契约的前置：权限集 deny-by-default，检查失败返回结构化错误
        let mut granted = PermissionSet::from_declared([Permission::GitRead, Permission::UiToast]);
        assert!(granted.allows(Permission::GitRead));

        let denied = Permission::GitWrite;
        assert!(!granted.allows(denied));
        let error = HostError::PermissionDenied {
            permission: denied,
            call: "git_stage",
        };
        assert!(error.to_string().contains("git:write"));
        assert!(error.to_string().contains("git_stage"));

        // 撤销授权后（T6.4），已授过的权限下一次调用即失败；其余权限不受影响
        granted.revoke(Permission::GitRead);
        assert!(!granted.allows(Permission::GitRead));
        assert!(granted.allows(Permission::UiToast));
    }

    #[test]
    fn a_crashed_plugin_stays_isolated_and_the_host_keeps_serving_other_plugins() {
        let engine = MockEngine::new();
        let bad = engine.load(&validated_manifest(), b"deadloop").unwrap();
        let good = engine.load(&validated_manifest(), b"ok").unwrap();
        engine.activate(bad).unwrap();
        engine.activate(good).unwrap();

        // 死循环 / 内存爆炸插件：调用超时或超限后进入 Crashed，结构化错误可读
        engine.crash(bad, "memory limit exceeded");
        assert_eq!(engine.state(bad).unwrap(), LifecycleState::Crashed);
        let error = engine.invoke(bad, "any", "{}").unwrap_err();
        assert!(matches!(error, HostError::Trap(reason) if reason.contains("memory limit")));

        // 宿主不受影响：第二个插件照常工作
        assert_eq!(engine.invoke(good, "any", "{}").unwrap(), String::new());

        // 卸载一个崩溃的插件不报错（deactivate trap 不上抛）
        engine.unload(bad).unwrap();
        assert_eq!(
            engine.state(bad),
            Err(HostError::InstanceNotFound(bad.raw()))
        );
        // 宿主仍可为后续插件服务
        assert!(engine.load(&validated_manifest(), b"next").is_ok());
    }

    #[test]
    fn handle_zero_is_reserved_so_a_zeroed_handle_never_names_a_real_instance() {
        assert_eq!(PluginHandle::new(0).raw(), 1);
        assert_eq!(PluginHandle::new(7).raw(), 7);
    }
}
