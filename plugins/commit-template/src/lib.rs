//! 提交信息模板插件（T6.5 示例一）。
//!
//! 提供三套模板（feat / fix / chore）各一个命令：读取当前分支名与变更文件
//! 列表（`get_status`），按**纯字符串规则**生成建议的提交信息骨架并通过
//! `toast` 提示——没有任何 AI 参与（红线 R1）。
//!
//! 模板规则：
//! - 分支名 `feat/…` / `123-描述` 之类只做展示；主题行由用户按变更内容补全；
//! - 变更文件按目录归组给出 `scope` 建议（`crates/foo` → `foo`）；
//! - 生成 `type(scope): ` 前缀 + 空行 + 正文要点（每个文件一行）。

#![no_std]

extern crate alloc;

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use forgedesk_plugin_sdk::{host_call, jstr, log, pack_result, read_args, show_toast};

/// `get_status`（op 2）。
const OP_GET_STATUS: i32 = 2;

#[no_mangle]
pub extern "C" fn fd_activate() -> i32 {
    log(1, "commit-template activated");
    0
}

#[no_mangle]
pub extern "C" fn fd_deactivate() -> i32 {
    0
}

/// 命令入口：命令全名决定模板类型。
#[no_mangle]
pub extern "C" fn fd_invoke(ptr: i32, len: i32) -> i64 {
    // payload 形如 {"command":"<全名>","args":{..}}（引擎注入命令名）
    let payload = read_args(ptr, len);
    let command = extract_field(&payload, "command").unwrap_or_default();
    let kind = match command.rsplit('.').next().unwrap_or_default() {
        "fill-fix" => "fix",
        "fill-chore" => "chore",
        _ => "feat",
    };
    match build_message(kind) {
        Ok(message) => {
            let _ = show_toast("success", "提交信息骨架已生成");
            pack_result(&format!("{{\"message\":{}}}", jstr(&message)))
        }
        Err(error) => {
            log(3, &format!("get_status failed: {error}"));
            pack_result("{\"error\":\"get_status failed (is a repository open?)\"}")
        }
    }
}

fn build_message(kind: &str) -> Result<String, String> {
    let raw = host_call(OP_GET_STATUS, "{}")?;
    // 宿主返回 {"entries":[{"path":"..","status":".."},..]}；只取路径与目录
    let paths = extract_paths(&raw);
    Ok(render(kind, &paths))
}

/// 从受控 JSON 文本里取一个字符串字段的值（避免拖完整解析器撑大 wasm）。
fn extract_field(raw: &str, field: &str) -> Option<String> {
    let marker = format!("\"{}\":\"", field);
    let pos = raw.find(&marker)? + marker.len();
    let tail = &raw[pos..];
    let end = tail.find('"')?;
    Some(tail[..end].to_owned())
}

/// 极简提取：在 JSON 文本里抓所有 `"path":"…"` 的值（受控形状够用，避免
/// 插件里拖一个完整 JSON 解析器把 wasm 撑大）。
fn extract_paths(raw: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let marker = "\"path\":\"";
    let mut rest = raw;
    while let Some(pos) = rest.find(marker) {
        let tail = &rest[pos + marker.len()..];
        if let Some(end) = tail.find('"') {
            paths.push(tail[..end].to_owned());
            rest = &tail[end..];
        } else {
            break;
        }
    }
    paths
}

/// 按目录归组建议 scope（`crates/foo/x.rs` → `foo`；根文件 → 无 scope）。
fn suggest_scope(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').collect();
    match segments.len() {
        0 | 1 => None,
        _ => {
            let first = segments[0];
            let scope = if first == "crates" || first == "plugins" || first == "src" {
                segments.get(1).copied().unwrap_or(first)
            } else {
                first
            };
            Some(scope.to_owned())
        }
    }
}

fn render(kind: &str, paths: &[String]) -> String {
    let mut scopes: Vec<String> = paths.iter().filter_map(|p| suggest_scope(p)).collect();
    scopes.sort();
    scopes.dedup();
    let scope_part = if scopes.len() == 1 {
        format!("({})", scopes[0])
    } else {
        String::new()
    };
    let mut body = String::new();
    for path in paths.iter().take(10) {
        body.push_str(&format!("- {path}\n"));
    }
    if paths.len() > 10 {
        body.push_str(&format!("- … 其余 {} 个文件\n", paths.len() - 10));
    }
    format!("{kind}{scope_part}: \n\n{kind} body:\n{body}")
}
