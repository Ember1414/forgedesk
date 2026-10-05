//! 示例插件的集成冒烟测试（T6.5）。
//!
//! 加载**入库的真实 wasm 产物**（`plugins/examples/*/plugin.wasm`），
//! 走完整的 load → activate → invoke/render 链路。这层测试的意义：
//!
//! 1. SDK 与宿主 ABI 的真实往返（手编 wasm 夹具覆盖不了 SDK 的代码路径）；
//! 2. CI 里"源码与产物同步"被破坏时（改了插件源码忘了跑 build 脚本），
//!    行为差异在这里最先暴露；
//! 3. 示例插件就是开发文档——它们坏了等于文档坏了。
//!
//! 注意：仓库范围的宿主调用（get_status 等）在测试里没有打开的仓库，
//! 服务返回 NOT_FOUND——插件应当**优雅降级**（返回错误提示而不是 trap），
//! 这本身是被断言的行为。

//! 示例插件冒烟测试：允许 unwrap/panic（测试模块约定，见 CODING_STYLE §2.1）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use forgedesk_plugin_host::engine_wasmi::WasmiEngine;
use forgedesk_plugin_host::host::HostServices;
use forgedesk_plugin_host::manifest::PluginManifest;
use forgedesk_plugin_host::panel_dsl::validate_panel_dsl;
use forgedesk_plugin_host::runtime::{HostError, LifecycleState, PluginEngine, RuntimeLimits};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const EXAMPLES: &[(&str, &str)] = &[
    ("commit-template", "com.example.commit-template"),
    ("repo-stats", "com.example.repo-stats"),
    ("repo-audit", "com.example.repo-audit"),
];

fn example_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/examples")
        .join(name)
}

/// 无网络/无仓库的兜底服务（示例插件在无仓库环境下应优雅降级）。
#[derive(Debug, Default)]
struct BareServices {
    registrations: Mutex<Vec<String>>,
}

impl HostServices for BareServices {
    fn repo_info(&self, _: &str) -> Result<Value, HostError> {
        Err(HostError::NotFound("no repository open".to_owned()))
    }
    fn status(&self, _: &str, _: Option<String>) -> Result<Value, HostError> {
        Err(HostError::NotFound("no repository open".to_owned()))
    }
    fn read_file(&self, _: &str, rel_path: &str) -> Result<Value, HostError> {
        Err(HostError::NotFound(format!("not found: {rel_path}")))
    }
    fn list_dir(&self, _: &str, _: &str) -> Result<Value, HostError> {
        Err(HostError::NotFound("no repository open".to_owned()))
    }
    fn write_file(&self, _: &str, _: &str, _: &str) -> Result<Value, HostError> {
        Err(HostError::NotFound("no repository open".to_owned()))
    }
    fn http_get_json(&self, _: &str, _: &str, _: Value) -> Result<Value, HostError> {
        Err(HostError::Engine("http unavailable in tests".to_owned()))
    }
    fn get_setting(&self, _: &str, _: &str) -> Result<Value, HostError> {
        Ok(serde_json::json!({ "value": null }))
    }
    fn set_setting(&self, _: &str, _: &str, _: Value) -> Result<Value, HostError> {
        Ok(serde_json::json!({}))
    }
    fn git_log(&self, _: &str, _: u32, _: Option<String>) -> Result<Value, HostError> {
        Err(HostError::NotFound("no repository open".to_owned()))
    }
    fn git_stage(&self, _: &str, _: Vec<String>) -> Result<Value, HostError> {
        Err(HostError::NotFound("no repository open".to_owned()))
    }
    fn git_commit(&self, _: &str, _: &str) -> Result<Value, HostError> {
        Err(HostError::Engine("not wired".to_owned()))
    }
    fn register_command(
        &self,
        plugin_id: &str,
        id: &str,
        _: &str,
        _: Option<String>,
    ) -> Result<Value, HostError> {
        self.registrations
            .lock()
            .unwrap()
            .push(format!("command:{plugin_id}:{id}"));
        Ok(serde_json::json!({}))
    }
    fn register_panel(
        &self,
        plugin_id: &str,
        id: &str,
        _: &str,
        _: &str,
    ) -> Result<Value, HostError> {
        self.registrations
            .lock()
            .unwrap()
            .push(format!("panel:{plugin_id}:{id}"));
        Ok(serde_json::json!({}))
    }
    fn show_toast(&self, _: &str, _: &str, _: &str) -> Result<Value, HostError> {
        Ok(serde_json::json!({}))
    }
    fn subscribe_events(&self, _: &str, _: Vec<String>) -> Result<Value, HostError> {
        Ok(serde_json::json!({}))
    }
}

fn load_example(name: &str) -> (WasmiEngine, forgedesk_plugin_host::runtime::PluginHandle) {
    let dir = example_dir(name);
    let manifest_text = std::fs::read_to_string(dir.join("plugin.json"))
        .unwrap_or_else(|error| panic!("example {name} 缺少 plugin.json: {error}"));
    let manifest = PluginManifest::parse(&manifest_text)
        .unwrap_or_else(|error| panic!("example {name} 清单非法: {error}"));
    let wasm = std::fs::read(dir.join("plugin.wasm"))
        .unwrap_or_else(|error| panic!("example {name} 缺少产物 plugin.wasm: {error}"));

    let services: Arc<dyn HostServices> = Arc::new(BareServices::default());
    let engine = WasmiEngine::new(RuntimeLimits::default(), services);
    let handle = engine.load(&manifest, &wasm).expect("load");
    engine.activate(handle).expect("activate");
    (engine, handle)
}

#[test]
fn every_example_manifest_matches_its_declared_contributions() {
    for (name, id) in EXAMPLES {
        let manifest_text = std::fs::read_to_string(example_dir(name).join("plugin.json")).unwrap();
        let manifest = PluginManifest::parse(&manifest_text).unwrap();
        assert_eq!(manifest.id, *id, "示例 {name} 的 id 与登记不一致");
    }
}

#[test]
fn commit_template_answers_the_fill_command_even_without_a_repository() {
    let (engine, handle) = load_example("commit-template");

    let result = engine
        .invoke(handle, "com.example.commit-template.fill-feat", "{}")
        .expect("invoke 不应 trap（无仓库应优雅降级）");
    let payload: Value = serde_json::from_str(&result).expect("结果应是 JSON");
    // 无仓库：get_status 失败 → 插件返回 error 提示而不是伪造内容
    assert!(payload.get("error").is_some(), "实际结果: {result}");
    // 插件没有被隔离：宿主继续可用
    assert_eq!(engine.state(handle).unwrap(), LifecycleState::Active);
}

#[test]
fn repo_stats_panel_renders_valid_dsl_even_without_a_repository() {
    let (engine, handle) = load_example("repo-stats");

    let dsl = engine
        .render_panel(handle, "com.example.repo-stats.stats")
        .expect("render_panel 不应失败");
    validate_panel_dsl(dsl.as_bytes()).expect("DSL 必须通过宿主校验");
}

#[test]
fn repo_audit_panel_and_markdown_report_round_trip() {
    let (engine, handle) = load_example("repo-audit");

    let dsl = engine
        .render_panel(handle, "com.example.repo-audit.audit")
        .expect("render_panel 不应失败");
    validate_panel_dsl(dsl.as_bytes()).expect("DSL 必须通过宿主校验");

    let result = engine
        .invoke(handle, "com.example.repo-audit.copy-report", "{}")
        .expect("copy-report 不应 trap");
    let payload: Value = serde_json::from_str(&result).expect("结果应是 JSON");
    let markdown = payload
        .get("markdown")
        .and_then(Value::as_str)
        .expect("应携带 markdown 字段");
    assert!(
        markdown.contains("仓库巡检报告"),
        "报告标题缺失: {markdown}"
    );
}

#[test]
fn registering_contributions_happens_at_activation() {
    // 三个示例都在 fd_activate 里动态注册贡献点（面板/命令）——
    // 顺带验证 HostServices 的注册路径在真实 wasm 下可用
    let bare = Arc::new(BareServices::default());
    let services: Arc<dyn HostServices> = bare.clone();
    let engine = WasmiEngine::new(RuntimeLimits::default(), services);
    for (name, _) in EXAMPLES {
        let dir = example_dir(name);
        let manifest =
            PluginManifest::parse(&std::fs::read_to_string(dir.join("plugin.json")).unwrap())
                .unwrap();
        let wasm = std::fs::read(dir.join("plugin.wasm")).unwrap();
        let handle = engine.load(&manifest, &wasm).unwrap();
        engine.activate(handle).unwrap();
        engine.unload(handle).unwrap();
    }
    assert!(
        !bare.registrations.lock().unwrap().is_empty(),
        "激活时应产生动态注册"
    );
}
