//! [`PluginEngine`] 的 wasmi 实现（T6.1）。
//!
//! # 执行边界怎么落实
//!
//! wasmi 是解释器，没有 wasmtime 的 epoch 中断，执行边界的两根支柱是：
//!
//! 1. **fuel 预算**（确定性）：引擎以 `consume_fuel(true)` 构建，每次命令调用前
//!    重置为 [`RuntimeLimits::fuel_budget`]（按构建 profile 取默认值，见其文档）。
//!    超出 → `TrapCode::OutOfFuel` → 映射为 [`HostError::Timeout`]；墙钟超时
//!    （5s 宿主函数调用）随 T6.2 的宿主函数入口检查落地——宿主函数是唯一能在
//!    插件执行中途看到墙钟的地方。
//! 2. **内存上限**（`StoreLimits`）：线性内存增长超过 [`RuntimeLimits::max_plugin_memory_bytes`]
//!    时 `memory.grow` 返回 -1（wasm 规范行为，不是 trap），表格/实例数同样受限。
//!
//! # 调用约定（T6.1 阶段的最小 ABI）
//!
//! - 插件必须导出 `memory`；
//! - `fd_alloc(len: i32) -> i32`：在插件线性内存里划出一块可写区域，返回指针；
//! - `fd_invoke(ptr: i32, len: i32) -> i64`：参数是宿主写入 `fd_alloc` 区域的
//!   JSON 字节；返回值打包为 `(结果指针 << 32) | 结果长度`；
//! - `fd_activate() -> i32` / `fd_deactivate() -> i32`：可选；返回值暂不解释，
//!   生命周期失败只由 trap / 资源超限触发（完整 ABI 随 T6.2 的 PLUGIN-API 文档定稿）。
//!
//! # 崩溃隔离
//!
//! 所有 wasmi 错误都经 [`WasmiEngine::map_error`] 归一为 [`HostError`]；
//! trap 后实例标记 [`LifecycleState::Crashed`] 并缓存失败原因，后续调用直接返回
//! 缓存错误。实例各自的 `Store` 完全独立，一个实例 trap 不触碰其它实例的状态。

use parking_lot::Mutex;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::host::{self, SharedServices};
use crate::manifest::ValidatedManifest;
use crate::runtime::{
    HostError, LifecycleState, PermissionSet, PluginEngine, PluginHandle, RuntimeLimits,
};

/// 插件返回值的最大字节数（防止一个返回值拖垮宿主内存）。
const MAX_RESULT_BYTES: usize = 1024 * 1024;

/// 插件执行线程的原生栈大小。
///
/// 为什么需要专用执行线程：wasmi 解释/翻译链路的原生栈占用在 debug 构建下
/// 极深（wasmi 自己的注释也记录过 Windows CI 上因 inline 膨胀导致的栈溢出；
/// 本仓库实测 debug 下超过 32MB），宿主主线程/测试线程的默认栈远远不够。
/// 让所有 wasm 执行在带大栈的专用线程上进行，宿主线程的栈需求归零；
/// 栈是虚拟内存预留（按需提交），64MB 预留的真实内存占用很小。
/// wasm 层面的递归深度由 wasmi 的 `TrapCode::StackOverflow` 上限拦截，
/// 不会穿透到原生栈溢出。
const EXECUTOR_STACK_BYTES: usize = 64 * 1024 * 1024;

/// 在专用执行线程上运行 `f`（同步等待完成）。
///
/// 每次调用现建现毁一个 scoped 线程：插件命令不是热路径，线程创建的开销
/// （微秒级）远小于把宿主栈撑大的常驻代价。panic 被归一为结构化错误。
fn on_executor_stack<T, F>(f: F) -> Result<T, HostError>
where
    F: FnOnce() -> Result<T, HostError> + Send,
    T: Send,
{
    let joined = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(EXECUTOR_STACK_BYTES)
            .spawn_scoped(scope, f)
            .map_err(|error| {
                HostError::Engine(format!(
                    "could not spawn the plugin executor thread: {error}"
                ))
            })?
            .join()
            .map_err(|_| HostError::Engine("the plugin executor thread panicked".to_owned()))
    });
    // scope 直接返回闭包值；spawn/join 两层错误都是 HostError，依次展开
    joined?
}

/// 宿主函数（未来的 T6.2）与内部检查用来打断插件的错误载体。
#[derive(Debug)]
struct EngineStop(HostError);

impl std::fmt::Display for EngineStop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl wasmi::errors::HostError for EngineStop {}

/// 单条插件日志（`fd.log` 写入；T6.4 日志页消费）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PluginLogEntry {
    /// Unix 毫秒时间戳（前端用 Intl 格式化）。
    pub time_ms: i64,
    /// 0=debug 1=info 2=warn 3=error（fd.log 的 level 原值）。
    pub level: u8,
    /// 插件给出的文本（≤ 8 KiB，进日志前由脱敏层兜底）。
    pub message: String,
}

/// 每实例日志环形缓冲上限。
const PLUGIN_LOG_CAP: usize = 500;

/// 每个 Store 挂的资源上限、fuel 预算与宿主调用上下文
/// （limiter 闭包与 `fd.*` 导入都要从 data 取）。
struct PluginData {
    limits: wasmi::StoreLimits,
    fuel_budget: u64,
    plugin_id: String,
    granted: PermissionSet,
    services: SharedServices,
    /// `fd.host_call` 的结果/错误详情 staging，`fd.host_result` 取回。
    staging: Vec<u8>,
    /// `fd.log` 的环形缓冲（T6.4 日志页）。
    logs: Mutex<VecDeque<PluginLogEntry>>,
    /// 成功的宿主调用计数（按所需权限聚合；T6.4 权限面板的"使用次数"）。
    usage: Mutex<BTreeMap<crate::permission::Permission, u64>>,
}

struct PluginInstance {
    store: wasmi::Store<PluginData>,
    instance: wasmi::Instance,
    state: LifecycleState,
    last_failure: Option<HostError>,
}

/// wasmi 引擎。构建开销一次；实例间经 `Store` 天然隔离。
pub struct WasmiEngine {
    engine: wasmi::Engine,
    limits: RuntimeLimits,
    services: SharedServices,
    instances: parking_lot::RwLock<BTreeMap<u64, PluginInstance>>,
    next_id: AtomicU64,
}

impl WasmiEngine {
    /// 以给定资源上限与宿主服务实现构建引擎。
    pub fn new(limits: RuntimeLimits, services: SharedServices) -> Self {
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        Self {
            engine: wasmi::Engine::new(&config),
            limits,
            services,
            instances: parking_lot::RwLock::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// 以规格默认上限构建（64MB / 5s / 30s / fuel 按 profile）。
    pub fn with_default_limits(services: SharedServices) -> Self {
        Self::new(RuntimeLimits::default(), services)
    }

    /// 把 wasmi 错误归一为结构化 [`HostError`]。
    ///
    /// 归一顺序：宿主侧主动打断（[`EngineStop`]）→ 资源类（fuel 耗尽）→ 其余一律
    /// 按插件 trap 处理。fuel 耗尽语义上就是"执行超过时限"。
    fn map_error(&self, error: wasmi::Error, operation: &'static str) -> HostError {
        if let Some(stop) = error.downcast_ref::<EngineStop>() {
            return stop.0.clone();
        }
        let out_of_fuel = matches!(
            error.kind(),
            wasmi::errors::ErrorKind::TrapCode(wasmi::TrapCode::OutOfFuel)
                | wasmi::errors::ErrorKind::Memory(wasmi::errors::MemoryError::OutOfFuel { .. })
                | wasmi::errors::ErrorKind::Table(wasmi::errors::TableError::OutOfFuel { .. })
                // fuel 在宿主函数边界耗尽时，wasmi 以可恢复错误的形式冒出
                | wasmi::errors::ErrorKind::ResumableOutOfFuel(_)
        );
        if out_of_fuel {
            return HostError::Timeout {
                operation,
                limit_ms: self.limits.command_timeout.as_millis() as u64,
            };
        }
        HostError::Trap(error.to_string())
    }

    fn memory_of(
        instance: &wasmi::Instance,
        store: &wasmi::Store<PluginData>,
    ) -> Result<wasmi::Memory, HostError> {
        instance
            .get_export(store, "memory")
            .and_then(|entry| entry.into_memory())
            .ok_or_else(|| HostError::Engine("plugin does not export its linear memory".to_owned()))
    }

    /// 导入（host function）内读取插件内存中的一段字节。
    fn import_read_bytes(
        caller: &wasmi::Caller<'_, PluginData>,
        field: &'static str,
        ptr: i32,
        len: i32,
        max: usize,
    ) -> Result<Vec<u8>, HostError> {
        if ptr < 0 || len < 0 {
            return Err(HostError::InvalidArgument(
                field,
                "negative ptr/len".to_owned(),
            ));
        }
        let len = len as usize;
        if len > max {
            return Err(HostError::InvalidArgument(
                field,
                "exceeds the size cap".to_owned(),
            ));
        }
        let memory = caller
            .get_export("memory")
            .and_then(|entry| entry.into_memory())
            .ok_or_else(|| {
                HostError::Engine("plugin does not export its linear memory".to_owned())
            })?;
        let size = memory.data_size(caller);
        let start = ptr as usize;
        if start.checked_add(len).is_none_or(|end| end > size) {
            return Err(HostError::InvalidArgument(
                field,
                "out of the plugin's memory bounds".to_owned(),
            ));
        }
        let mut buffer = vec![0u8; len];
        memory
            .read(caller, start, &mut buffer)
            .map_err(|error| HostError::Engine(format!("reading plugin memory failed: {error}")))?;
        Ok(buffer)
    }

    /// 注册 `fd.*` 导入（T6.2 宿主函数）。
    ///
    /// 对所有插件注册同一套导入：能力裁剪不在这里做——清单声明之外的能力
    /// 由派发器的权限表拦截（结构保证），导入集合只暴露"存在哪些操作"，
    /// 不暴露"这个插件被允许哪些操作"。
    fn register_imports(linker: &mut wasmi::Linker<PluginData>) -> Result<(), HostError> {
        linker
            .func_wrap(
                "fd",
                "log",
                |caller: wasmi::Caller<'_, PluginData>, level: i32, ptr: i32, len: i32| {
                    let message = WasmiEngine::import_read_bytes(
                        &caller,
                        "log",
                        ptr,
                        len,
                        host::MAX_LOG_BYTES,
                    );
                    let Ok(message) = message else {
                        return; // 读不出来就丢弃：日志是尽力而为的通道
                    };
                    let text = String::from_utf8_lossy(&message).into_owned();
                    let plugin_id = caller.data().plugin_id.clone();
                    match level {
                        2 => tracing::warn!(target: "plugin", plugin_id, "{text}"),
                        3 => tracing::error!(target: "plugin", plugin_id, "{text}"),
                        0 => tracing::debug!(target: "plugin", plugin_id, "{text}"),
                        _ => tracing::info!(target: "plugin", plugin_id, "{text}"),
                    }
                    // 环形缓冲：T6.4 日志页的数据源（实例内，随卸载消失）
                    let time_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
                        .unwrap_or_default();
                    let mut logs = caller.data().logs.lock();
                    if logs.len() >= PLUGIN_LOG_CAP {
                        logs.pop_front();
                    }
                    logs.push_back(PluginLogEntry {
                        time_ms,
                        level: level.clamp(0, 3) as u8,
                        message: text,
                    });
                },
            )
            .map_err(|error| HostError::Engine(format!("could not register fd.log: {error}")))?;
        linker
            .func_wrap(
                "fd",
                "host_call",
                |mut caller: wasmi::Caller<'_, PluginData>,
                 op: i32,
                 arg_ptr: i32,
                 arg_len: i32|
                 -> i32 {
                    let args = match WasmiEngine::import_read_bytes(
                        &caller,
                        "args",
                        arg_ptr,
                        arg_len,
                        host::MAX_ARG_BYTES,
                    ) {
                        Ok(args) => args,
                        Err(error) => {
                            caller.data_mut().staging.clear();
                            host::stage_error(&mut caller.data_mut().staging, &error);
                            return host::error_code(&error);
                        }
                    };
                    let data = caller.data();
                    let plugin_id = data.plugin_id.clone();
                    let granted = data.granted.clone();
                    let services = std::sync::Arc::clone(&data.services);
                    let code = host::dispatch_host_call(
                        &plugin_id,
                        &granted,
                        services.as_ref(),
                        op,
                        &args,
                        &mut caller.data_mut().staging,
                    );
                    // "该权限被使用的次数"（T6.4 权限面板）：只计成功调用
                    if code == host::ERR_OK {
                        if let Some(host_op) = crate::host::HostOp::from_id(op) {
                            *caller
                                .data_mut()
                                .usage
                                .lock()
                                .entry(host_op.permission())
                                .or_default() += 1;
                        }
                    }
                    code
                },
            )
            .map_err(|error| {
                HostError::Engine(format!("could not register fd.host_call: {error}"))
            })?;
        linker
            .func_wrap(
                "fd",
                "host_result",
                |mut caller: wasmi::Caller<'_, PluginData>, out_ptr: i32, out_cap: i32| -> i32 {
                    if out_ptr < 0 || out_cap < 0 {
                        return host::ERR_INVALID_ARGUMENT;
                    }
                    let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory())
                    else {
                        return host::ERR_GENERIC;
                    };
                    let size = memory.data_size(&caller);
                    let start = out_ptr as usize;
                    let cap = out_cap as usize;
                    if start.checked_add(cap).is_none_or(|end| end > size) {
                        return host::ERR_INVALID_ARGUMENT;
                    }
                    let mut buffer = vec![0u8; cap];
                    match host::take_staged_result_into(&mut caller.data_mut().staging, &mut buffer)
                    {
                        Ok(0) => 0,
                        Ok(written) => {
                            memory
                                .write(&mut caller, start, &buffer[..written])
                                .unwrap_or_else(|_| {
                                    tracing::error!(
                                        "writing host result into plugin memory failed"
                                    );
                                });
                            written as i32
                        }
                        Err(code) => code,
                    }
                },
            )
            .map_err(|error| {
                HostError::Engine(format!("could not register fd.host_result: {error}"))
            })?;
        Ok(())
    }

    fn reset_fuel(store: &mut wasmi::Store<PluginData>) -> Result<(), HostError> {
        store
            .set_fuel(store.data().fuel_budget)
            .map_err(|error| HostError::Engine(format!("could not reset fuel: {error}")))
    }

    fn cached_failure_or(state: LifecycleState, last_failure: Option<&HostError>) -> HostError {
        match (state, last_failure) {
            (LifecycleState::Crashed, Some(failure)) => failure.clone(),
            (state, _) => HostError::Engine(format!("plugin is not callable in state {state:?}")),
        }
    }
}

impl PluginEngine for WasmiEngine {
    fn load(&self, manifest: &ValidatedManifest, wasm: &[u8]) -> Result<PluginHandle, HostError> {
        // 翻译 + 实例化是原生栈需求最深的一段，放到执行线程上
        let engine = &self.engine;
        let limits = self.limits;
        let services = std::sync::Arc::clone(&self.services);
        let (store, instance) = on_executor_stack(move || {
            let module = wasmi::Module::new(engine, wasm).map_err(|error| {
                HostError::Engine(format!("wasm module failed to load: {error}"))
            })?;
            let mut store = wasmi::Store::new(
                engine,
                PluginData {
                    limits: wasmi::StoreLimitsBuilder::new()
                        .memory_size(limits.max_plugin_memory_bytes as usize)
                        .build(),
                    fuel_budget: limits.fuel_budget,
                    plugin_id: manifest.id.clone(),
                    // T6.4 会在这里取"清单声明 ∩ 用户逐项授权"的交集；
                    // 当前阶段以清单声明为准（与 T6.1 行为一致）
                    granted: PermissionSet::from_declared(manifest.permissions.iter().copied()),
                    services: std::sync::Arc::clone(&services),
                    staging: Vec::new(),
                    logs: Mutex::new(VecDeque::new()),
                    usage: Mutex::new(BTreeMap::new()),
                },
            );
            store.limiter(|data: &mut PluginData| &mut data.limits);
            let mut linker = <wasmi::Linker<PluginData>>::new(engine);
            Self::register_imports(&mut linker)?;
            let instance = linker
                .instantiate_and_start(&mut store, &module)
                .map_err(|error| {
                    HostError::Engine(format!("wasm module failed to instantiate: {error}"))
                })?;
            Ok((store, instance))
        })?;

        let raw = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.instances.write().insert(
            raw,
            PluginInstance {
                store,
                instance,
                state: LifecycleState::Loaded,
                last_failure: None,
            },
        );
        Ok(PluginHandle::new(raw))
    }

    fn activate(&self, handle: PluginHandle) -> Result<(), HostError> {
        let mut instances = self.instances.write();
        let plugin = instances
            .get_mut(&handle.raw())
            .ok_or(HostError::InstanceNotFound(handle.raw()))?;
        match plugin.state {
            LifecycleState::Active => return Ok(()),
            LifecycleState::Crashed | LifecycleState::Disabled => {
                return Err(Self::cached_failure_or(
                    plugin.state,
                    plugin.last_failure.as_ref(),
                ));
            }
            LifecycleState::Loaded | LifecycleState::Deactivated => {}
        }
        on_executor_stack(|| -> Result<(), HostError> {
            Self::reset_fuel(&mut plugin.store)?;
            let result = plugin
                .instance
                .get_typed_func::<(), i32>(&plugin.store, "fd_activate")
                .ok()
                .map(|activate| activate.call(&mut plugin.store, ()));
            match result {
                Some(Ok(_)) => {
                    plugin.state = LifecycleState::Active;
                    Ok(())
                }
                Some(Err(error)) => {
                    let mapped = self.map_error(error, "command");
                    plugin.state = LifecycleState::Crashed;
                    plugin.last_failure = Some(mapped.clone());
                    Err(mapped)
                }
                // 没有 fd_activate 导出的插件视为立即激活成功
                None => {
                    plugin.state = LifecycleState::Active;
                    Ok(())
                }
            }
        })
    }

    fn invoke(
        &self,
        handle: PluginHandle,
        command: &str,
        arg_json: &str,
    ) -> Result<String, HostError> {
        let mut instances = self.instances.write();
        let plugin = instances
            .get_mut(&handle.raw())
            .ok_or(HostError::InstanceNotFound(handle.raw()))?;
        if !plugin.state.is_callable() {
            return Err(Self::cached_failure_or(
                plugin.state,
                plugin.last_failure.as_ref(),
            ));
        }
        on_executor_stack(move || -> Result<String, HostError> {
            Self::reset_fuel(&mut plugin.store)?;
            let memory = Self::memory_of(&plugin.instance, &plugin.store)?;

            let alloc = plugin
                .instance
                .get_typed_func::<i32, i32>(&plugin.store, "fd_alloc")
                .map_err(|_| HostError::Engine("plugin does not export fd_alloc".to_owned()))?;
            let invoke = plugin
                .instance
                .get_typed_func::<(i32, i32), i64>(&plugin.store, "fd_invoke")
                .map_err(|_| HostError::Engine("plugin does not export fd_invoke".to_owned()))?;

            // 命令与参数合并成一个 payload：插件从 `command` 字段知道该执行什么
            //（arg_json 必须是合法 JSON 对象/值，否则按参数错误处理）
            let args_value: serde_json::Value = serde_json::from_str(arg_json)
                .map_err(|error| HostError::InvalidArgument("arg_json", error.to_string()))?;
            let payload = format!(
                "{{\"command\":{},\"args\":{}}}",
                serde_json::to_string(command).unwrap_or_default(),
                args_value
            );
            let args = payload.as_bytes();
            let args_len = i32::try_from(args.len())
                .map_err(|_| HostError::InvalidArgument("arg_json", "too large".to_owned()))?;
            let ptr = alloc
                .call(&mut plugin.store, args_len)
                .map_err(|error| self.map_error(error, "command"))?;
            if ptr < 0 {
                return Err(HostError::InvalidArgument(
                    "fd_alloc",
                    format!("returned a negative pointer: {ptr}"),
                ));
            }
            let ptr = ptr as usize;

            // 写入参数前做边界校验：wasmi 的越界写会 trap，但显式校验把"插件分配器
            // 撒谎"变成结构化错误而不是崩溃现场
            let memory_size = memory.data_size(&plugin.store);
            if args.len() > MAX_RESULT_BYTES
                || ptr
                    .checked_add(args.len())
                    .is_none_or(|end| end > memory_size)
            {
                return Err(HostError::InvalidArgument(
                    "fd_alloc",
                    "returned region is out of the plugin's memory bounds".to_owned(),
                ));
            }
            memory
                .write(&mut plugin.store, ptr, args)
                .map_err(|error| {
                    HostError::Engine(format!("writing args into plugin memory failed: {error}"))
                })?;

            let call_result = invoke.call(&mut plugin.store, (ptr as i32, args_len));
            let packed = match call_result {
                Ok(packed) => packed,
                Err(error) => {
                    let mapped = self.map_error(error, "command");
                    // invoke 失败即崩溃：记入状态，后续调用直接返回缓存错误
                    plugin.state = LifecycleState::Crashed;
                    plugin.last_failure = Some(mapped.clone());
                    return Err(mapped);
                }
            };

            let packed = packed as u64;
            let result_ptr = (packed >> 32) as u32 as usize;
            let result_len = (packed & 0xFFFF_FFFF) as usize;
            if result_len > MAX_RESULT_BYTES
                || result_ptr
                    .checked_add(result_len)
                    .is_none_or(|end| end > memory_size)
            {
                return Err(HostError::InvalidArgument(
                    "fd_invoke",
                    "returned result region is out of the plugin's memory bounds".to_owned(),
                ));
            }
            let mut buffer = vec![0u8; result_len];
            memory
                .read(&plugin.store, result_ptr, &mut buffer)
                .map_err(|error| {
                    HostError::Engine(format!("reading result from plugin memory failed: {error}"))
                })?;
            Ok(String::from_utf8_lossy(&buffer).into_owned())
        })
    }

    fn unload(&self, handle: PluginHandle) -> Result<(), HostError> {
        let mut instances = self.instances.write();
        let Some(mut plugin) = instances.remove(&handle.raw()) else {
            // 卸载不存在的实例视为幂等成功（管理页"清理残留"路径会走到）
            return Ok(());
        };
        on_executor_stack(move || -> Result<(), HostError> {
            Self::reset_fuel(&mut plugin.store)?;
            // 停用一个（哪怕即将崩溃的）插件是成功操作：deactivate 的 trap 不上抛
            if let Some(Err(_)) = plugin
                .instance
                .get_typed_func::<(), i32>(&plugin.store, "fd_deactivate")
                .ok()
                .map(|deactivate| deactivate.call(&mut plugin.store, ()))
            {
                tracing::warn!(plugin_id = handle.raw(), "plugin trapped during deactivate");
            }
            Ok(())
        })
    }

    fn state(&self, handle: PluginHandle) -> Result<LifecycleState, HostError> {
        self.instances
            .read()
            .get(&handle.raw())
            .map(|plugin| plugin.state)
            .ok_or(HostError::InstanceNotFound(handle.raw()))
    }

    fn render_panel(&self, handle: PluginHandle, panel_id: &str) -> Result<String, HostError> {
        let mut instances = self.instances.write();
        let plugin = instances
            .get_mut(&handle.raw())
            .ok_or(HostError::InstanceNotFound(handle.raw()))?;
        if !plugin.state.is_callable() {
            return Err(Self::cached_failure_or(
                plugin.state,
                plugin.last_failure.as_ref(),
            ));
        }
        on_executor_stack(move || -> Result<String, HostError> {
            Self::reset_fuel(&mut plugin.store)?;
            let memory = Self::memory_of(&plugin.instance, &plugin.store)?;

            let alloc = plugin
                .instance
                .get_typed_func::<i32, i32>(&plugin.store, "fd_alloc")
                .map_err(|_| HostError::Engine("plugin does not export fd_alloc".to_owned()))?;
            let render = plugin
                .instance
                .get_typed_func::<(i32, i32), i64>(&plugin.store, "fd_render_panel")
                .map_err(|_| {
                    HostError::Engine("plugin does not export fd_render_panel".to_owned())
                })?;

            let id_bytes = panel_id.as_bytes();
            let id_len = i32::try_from(id_bytes.len())
                .map_err(|_| HostError::InvalidArgument("panel_id", "too large".to_owned()))?;
            let ptr = alloc
                .call(&mut plugin.store, id_len)
                .map_err(|error| self.map_error(error, "command"))?;
            if ptr < 0 {
                return Err(HostError::InvalidArgument(
                    "fd_alloc",
                    format!("returned a negative pointer: {ptr}"),
                ));
            }
            let ptr = ptr as usize;
            let size = memory.data_size(&plugin.store);
            if ptr.checked_add(id_bytes.len()).is_none_or(|end| end > size) {
                return Err(HostError::InvalidArgument(
                    "fd_alloc",
                    "returned region is out of the plugin's memory bounds".to_owned(),
                ));
            }
            memory
                .write(&mut plugin.store, ptr, id_bytes)
                .map_err(|error| HostError::Engine(format!("writing panel id failed: {error}")))?;

            let call_result = render.call(&mut plugin.store, (ptr as i32, id_len));
            let packed = match call_result {
                Ok(packed) => packed,
                Err(error) => {
                    let mapped = self.map_error(error, "command");
                    plugin.state = LifecycleState::Crashed;
                    plugin.last_failure = Some(mapped.clone());
                    return Err(mapped);
                }
            };

            let packed = packed as u64;
            let result_ptr = (packed >> 32) as u32 as usize;
            let result_len = (packed & 0xFFFF_FFFF) as usize;
            if result_len > host::MAX_RESULT_BYTES
                || result_ptr
                    .checked_add(result_len)
                    .is_none_or(|end| end > size)
            {
                return Err(HostError::InvalidArgument(
                    "fd_render_panel",
                    "returned result region is out of the plugin's memory bounds".to_owned(),
                ));
            }
            let mut buffer = vec![0u8; result_len];
            memory
                .read(&plugin.store, result_ptr, &mut buffer)
                .map_err(|error| HostError::Engine(format!("reading panel DSL failed: {error}")))?;
            // 宿主侧校验：前端永远拿到良构 DSL（兜底错误卡片仍是第二道防线）
            crate::panel_dsl::validate_panel_dsl(&buffer)?;
            Ok(String::from_utf8_lossy(&buffer).into_owned())
        })
    }
}

impl WasmiEngine {
    /// 读取插件的日志（最近的 `limit` 条，时间升序）。
    pub fn plugin_logs(
        &self,
        handle: PluginHandle,
        limit: usize,
    ) -> Result<Vec<PluginLogEntry>, HostError> {
        let instances = self.instances.read();
        let plugin = instances
            .get(&handle.raw())
            .ok_or(HostError::InstanceNotFound(handle.raw()))?;
        let logs = plugin.store.data().logs.lock();
        let start = logs.len().saturating_sub(limit);
        Ok(logs.iter().skip(start).cloned().collect())
    }

    /// 读取插件的权限使用计数（成功调用次数，按权限聚合）。
    pub fn permission_usage(
        &self,
        handle: PluginHandle,
    ) -> Result<Vec<(crate::permission::Permission, u64)>, HostError> {
        let instances = self.instances.read();
        let plugin = instances
            .get(&handle.raw())
            .ok_or(HostError::InstanceNotFound(handle.raw()))?;
        let usage = plugin.store.data().usage.lock();
        Ok(usage
            .iter()
            .map(|(permission, count)| (*permission, *count))
            .collect())
    }

    /// 设置实例的生效权限集（加载后、激活前由管理器调用一次；
    /// 运行中的收缩走 [`Self::revoke_permission`]，这里不做扩权）。
    pub fn set_effective_permissions(
        &self,
        handle: PluginHandle,
        permissions: std::collections::BTreeSet<crate::permission::Permission>,
    ) -> Result<(), HostError> {
        let mut instances = self.instances.write();
        let plugin = instances
            .get_mut(&handle.raw())
            .ok_or(HostError::InstanceNotFound(handle.raw()))?;
        plugin.store.data_mut().granted = PermissionSet::from_declared(permissions);
        Ok(())
    }

    /// 运行时撤销一项权限（T6.4：撤销后插件下一次调用即失败）。
    pub fn revoke_permission(
        &self,
        handle: PluginHandle,
        permission: crate::permission::Permission,
    ) -> Result<(), HostError> {
        let mut instances = self.instances.write();
        let plugin = instances
            .get_mut(&handle.raw())
            .ok_or(HostError::InstanceNotFound(handle.raw()))?;
        plugin.store.data_mut().granted.revoke(permission);
        Ok(())
    }

    /// 异步分发事件给已订阅且激活的插件（T6.3）。
    ///
    /// 立即返回订阅者数量；实际回调在分离线程上逐个执行，每个回调受 fuel
    /// 预算约束（死循环回调最终被 fuel 终止），调用方**永不阻塞**——这是
    /// T6.3 验收"事件回调超时不影响宿主流程"的落实方式。
    pub fn dispatch_event(self: Arc<Self>, event: &'static str, payload: &str) -> usize {
        let targets: Vec<u64> = {
            let instances = self.instances.read();
            instances
                .iter()
                .filter(|(_, plugin)| {
                    plugin.state.is_callable()
                        && self
                            .services
                            .event_interest(&plugin.store.data().plugin_id, event)
                })
                .map(|(raw, _)| *raw)
                .collect()
        };
        if targets.is_empty() {
            return 0;
        }
        let count = targets.len();
        let engine = Arc::clone(&self);
        let payload = payload.to_owned();
        std::thread::spawn(move || {
            for raw in targets {
                let handle = PluginHandle::new(raw);
                if let Err(error) = engine.run_event_callback(handle, event, &payload) {
                    tracing::warn!(event, error = %error, "plugin event callback failed");
                }
            }
        });
        count
    }

    /// 在插件实例上执行一次 `fd_on_event`（payload JSON 经 fd_alloc 传入）。
    fn run_event_callback(
        &self,
        handle: PluginHandle,
        _event: &'static str,
        payload: &str,
    ) -> Result<(), HostError> {
        let mut instances = self.instances.write();
        let Some(plugin) = instances.get_mut(&handle.raw()) else {
            return Ok(());
        };
        if !plugin.state.is_callable() {
            return Ok(());
        }
        on_executor_stack(move || -> Result<(), HostError> {
            Self::reset_fuel(&mut plugin.store)?;
            let memory = Self::memory_of(&plugin.instance, &plugin.store)?;
            let alloc = plugin
                .instance
                .get_typed_func::<i32, i32>(&plugin.store, "fd_alloc")
                .map_err(|_| HostError::Engine("plugin does not export fd_alloc".to_owned()))?;
            let callback = plugin
                .instance
                .get_typed_func::<(i32, i32), i32>(&plugin.store, "fd_on_event")
                .map_err(|_| HostError::Engine("plugin does not export fd_on_event".to_owned()))?;

            let bytes = payload.as_bytes();
            let len = i32::try_from(bytes.len())
                .map_err(|_| HostError::InvalidArgument("payload", "too large".to_owned()))?;
            let ptr = alloc
                .call(&mut plugin.store, len)
                .map_err(|error| self.map_error(error, "event"))?;
            if ptr < 0 {
                return Err(HostError::InvalidArgument(
                    "fd_alloc",
                    format!("returned a negative pointer: {ptr}"),
                ));
            }
            let ptr = ptr as usize;
            let size = memory.data_size(&plugin.store);
            if bytes.len() > host::MAX_ARG_BYTES
                || ptr.checked_add(bytes.len()).is_none_or(|end| end > size)
            {
                return Err(HostError::InvalidArgument(
                    "fd_alloc",
                    "returned region is out of the plugin's memory bounds".to_owned(),
                ));
            }
            memory
                .write(&mut plugin.store, ptr, bytes)
                .map_err(|error| HostError::Engine(format!("writing payload failed: {error}")))?;

            callback
                .call(&mut plugin.store, (ptr as i32, len))
                .map(|_| ())
                .map_err(|error| self.map_error(error, "event"))
        })
        .inspect_err(|error| {
            // 回调失败（含 fuel 耗尽）：按崩溃隔离处理，宿主与其他插件不受影响
            let mut instances = self.instances.write();
            if let Some(plugin) = instances.get_mut(&handle.raw()) {
                plugin.state = LifecycleState::Crashed;
                plugin.last_failure = Some(error.clone());
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::manifest::PluginManifest;
    use crate::runtime::RuntimeLimits;

    fn manifest_with_permissions(perms: &[&str]) -> ValidatedManifest {
        manifest_for("com.example.wasmi", perms)
    }

    fn manifest_for(id: &str, perms: &[&str]) -> ValidatedManifest {
        let permissions = perms
            .iter()
            .map(|p| format!(r#""{p}""#))
            .collect::<Vec<_>>()
            .join(",");
        let text = format!(
            r#"{{"id": "{id}", "name": "Wasmi Test", "version": "1.0.0",
                "apiVersion": "0.1", "author": "test", "license": "MIT",
                "description": "test plugin", "main": "plugin.wasm",
                "permissions": [{permissions}]}}"#
        );
        PluginManifest::parse(&text).unwrap()
    }

    fn validated_manifest() -> ValidatedManifest {
        manifest_with_permissions(&["ui:toast"])
    }

    /// 记录调用轨迹的宿主服务（引擎测试用；host.rs 有自己的更全 mock）。
    #[derive(Default)]
    struct RecordingServices {
        calls: std::sync::Mutex<Vec<String>>,
        subscriptions: std::sync::Mutex<std::collections::BTreeMap<String, Vec<String>>>,
    }

    impl RecordingServices {
        fn record(&self, what: String) {
            self.calls.lock().unwrap().push(what);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl crate::host::HostServices for RecordingServices {
        fn repo_info(&self, plugin_id: &str) -> Result<serde_json::Value, HostError> {
            self.record(format!("repo_info:{plugin_id}"));
            Ok(serde_json::json!({
                "path": "/repo", "name": "repo",
                "currentBranch": "main", "isDirty": false
            }))
        }
        fn status(
            &self,
            plugin_id: &str,
            _filter: Option<String>,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("status:{plugin_id}"));
            Ok(serde_json::json!({"entries": []}))
        }
        fn read_file(
            &self,
            plugin_id: &str,
            rel_path: &str,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("read_file:{plugin_id}:{rel_path}"));
            Ok(serde_json::json!({"content": "x"}))
        }
        fn list_dir(
            &self,
            plugin_id: &str,
            rel_path: &str,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("list_dir:{plugin_id}:{rel_path}"));
            Ok(serde_json::json!({"entries": []}))
        }
        fn write_file(
            &self,
            plugin_id: &str,
            rel_path: &str,
            _content: &str,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("write_file:{plugin_id}:{rel_path}"));
            Ok(serde_json::json!({}))
        }
        fn http_get_json(
            &self,
            plugin_id: &str,
            url: &str,
            _headers: serde_json::Value,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("http:{plugin_id}:{url}"));
            Ok(serde_json::json!({"status": 200, "body": {}}))
        }
        fn get_setting(&self, plugin_id: &str, key: &str) -> Result<serde_json::Value, HostError> {
            self.record(format!("get_setting:{plugin_id}:{key}"));
            Ok(serde_json::json!({"value": null}))
        }
        fn set_setting(
            &self,
            plugin_id: &str,
            key: &str,
            _value: serde_json::Value,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("set_setting:{plugin_id}:{key}"));
            Ok(serde_json::json!({}))
        }
        fn git_log(
            &self,
            plugin_id: &str,
            _limit: u32,
            _path: Option<String>,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("git_log:{plugin_id}"));
            Ok(serde_json::json!({"commits": []}))
        }
        fn git_stage(
            &self,
            plugin_id: &str,
            _paths: Vec<String>,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("git_stage:{plugin_id}"));
            Ok(serde_json::json!({}))
        }
        fn git_commit(
            &self,
            plugin_id: &str,
            _message: &str,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("git_commit:{plugin_id}"));
            Ok(serde_json::json!({"commitId": "abc1234"}))
        }
        fn register_command(
            &self,
            plugin_id: &str,
            id: &str,
            _title: &str,
            _keybinding: Option<String>,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("register_command:{plugin_id}:{id}"));
            Ok(serde_json::json!({}))
        }
        fn register_panel(
            &self,
            plugin_id: &str,
            id: &str,
            _title: &str,
            _location: &str,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("register_panel:{plugin_id}:{id}"));
            Ok(serde_json::json!({}))
        }
        fn show_toast(
            &self,
            plugin_id: &str,
            level: &str,
            _message: &str,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("toast:{plugin_id}:{level}"));
            Ok(serde_json::json!({}))
        }
        fn subscribe_events(
            &self,
            plugin_id: &str,
            events: Vec<String>,
        ) -> Result<serde_json::Value, HostError> {
            self.record(format!("subscribe:{plugin_id}:{events:?}"));
            self.subscriptions
                .lock()
                .unwrap()
                .entry(plugin_id.to_owned())
                .or_default()
                .extend(events);
            Ok(serde_json::json!({}))
        }
        fn event_interest(&self, plugin_id: &str, event: &str) -> bool {
            self.subscriptions
                .lock()
                .unwrap()
                .get(plugin_id)
                .is_some_and(|events| events.iter().any(|e| e == event))
        }
    }

    fn test_services() -> (std::sync::Arc<RecordingServices>, SharedServices) {
        let services: std::sync::Arc<RecordingServices> =
            std::sync::Arc::new(RecordingServices::default());
        // 通过显式标注触发 Arc<RecordingServices> → Arc<dyn HostServices> 的
        // unsize 强转（Arc::clone 的泛型参数不会自动窄化）
        let shared: SharedServices = services.clone();
        (services, shared)
    }

    // ---------- 手编 wasm 夹具 ----------
    //
    // 测试插件不引入 wasm32 构建目标（CI 与本地都要 rustup target），而是直接
    // 编码最小模块。类型区固定五条：t0=()->i32、t1=(i32,i32)->i64、t2=(i32)->i32、
    // t3=(i32,i32,i32)->i32（fd.host_call）、t4=(i32,i32)->i32（fd.host_result）。

    use super::fixture::*;

    // ---------- 用例 ----------

    #[test]
    fn a_normal_plugin_round_trips_the_full_lifecycle_and_returns_its_result() {
        let (_services, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let handle = engine
            .load(&validated_manifest(), &normal_plugin())
            .unwrap();

        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Loaded);
        engine.activate(handle).unwrap();
        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Active);

        let result = engine.invoke(handle, "greet", "{}").unwrap();
        assert_eq!(result, "hello world");

        engine.unload(handle).unwrap();
        assert_eq!(
            engine.state(handle),
            Err(HostError::InstanceNotFound(handle.raw()))
        );
    }

    #[test]
    fn a_deactivated_plugin_can_be_reactivated() {
        let (_services, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let handle = engine
            .load(&validated_manifest(), &normal_plugin())
            .unwrap();
        engine.activate(handle).unwrap();
        // 直接走内部状态模拟 Deactivated（invoke 需要可调用状态，不能先 unload）
        engine
            .instances
            .write()
            .get_mut(&handle.raw())
            .unwrap()
            .state = LifecycleState::Deactivated;
        engine.activate(handle).unwrap();
        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Active);
    }

    #[test]
    fn an_infinite_loop_plugin_is_stopped_by_the_fuel_budget_and_quarantined() {
        let (_services, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let good = engine
            .load(&validated_manifest(), &normal_plugin())
            .unwrap();
        let bomb = engine.load(&validated_manifest(), &bomb_plugin()).unwrap();

        // 激活死循环插件：不挂起宿主，fuel 耗尽返回结构化超时
        // （执行在引擎内部的大栈执行线程上进行，测试线程不会被拖垮）
        let error = engine.activate(bomb).unwrap_err();
        assert!(
            matches!(error, HostError::Timeout { .. }),
            "实际错误: {error}"
        );
        assert_eq!(engine.state(bomb).unwrap(), LifecycleState::Crashed);

        // 隔离：崩溃插件后续调用返回缓存错误；正常插件不受任何影响
        assert!(engine.invoke(bomb, "x", "{}").is_err());
        engine.activate(good).unwrap();
        assert_eq!(engine.invoke(good, "greet", "{}").unwrap(), "hello world");

        // 卸载崩溃插件是成功操作，宿主继续可用
        engine.unload(bomb).unwrap();
        assert_eq!(
            engine.state(bomb),
            Err(HostError::InstanceNotFound(bomb.raw()))
        );
        assert!(engine.invoke(good, "greet", "{}").is_ok());
    }

    #[test]
    fn a_trapping_plugin_becomes_a_structured_error_and_never_panics_the_host() {
        let (_services, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let handle = engine.load(&validated_manifest(), &trap_plugin()).unwrap();
        engine.activate(handle).unwrap();

        let error = engine.invoke(handle, "boom", "{}").unwrap_err();
        assert!(matches!(error, HostError::Trap(_)), "实际错误: {error}");
        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Crashed);
        // 再调用：缓存的同一个结构化错误，不是 panic
        assert_eq!(engine.invoke(handle, "boom", "{}"), Err(error));
    }

    #[test]
    fn invalid_wasm_bytes_fail_to_load_with_a_structured_engine_error() {
        let (_services, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let error = engine
            .load(&validated_manifest(), b"not wasm at all")
            .unwrap_err();
        assert!(matches!(error, HostError::Engine(_)), "实际错误: {error}");
    }

    #[test]
    fn a_memory_grow_beyond_the_store_limit_traps_and_quarantines_the_plugin() {
        // 8MB 上限的引擎 + 试图 grow 4GB 的插件：上限让 grow 按规范返回 -1，
        // 插件据此 trap；宿主以结构化错误收场且继续可用
        let small = RuntimeLimits {
            max_plugin_memory_bytes: 8 * 1024 * 1024,
            ..RuntimeLimits::default()
        };
        let (_services, services) = test_services();
        let engine = WasmiEngine::new(small, services);
        let grow_bomb = build_module(
            &[],
            &[
                ("fd_activate", TYPE_UNIT_I32, &grow_bomb_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
            ],
            &[],
        );
        let handle = engine.load(&validated_manifest(), &grow_bomb).unwrap();
        let error = engine.activate(handle).unwrap_err();
        assert!(matches!(error, HostError::Trap(_)), "实际错误: {error}");
        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Crashed);

        // 宿主无恙：还能正常加载并运行其它插件
        let good = engine
            .load(&validated_manifest(), &normal_plugin())
            .unwrap();
        engine.activate(good).unwrap();
        assert_eq!(engine.invoke(good, "greet", "{}").unwrap(), "hello world");
    }

    #[test]
    fn plugins_without_optional_lifecycle_exports_activate_immediately() {
        // 只有 fd_alloc/fd_invoke 的插件：activate 无导出 → 直接 Active
        let module = build_module(
            &[],
            &[
                ("fd_alloc", TYPE_LEN_PTR, &alloc_body()),
                ("fd_invoke", TYPE_ARGS_I64, &invoke_body_returning_hello()),
            ],
            &[(1024, b"hello world")],
        );
        let (_services, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let handle = engine.load(&validated_manifest(), &module).unwrap();
        engine.activate(handle).unwrap();
        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Active);
        assert_eq!(engine.invoke(handle, "greet", "{}").unwrap(), "hello world");
    }

    #[test]
    fn unloading_a_missing_handle_is_an_idempotent_success() {
        let (_services, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let handle = PluginHandle::new(999);
        assert!(engine.unload(handle).is_ok());
        assert_eq!(
            engine.state(handle),
            Err(HostError::InstanceNotFound(handle.raw()))
        );
    }

    // ---------- T6.2：宿主函数端到端（真实 wasmi 导入路径） ----------

    #[test]
    fn a_plugin_reaches_host_services_through_the_real_wasmi_imports() {
        let (recorder, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let module = host_call_plugin(crate::host::HostOp::GetRepoInfo, "{}");
        let manifest = manifest_with_permissions(&["git:read"]);
        let handle = engine.load(&manifest, &module).unwrap();
        engine.activate(handle).unwrap();

        // fd_invoke → fd.host_call → 派发器 → 服务 → staging → fd.host_result
        // → 插件内存 → fd_invoke 返回区 → 引擎读回
        let result = engine.invoke(handle, "repo", "{}").unwrap();
        assert!(
            result.contains("\"currentBranch\":\"main\""),
            "实际结果: {result}"
        );
        assert_eq!(
            recorder.calls(),
            vec!["repo_info:com.example.wasmi".to_owned()],
            "服务应恰好收到一次带 plugin_id 的调用"
        );
    }

    #[test]
    fn a_plugin_calling_an_op_outside_its_manifest_permissions_is_denied_end_to_end() {
        let (recorder, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        // 清单未声明 git:read，但插件仍尝试 get_repo_info
        let module = host_call_plugin(crate::host::HostOp::GetRepoInfo, "{}");
        let manifest = manifest_with_permissions(&["ui:toast"]);
        let handle = engine.load(&manifest, &module).unwrap();
        engine.activate(handle).unwrap();

        let result = engine.invoke(handle, "repo", "{}").unwrap();
        assert!(result.contains("\"error\""), "实际结果: {result}");
        assert!(
            result.contains("-2"),
            "应携带 PERMISSION_DENIED 码: {result}"
        );
        assert!(recorder.calls().is_empty(), "越权调用绝不能触达服务层");
        // 宿主不崩：引擎还能继续正常工作
        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Active);
    }

    #[test]
    fn a_host_call_with_malformed_args_returns_the_error_payload_via_staging() {
        let (_recorder, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        // limit=0 违反 1..=1000 的范围校验
        let module = host_call_plugin(crate::host::HostOp::GetGitLog, r#"{"limit":0}"#);
        let manifest = manifest_with_permissions(&["git:read"]);
        let handle = engine.load(&manifest, &module).unwrap();
        engine.activate(handle).unwrap();

        let result = engine.invoke(handle, "log", "{}").unwrap();
        assert!(result.contains("\"error\""), "实际结果: {result}");
        assert!(
            result.contains("-3"),
            "应携带 INVALID_ARGUMENT 码: {result}"
        );
    }

    #[test]
    fn a_plugin_reaches_the_log_import_and_the_host_reads_its_memory() {
        let (_recorder, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let module = log_plugin();
        let manifest = manifest_with_permissions(&[]);
        let handle = engine.load(&manifest, &module).unwrap();
        engine.activate(handle).unwrap();
        let result = engine.invoke(handle, "log", "{}").unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn single_threaded_manual_repro_of_the_host_import_path() {
        let (_recorder, services) = test_services();
        let engine_wasmi = WasmiEngine::with_default_limits(std::sync::Arc::clone(&services));
        let module_bytes = log_plugin();
        let manifest = manifest_with_permissions(&[]);

        let engine = &engine_wasmi.engine;
        let limits = engine_wasmi.limits;
        let module = wasmi::Module::new(engine, &module_bytes).unwrap();
        let mut store = wasmi::Store::new(
            engine,
            PluginData {
                limits: wasmi::StoreLimitsBuilder::new()
                    .memory_size(limits.max_plugin_memory_bytes as usize)
                    .build(),
                fuel_budget: limits.fuel_budget,
                plugin_id: manifest.id.clone(),
                granted: PermissionSet::from_declared(manifest.permissions.iter().copied()),
                services,
                staging: Vec::new(),
                logs: Mutex::new(VecDeque::new()),
                usage: Mutex::new(BTreeMap::new()),
            },
        );
        store.limiter(|data: &mut PluginData| &mut data.limits);
        let mut linker = <wasmi::Linker<PluginData>>::new(engine);
        // BISECT A：探针式内联简化闭包（无 import_read_bytes / 无 tracing）
        linker
            .func_wrap(
                "fd",
                "log",
                |caller: wasmi::Caller<'_, PluginData>, _level: i32, ptr: i32, len: i32| {
                    let memory = caller
                        .get_export("memory")
                        .and_then(|e| e.into_memory())
                        .unwrap();
                    let size = memory.data_size(&caller);
                    assert!(ptr >= 0 && len >= 0);
                    let start = ptr as usize;
                    let len = len as usize;
                    assert!(start + len <= size);
                    let mut buf = vec![0u8; len];
                    memory.read(&caller, start, &mut buf).unwrap();
                    assert_eq!(String::from_utf8_lossy(&buf), "hello");
                },
            )
            .unwrap();
        let instance = linker.instantiate_and_start(&mut store, &module).unwrap();
        store.set_fuel(limits.fuel_budget).unwrap();

        let test = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "fd_invoke")
            .unwrap();
        // fd_invoke 自身的 wasm 参数 (ptr, len)：宿主操作忽略它
        let packed = test.call(&mut store, (8192, 2)).unwrap();
        let packed = packed as u64;
        let ptr = (packed >> 32) as u32 as usize;
        let len = (packed & 0xFFFF_FFFF) as usize;
        let memory = instance
            .get_export(&store, "memory")
            .and_then(|e| e.into_memory())
            .unwrap();
        let mut buf = vec![0u8; len];
        memory.read(&store, ptr, &mut buf).unwrap();
        assert_eq!(String::from_utf8_lossy(&buf), "hello");
    }

    // ---------- T6.3：面板 DSL 与事件分发 ----------

    const SAMPLE_DSL: &str = r#"[
        {"type": "heading", "text": "Stats"},
        {"type": "table", "columns": ["author", "commits"], "rows": [["a", "12"]]},
        {"type": "button", "command": "com.example.wasmi.refresh", "label": "Refresh"}
    ]"#;

    #[test]
    fn render_panel_returns_validated_dsl_from_the_plugin() {
        let (_recorder, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let module = panel_plugin(SAMPLE_DSL);
        let manifest = manifest_with_permissions(&["ui:panel"]);
        let handle = engine.load(&manifest, &module).unwrap();
        engine.activate(handle).unwrap();

        let dsl = engine.render_panel(handle, "stats").unwrap();
        assert_eq!(dsl, SAMPLE_DSL, "校验不得改动 DSL 内容");
    }

    #[test]
    fn render_panel_rejects_malformed_dsl_as_a_structured_error() {
        let (_recorder, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let module = panel_plugin(r#"[{"type": "iframe", "src": "https://evil"}]"#);
        let manifest = manifest_with_permissions(&["ui:panel"]);
        let handle = engine.load(&manifest, &module).unwrap();
        engine.activate(handle).unwrap();

        let error = engine.render_panel(handle, "stats").unwrap_err();
        assert!(matches!(error, HostError::InvalidArgument(_, _)), "{error}");
        assert!(error.to_string().contains("iframe"), "{error}");
        // 插件没有 trap：DSL 校验失败是数据问题，不是崩溃
        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Active);
    }

    #[test]
    fn render_panel_traps_quarantine_the_plugin() {
        let (_recorder, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let module = trap_plugin(); // fd_invoke trap，但 render_panel 缺失 → Engine 错误
        let manifest = manifest_with_permissions(&["ui:panel"]);
        let handle = engine.load(&manifest, &module).unwrap();
        engine.activate(handle).unwrap();

        let error = engine.render_panel(handle, "stats").unwrap_err();
        assert!(matches!(error, HostError::Engine(_)), "{error}");
    }

    #[test]
    fn dispatch_event_reaches_subscribed_plugins_without_blocking_the_caller() {
        let (recorder, services) = test_services();
        let engine = Arc::new(WasmiEngine::with_default_limits(Arc::clone(&services)));

        let module = event_plugin();
        let manifest = manifest_with_permissions(&["git:read", "ui:toast"]);
        let handle = engine.load(&manifest, &module).unwrap();
        engine.activate(handle).unwrap();
        // 组合根登记订阅（此处直接驱动 mock 模拟 T6.4 的授权流程产物）
        services
            .subscribe_events("com.example.wasmi", vec!["repo_changed".to_owned()])
            .unwrap();

        let payload = r#"{"event":"repo_changed","level":"info","message":"from event"}"#;
        let started = std::time::Instant::now();
        let subscribers = Arc::clone(&engine).dispatch_event("repo_changed", payload);
        let elapsed = started.elapsed();
        assert_eq!(subscribers, 1);
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "分发必须立即返回，实际 {elapsed:?}"
        );

        // 回调在后台执行：轮询等待 toast 到达服务层
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if recorder
                .calls()
                .iter()
                .any(|call| call.starts_with("toast:com.example.wasmi:info"))
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "回调 3 秒内未执行：{:?}",
                recorder.calls()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn an_infinite_loop_event_callback_never_blocks_the_host() {
        let (_recorder, services) = test_services();
        let engine = Arc::new(WasmiEngine::with_default_limits(Arc::clone(&services)));

        let slow = slow_event_plugin();
        let slow_manifest = manifest_with_permissions(&["git:read"]);
        let slow_handle = engine.load(&slow_manifest, &slow).unwrap();
        engine.activate(slow_handle).unwrap();
        services
            .subscribe_events("com.example.wasmi", vec!["commit_created".to_owned()])
            .unwrap();

        // 正常插件作为"宿主仍可用"的对照组（不同插件 id：id 在真实安装中唯一）
        let good = normal_plugin();
        let good_handle = engine
            .load(&manifest_for("com.example.good", &[]), &good)
            .unwrap();
        engine.activate(good_handle).unwrap();

        let started = std::time::Instant::now();
        let subscribers = Arc::clone(&engine).dispatch_event("commit_created", "{}");
        assert_eq!(subscribers, 1);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "死循环回调不得阻塞分发调用方"
        );

        // 宿主立刻可用：好插件的命令照常执行（不等待崩溃回调结束）
        let result = engine.invoke(good_handle, "greet", "{}").unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn dispatch_event_with_no_subscribers_is_a_no_op() {
        let (_recorder, services) = test_services();
        let engine = Arc::new(WasmiEngine::with_default_limits(services));
        assert_eq!(Arc::clone(&engine).dispatch_event("repo_opened", "{}"), 0);
    }

    // ---------- T6.4：插件日志、权限用量、运行时撤权 ----------

    #[test]
    fn plugin_logs_capture_fd_log_output_in_order() {
        let (_recorder, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let module = log_plugin(); // fd_invoke 期间调用 fd.log("hello", level=0)
        let manifest = manifest_with_permissions(&[]);
        let handle = engine.load(&manifest, &module).unwrap();
        engine.activate(handle).unwrap();
        engine.invoke(handle, "log", "{}").unwrap();

        let logs = engine.plugin_logs(handle, 100).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message, "hello");
        assert_eq!(logs[0].level, 0);
        assert!(logs[0].time_ms > 0);
        // 卸载后实例消失，日志不可取
        engine.unload(handle).unwrap();
        assert!(engine.plugin_logs(handle, 100).is_err());
    }

    #[test]
    fn permission_usage_counts_successful_host_calls_per_permission() {
        let (_recorder, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let module = host_call_plugin(crate::host::HostOp::GetRepoInfo, "{}");
        let manifest = manifest_with_permissions(&["git:read"]);
        let handle = engine.load(&manifest, &module).unwrap();
        engine.activate(handle).unwrap();

        engine.invoke(handle, "repo", "{}").unwrap();
        engine.invoke(handle, "repo", "{}").unwrap();

        let usage = engine.permission_usage(handle).unwrap();
        assert_eq!(usage, vec![(crate::permission::Permission::GitRead, 2)]);
    }

    #[test]
    fn revoking_a_permission_denies_the_next_host_call_on_a_running_instance() {
        let (_recorder, services) = test_services();
        let engine = WasmiEngine::with_default_limits(services);
        let module = host_call_plugin(crate::host::HostOp::GetRepoInfo, "{}");
        let manifest = manifest_with_permissions(&["git:read"]);
        let handle = engine.load(&manifest, &module).unwrap();
        engine.activate(handle).unwrap();
        assert!(engine.invoke(handle, "repo", "{}").is_ok());

        // T6.4：撤销后下一次调用立即失败，实例不崩（权限拒绝是数据不是故障）
        engine
            .revoke_permission(handle, crate::permission::Permission::GitRead)
            .unwrap();
        let denied = engine.invoke(handle, "repo", "{}").unwrap();
        assert!(denied.contains("-2"), "应携带 PERMISSION_DENIED: {denied}");
        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Active);
    }
}

/// 手编 wasm 测试夹具（engine 与 manager 的测试共用；CI 无需 wasm32 目标）。
#[cfg(test)]
pub(crate) mod fixture {
    pub(crate) const TYPE_UNIT_I32: u32 = 0;
    pub(crate) const TYPE_ARGS_I64: u32 = 1;
    pub(crate) const TYPE_LEN_PTR: u32 = 2;
    pub(crate) const TYPE_HOST_CALL: u32 = 3;
    pub(crate) const TYPE_HOST_RESULT: u32 = 4;
    pub(crate) const TYPE_LOG: u32 = 5;

    pub(crate) fn leb_u64(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    pub(crate) fn sleb_i64(mut value: i64, out: &mut Vec<u8>) {
        loop {
            let byte = (value as u8) & 0x7f;
            value >>= 7;
            let sign_bit_set = byte & 0x40 != 0;
            if (value == 0 && !sign_bit_set) || (value == -1 && sign_bit_set) {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }

    pub(crate) fn section(id: u8, payload: &[u8], out: &mut Vec<u8>) {
        out.push(id);
        leb_u64(payload.len() as u64, out);
        out.extend_from_slice(payload);
    }

    /// 构建一个测试插件模块。
    ///
    /// `imports` 为 `fd` 模块下的导入（按序占据函数索引 0..n）；
    /// `exports` 为本地定义的导出（索引 = imports.len() + 序号）；
    /// `data` 为 (偏移, 字节) 数据段列表。
    pub(crate) fn build_module(
        imports: &[(&'static str, u32)],
        exports: &[(&'static str, u32, &[u8])],
        data: &[(u32, &[u8])],
    ) -> Vec<u8> {
        let mut module = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];

        // type section：六条固定类型（wasm 段顺序固定：type 必须在 import 之前）
        let mut types = vec![0x06];
        types.extend_from_slice(&[
            0x60, 0x00, 0x01, 0x7f, // t0: () -> i32
            0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e, // t1: (i32,i32) -> i64
            0x60, 0x01, 0x7f, 0x01, 0x7f, // t2: (i32) -> i32
            0x60, 0x03, 0x7f, 0x7f, 0x7f, 0x01, 0x7f, // t3: (i32,i32,i32) -> i32
            0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7f, // t4: (i32,i32) -> i32
            0x60, 0x03, 0x7f, 0x7f, 0x7f, 0x00, // t5: (i32,i32,i32) -> ()
        ]);
        section(0x01, &types, &mut module);

        // import section：全部来自 "fd" 模块
        if !imports.is_empty() {
            let mut import_section = vec![imports.len() as u8];
            for (name, type_idx) in imports {
                leb_u64(2, &mut import_section); // "fd".len()
                import_section.extend_from_slice(b"fd");
                leb_u64(name.len() as u64, &mut import_section);
                import_section.extend_from_slice(name.as_bytes());
                import_section.push(0x00); // func
                leb_u64(u64::from(*type_idx), &mut import_section);
            }
            section(0x02, &import_section, &mut module);
        }

        // function section：导出顺序即函数索引
        let mut funcs = vec![exports.len() as u8];
        for (_, type_idx, _) in exports {
            leb_u64(u64::from(*type_idx), &mut funcs);
        }
        section(0x03, &funcs, &mut module);

        // memory section：1 页，无上限（上限由宿主 limiter 管）
        section(0x05, &[0x01, 0x00, 0x01], &mut module);

        // export section：函数（索引要加上导入数）+ memory
        let mut export_section = vec![exports.len() as u8 + 1];
        for (index, (name, _, _)) in exports.iter().enumerate() {
            leb_u64(name.len() as u64, &mut export_section);
            export_section.extend_from_slice(name.as_bytes());
            export_section.push(0x00);
            leb_u64((imports.len() + index) as u64, &mut export_section);
        }
        export_section.extend_from_slice(b"\x06memory\x02\x00");
        section(0x07, &export_section, &mut module);

        // code section：每个函数体 = local 计数 0x00 + body（body 自带 end）
        let mut code = vec![exports.len() as u8];
        for (_, _, body) in exports {
            let mut entry = vec![0x00];
            entry.extend_from_slice(body);
            leb_u64(entry.len() as u64, &mut code);
            code.extend_from_slice(&entry);
        }
        section(0x0a, &code, &mut module);

        // data section：每个段 = memidx 0 + offset(i32.const) + 字节
        if !data.is_empty() {
            let mut payload = vec![data.len() as u8];
            for (offset, bytes) in data {
                payload.push(0x00);
                payload.push(0x41); // i32.const
                sleb_i64(i64::from(*offset), &mut payload);
                payload.push(0x0b);
                leb_u64(bytes.len() as u64, &mut payload);
                payload.extend_from_slice(bytes);
            }
            section(0x0b, &payload, &mut module);
        }
        module
    }

    pub(crate) fn ok_body() -> Vec<u8> {
        vec![0x41, 0x00, 0x0b] // i32.const 0
    }

    pub(crate) fn alloc_body() -> Vec<u8> {
        // i32.const 2048（fd_alloc 永远返回同一块区域——测试夹具够用）
        vec![0x41, 0x80, 0x10, 0x0b]
    }

    pub(crate) fn invoke_body_returning_hello() -> Vec<u8> {
        // 返回 (ptr<<32)|len，ptr=1024、len=11（数据段内容 "hello world"）
        let mut body = vec![0x42];
        sleb_i64((1024i64 << 32) | 11, &mut body);
        body.push(0x0b);
        body
    }

    /// 纯自旋死循环：loop { br 0 } —— fuel 耗尽才会停。
    /// 注意：块结束会清除"不可达"标记（标准校验语义），()->i32 函数必须在
    /// 循环后补一个 i32 值才能通过校验（运行时永远到不了那里）。
    pub(crate) fn infinite_loop_body() -> Vec<u8> {
        vec![0x03, 0x40, 0x0c, 0x00, 0x0b, 0x41, 0x00, 0x0b]
    }

    /// 内存炸弹体：尝试一次性把线性内存 grow 65536 页（4GB）；store 上限会让
    /// grow 失败并按规范返回 -1，插件据此主动 unreachable——验证上限生效。
    pub(crate) fn grow_bomb_body() -> Vec<u8> {
        vec![
            0x41, 0x80, 0x80, 0x04, // i32.const 65536（LEB128）
            0x40, 0x00, // memory.grow (memidx 0)
            0x41, 0x7f, // i32.const -1
            0x46, // i32.eq
            0x04, 0x40, // if (empty)
            0x00, //   unreachable
            0x0b, // end if
            0x41, 0x00, // i32.const 0
            0x0b, // end func
        ]
    }

    pub(crate) fn trap_body() -> Vec<u8> {
        vec![0x00, 0x0b] // unreachable
    }

    pub(crate) fn normal_plugin() -> Vec<u8> {
        build_module(
            &[],
            &[
                ("fd_activate", TYPE_UNIT_I32, &ok_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
                ("fd_alloc", TYPE_LEN_PTR, &alloc_body()),
                ("fd_invoke", TYPE_ARGS_I64, &invoke_body_returning_hello()),
            ],
            &[(1024, b"hello world")],
        )
    }

    /// 调用一次 `fd.host_call` 并把 staging 结果带回 fd_invoke 返回区的插件体。
    ///
    /// 约定内存布局：参数 JSON 位于 1024，输出缓冲位于 2048（容量 1024）。
    pub(crate) fn host_call_body(op: i32, arg_len: i32) -> Vec<u8> {
        const ARG_PTR: i32 = 1024;
        const OUT_PTR: i32 = 2048;
        const OUT_CAP: i32 = 1024;
        let mut body = vec![];
        for value in [op, ARG_PTR, arg_len] {
            body.push(0x41);
            sleb_i64(i64::from(value), &mut body);
        }
        body.extend_from_slice(&[0x10, 0x00]); // call 0 = fd.host_call
        body.push(0x1a); // drop 返回码（结果看 staging）
        for value in [OUT_PTR, OUT_CAP] {
            body.push(0x41);
            sleb_i64(i64::from(value), &mut body);
        }
        body.extend_from_slice(&[0x10, 0x01]); // call 1 = fd.host_result
        body.push(0x1a); // drop 写入字节数
        body.push(0x42); // i64.const (OUT_PTR << 32) | OUT_CAP
        sleb_i64(((OUT_PTR as i64) << 32) | i64::from(OUT_CAP), &mut body);
        body.push(0x0b);
        body
    }

    /// 只调 `fd.log` 的最小插件：二分定位导入崩溃点（纯内存读路径）。
    /// fd_invoke 返回数据段内容，便于断言。
    pub(crate) fn log_plugin() -> Vec<u8> {
        let mut body = vec![];
        for value in [0i32, 1024, 5] {
            body.push(0x41);
            sleb_i64(i64::from(value), &mut body);
        }
        body.extend_from_slice(&[0x10, 0x00]); // call 0 = fd.log
        body.push(0x42); // i64.const (1024 << 32) | 5
        sleb_i64((1024i64 << 32) | 5, &mut body);
        body.push(0x0b);
        build_module(
            &[("log", TYPE_LOG)],
            &[
                ("fd_activate", TYPE_UNIT_I32, &ok_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
                ("fd_alloc", TYPE_LEN_PTR, &alloc_body_at(8192)),
                ("fd_invoke", TYPE_ARGS_I64, &body),
            ],
            &[(1024, b"hello")],
        )
    }

    pub(crate) fn alloc_body_at(ptr: i32) -> Vec<u8> {
        let mut body = vec![0x41];
        sleb_i64(i64::from(ptr), &mut body);
        body.push(0x0b);
        body
    }

    /// 构建一个"渲染指定面板"的测试插件：fd_render_panel 忽略面板 id，
    /// 返回数据段 1024 处的 DSL JSON（打包 ptr<<32|len）。
    pub(crate) fn panel_plugin(dsl_json: &str) -> Vec<u8> {
        let mut body = vec![0x42];
        sleb_i64((1024i64 << 32) | dsl_json.len() as i64, &mut body);
        body.push(0x0b);
        build_module(
            &[],
            &[
                ("fd_activate", TYPE_UNIT_I32, &ok_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
                ("fd_alloc", TYPE_LEN_PTR, &alloc_body_at(8192)),
                ("fd_render_panel", TYPE_ARGS_I64, &body),
            ],
            &[(1024, dsl_json.as_bytes())],
        )
    }

    /// 构建一个"收到事件后转发为 toast"的测试插件：
    /// fd_on_event(payload_ptr, payload_len) → host_call(SHOW_TOAST, payload)。
    pub(crate) fn event_plugin() -> Vec<u8> {
        let mut body = vec![0x41];
        sleb_i64(i64::from(crate::host::HostOp::ShowToast.id()), &mut body);
        body.extend_from_slice(&[0x20, 0x00, 0x20, 0x01]); // local.get 0, local.get 1
        body.extend_from_slice(&[0x10, 0x00]); // call 0 = fd.host_call
        body.push(0x1a); // drop 返回码
        body.extend_from_slice(&[0x41, 0x00]); // i32.const 0
        body.push(0x0b);
        build_module(
            &[("host_call", TYPE_HOST_CALL)],
            &[
                ("fd_activate", TYPE_UNIT_I32, &ok_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
                ("fd_alloc", TYPE_LEN_PTR, &alloc_body_at(8192)),
                ("fd_on_event", TYPE_HOST_RESULT, &body),
            ],
            &[],
        )
    }

    /// fd_on_event 死循环的插件：验证事件分发不阻塞宿主（fuel 终止回调）。
    pub(crate) fn slow_event_plugin() -> Vec<u8> {
        let mut body = vec![0x03, 0x40, 0x0c, 0x00, 0x0b]; // loop { br 0 }
        body.extend_from_slice(&[0x41, 0x00]); // i32.const 0（块结束清除不可达）
        body.push(0x0b);
        build_module(
            &[],
            &[
                ("fd_activate", TYPE_UNIT_I32, &ok_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
                ("fd_alloc", TYPE_LEN_PTR, &alloc_body_at(8192)),
                ("fd_on_event", TYPE_HOST_RESULT, &body),
            ],
            &[],
        )
    }

    /// 构建一个"调用指定宿主操作"的测试插件。
    ///
    /// 内存布局：1024 = 宿主操作参数 JSON（数据段）；2048 = staging 输出缓冲；
    /// 8192 = fd_alloc 的 invoke 参数区（引擎 invoke 会把命令参数写到这里，
    /// 本夹具不读它）。三者互不重叠。
    pub(crate) fn host_call_plugin(op: crate::host::HostOp, arg_json: &str) -> Vec<u8> {
        build_module(
            &[
                ("host_call", TYPE_HOST_CALL),
                ("host_result", TYPE_HOST_RESULT),
            ],
            &[
                ("fd_activate", TYPE_UNIT_I32, &ok_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
                ("fd_alloc", TYPE_LEN_PTR, &alloc_body_at(8192)),
                (
                    "fd_invoke",
                    TYPE_ARGS_I64,
                    &host_call_body(op.id(), arg_json.len() as i32),
                ),
            ],
            &[(1024, arg_json.as_bytes())],
        )
    }

    pub(crate) fn bomb_plugin() -> Vec<u8> {
        build_module(
            &[],
            &[
                ("fd_activate", TYPE_UNIT_I32, &infinite_loop_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
            ],
            &[],
        )
    }

    pub(crate) fn trap_plugin() -> Vec<u8> {
        build_module(
            &[],
            &[
                ("fd_activate", TYPE_UNIT_I32, &ok_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
                ("fd_alloc", TYPE_LEN_PTR, &alloc_body()),
                ("fd_invoke", TYPE_ARGS_I64, &trap_body()),
            ],
            &[],
        )
    }
}
