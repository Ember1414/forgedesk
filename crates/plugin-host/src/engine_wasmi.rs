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

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::manifest::ValidatedManifest;
use crate::runtime::{HostError, LifecycleState, PluginEngine, PluginHandle, RuntimeLimits};

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

/// 每个 Store 挂的资源上限与 fuel 预算（limiter 闭包需要从 data 取）。
#[derive(Debug)]
struct PluginData {
    limits: wasmi::StoreLimits,
    fuel_budget: u64,
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
    instances: parking_lot::RwLock<BTreeMap<u64, PluginInstance>>,
    next_id: AtomicU64,
}

impl WasmiEngine {
    /// 以给定资源上限构建引擎。
    pub fn new(limits: RuntimeLimits) -> Self {
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        Self {
            engine: wasmi::Engine::new(&config),
            limits,
            instances: parking_lot::RwLock::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// 以规格默认上限构建（64MB / 5s / 30s）。
    pub fn with_default_limits() -> Self {
        Self::new(RuntimeLimits::default())
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
    fn load(&self, _manifest: &ValidatedManifest, wasm: &[u8]) -> Result<PluginHandle, HostError> {
        // 翻译 + 实例化是原生栈需求最深的一段，放到执行线程上
        let engine = &self.engine;
        let limits = self.limits;
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
                },
            );
            store.limiter(|data: &mut PluginData| &mut data.limits);
            let linker = <wasmi::Linker<PluginData>>::new(engine);
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
        _command: &str,
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

            let args = arg_json.as_bytes();
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::manifest::PluginManifest;
    use crate::runtime::RuntimeLimits;

    fn validated_manifest() -> ValidatedManifest {
        PluginManifest::parse(
            r#"{
                "id": "com.example.wasmi",
                "name": "Wasmi Test",
                "version": "1.0.0",
                "apiVersion": "0.1",
                "author": "test",
                "license": "MIT",
                "description": "test plugin",
                "main": "plugin.wasm",
                "permissions": ["ui:toast"]
            }"#,
        )
        .unwrap()
    }

    // ---------- 手编 wasm 夹具 ----------
    //
    // 测试插件不引入 wasm32 构建目标（CI 与本地都要 rustup target），而是直接
    // 编码最小模块。类型区固定三条：t0=()->i32、t1=(i32,i32)->i64、t2=(i32)->i32。

    const TYPE_UNIT_I32: u32 = 0;
    const TYPE_ARGS_I64: u32 = 1;
    const TYPE_LEN_PTR: u32 = 2;

    fn leb_u64(mut value: u64, out: &mut Vec<u8>) {
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

    fn sleb_i64(mut value: i64, out: &mut Vec<u8>) {
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

    fn section(id: u8, payload: &[u8], out: &mut Vec<u8>) {
        out.push(id);
        leb_u64(payload.len() as u64, out);
        out.extend_from_slice(payload);
    }

    /// 构建一个测试插件模块：`exports` 按序为（导出名、类型、函数体）；
    /// `data` 作为线性内存 1024 偏移处的数据段（"hello world" 用）。
    fn build_module(exports: &[(&'static str, u32, &[u8])], data: Option<&[u8]>) -> Vec<u8> {
        let mut module = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];

        // type section：三条固定类型
        let mut types = vec![0x03];
        types.extend_from_slice(&[
            0x60, 0x00, 0x01, 0x7f, // t0: () -> i32
            0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7e, // t1: (i32,i32) -> i64
            0x60, 0x01, 0x7f, 0x01, 0x7f, // t2: (i32) -> i32
        ]);
        section(0x01, &types, &mut module);

        // function section：导出顺序即函数索引
        let mut funcs = vec![exports.len() as u8];
        for (_, type_idx, _) in exports {
            leb_u64(u64::from(*type_idx), &mut funcs);
        }
        section(0x03, &funcs, &mut module);

        // memory section：1 页，无上限（上限由宿主 limiter 管）
        section(0x05, &[0x01, 0x00, 0x01], &mut module);

        // export section：函数 + memory
        let mut export_section = vec![exports.len() as u8 + 1];
        for (index, (name, _, _)) in exports.iter().enumerate() {
            leb_u64(name.len() as u64, &mut export_section);
            export_section.extend_from_slice(name.as_bytes());
            export_section.push(0x00);
            leb_u64(index as u64, &mut export_section);
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

        // data section：offset i32.const 1024
        if let Some(bytes) = data {
            let mut payload = vec![0x01, 0x00, 0x41, 0x80, 0x08, 0x0b];
            leb_u64(bytes.len() as u64, &mut payload);
            payload.extend_from_slice(bytes);
            section(0x0b, &payload, &mut module);
        }
        module
    }

    fn ok_body() -> Vec<u8> {
        vec![0x41, 0x00, 0x0b] // i32.const 0
    }

    fn alloc_body() -> Vec<u8> {
        // i32.const 2048（fd_alloc 永远返回同一块区域——测试夹具够用）
        vec![0x41, 0x80, 0x10, 0x0b]
    }

    fn invoke_body_returning_hello() -> Vec<u8> {
        // 返回 (ptr<<32)|len，ptr=1024、len=11（数据段内容 "hello world"）
        let mut body = vec![0x42];
        sleb_i64((1024i64 << 32) | 11, &mut body);
        body.push(0x0b);
        body
    }

    /// 纯自旋死循环：loop { br 0 } —— fuel 耗尽才会停。
    /// 注意：块结束会清除"不可达"标记（标准校验语义），()->i32 函数必须在
    /// 循环后补一个 i32 值才能通过校验（运行时永远到不了那里）。
    fn infinite_loop_body() -> Vec<u8> {
        vec![0x03, 0x40, 0x0c, 0x00, 0x0b, 0x41, 0x00, 0x0b]
    }

    /// 内存炸弹体：尝试一次性把线性内存 grow 65536 页（4GB）；store 上限会让
    /// grow 失败并按规范返回 -1，插件据此主动 unreachable——验证上限生效。
    fn grow_bomb_body() -> Vec<u8> {
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

    fn trap_body() -> Vec<u8> {
        vec![0x00, 0x0b] // unreachable
    }

    fn normal_plugin() -> Vec<u8> {
        let hello = b"hello world";
        build_module(
            &[
                ("fd_activate", TYPE_UNIT_I32, &ok_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
                ("fd_alloc", TYPE_LEN_PTR, &alloc_body()),
                ("fd_invoke", TYPE_ARGS_I64, &invoke_body_returning_hello()),
            ],
            Some(hello),
        )
    }

    fn bomb_plugin() -> Vec<u8> {
        build_module(
            &[
                ("fd_activate", TYPE_UNIT_I32, &infinite_loop_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
            ],
            None,
        )
    }

    fn trap_plugin() -> Vec<u8> {
        build_module(
            &[
                ("fd_activate", TYPE_UNIT_I32, &ok_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
                ("fd_alloc", TYPE_LEN_PTR, &alloc_body()),
                ("fd_invoke", TYPE_ARGS_I64, &trap_body()),
            ],
            None,
        )
    }

    // ---------- 用例 ----------

    #[test]
    fn a_normal_plugin_round_trips_the_full_lifecycle_and_returns_its_result() {
        let engine = WasmiEngine::with_default_limits();
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
        let engine = WasmiEngine::with_default_limits();
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
        let engine = WasmiEngine::with_default_limits();
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
        let engine = WasmiEngine::with_default_limits();
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
        let engine = WasmiEngine::with_default_limits();
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
        let engine = WasmiEngine::new(small);
        let grow_bomb = build_module(
            &[
                ("fd_activate", TYPE_UNIT_I32, &grow_bomb_body()),
                ("fd_deactivate", TYPE_UNIT_I32, &ok_body()),
            ],
            None,
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
            &[
                ("fd_alloc", TYPE_LEN_PTR, &alloc_body()),
                ("fd_invoke", TYPE_ARGS_I64, &invoke_body_returning_hello()),
            ],
            Some(b"hello world"),
        );
        let engine = WasmiEngine::with_default_limits();
        let handle = engine.load(&validated_manifest(), &module).unwrap();
        engine.activate(handle).unwrap();
        assert_eq!(engine.state(handle).unwrap(), LifecycleState::Active);
        assert_eq!(engine.invoke(handle, "greet", "{}").unwrap(), "hello world");
    }

    #[test]
    fn unloading_a_missing_handle_is_an_idempotent_success() {
        let engine = WasmiEngine::with_default_limits();
        let handle = PluginHandle::new(999);
        assert!(engine.unload(handle).is_ok());
        assert_eq!(
            engine.state(handle),
            Err(HostError::InstanceNotFound(handle.raw()))
        );
    }
}
