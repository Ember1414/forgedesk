//! 仓库统计面板插件（T6.5 示例二）。
//!
//! 在 sidebar 注册面板：最近 30 天提交数、作者分布（表格）、按小时分布的
//! "条形热力图"（文本条形）、当前分支。全部数据来自 `get_repo_info` 与
//! `get_git_log`（最近 100 条内的聚合）。

#![no_std]

extern crate alloc;

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use forgedesk_plugin_sdk::{host_call, jstr, log, pack_result};

const OP_GET_REPO_INFO: i32 = 1;
const OP_GET_GIT_LOG: i32 = 9;

/// 一天的小时数（热力图桶）。
const HOUR_BUCKETS: usize = 24;

#[no_mangle]
pub extern "C" fn fd_activate() -> i32 {
    // 注册面板贡献点（清单里也声明了；动态注册演示扩展点用法）
    let _ = host_call(
        13,
        &format!(
            "{{\"id\":{},\"title\":{},\"location\":\"sidebar\"}}",
            jstr("stats"),
            jstr("仓库统计")
        ),
    );
    log(1, "repo-stats activated");
    0
}

#[no_mangle]
pub extern "C" fn fd_deactivate() -> i32 {
    0
}

/// 面板渲染：返回声明式 DSL。
#[no_mangle]
pub extern "C" fn fd_render_panel(ptr: i32, len: i32) -> i64 {
    let _panel_id = forgedesk_plugin_sdk::read_args(ptr, len);
    match render() {
        Ok(dsl) => pack_result(&dsl),
        Err(error) => {
            log(3, &format!("render failed: {error}"));
            pack_result(
                "[{\"type\":\"text\",\"text\":\"统计生成失败（需要打开一个仓库）。\",\"tone\":\"danger\"}]",
            )
        }
    }
}

fn render() -> Result<String, String> {
    let info = host_call(OP_GET_REPO_INFO, "{}")?;
    let branch = extract_field(&info, "currentBranch").unwrap_or_else(|| "(detached)".to_owned());
    let raw = host_call(OP_GET_GIT_LOG, "{\"limit\":100}")?;
    let commits = extract_commits(&raw);

    // no_std 没有 SystemTime：以"样本中最新提交时间"为锚点往回推 30 天——
    // 相对口径对统计分布完全够用，且避免插件依赖墙钟
    let max_time = commits.iter().map(|(_, time, _)| *time).max().unwrap_or(0);
    let month_ago = max_time - 30 * 24 * 3600;

    let mut recent = 0usize;
    let mut authors: Vec<(String, usize)> = Vec::new();
    let mut hours = [0usize; HOUR_BUCKETS];
    for (author, time, hour) in &commits {
        if *time >= month_ago {
            recent += 1;
            bump(&mut authors, author.clone());
        }
        hours[*hour as usize % HOUR_BUCKETS] += 1;
    }
    authors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let mut dsl = String::from("[");
    dsl.push_str(&format!(
        "{{\"type\":\"heading\",\"text\":{}}},",
        jstr(&format!("仓库统计 · {branch}"))
    ));
    dsl.push_str(&format!(
        "{{\"type\":\"text\",\"text\":{},\"tone\":\"muted\"}},",
        jstr(&format!(
            "最近 30 天提交：{recent}（样本：最近 {} 条）",
            commits.len()
        ))
    ));

    // 作者分布（前 5）
    dsl.push_str("{\"type\":\"heading\",\"text\":\"作者分布\"},");
    let mut table = String::from("{\"type\":\"table\",\"columns\":[\"作者\",\"提交\"],\"rows\":[");
    for (index, (author, count)) in authors.iter().take(5).enumerate() {
        if index > 0 {
            table.push(',');
        }
        table.push_str(&format!("[{},{}]", jstr(author), jstr(&count.to_string())));
    }
    table.push_str("]},");
    dsl.push_str(&table);

    // 按小时分布：文本条形（█ 数量与峰值成比例）
    let peak = hours.iter().copied().max().unwrap_or(0).max(1);
    dsl.push_str("{\"type\":\"heading\",\"text\":\"提交时间分布（按小时）\"},");
    let mut bars = String::from("{\"type\":\"list\",\"items\":[");
    for (hour, count) in hours.iter().enumerate() {
        let width = (count * 20) / peak;
        let bar: String = "█".repeat(width);
        if hour > 0 {
            bars.push(',');
        }
        bars.push_str(&jstr(&format!("{hour:02}:00 {bar} {}", count)));
    }
    bars.push_str("]}");
    dsl.push_str(&bars);
    dsl.push(']');
    Ok(dsl)
}

fn bump(entries: &mut Vec<(String, usize)>, author: String) {
    for entry in entries.iter_mut() {
        if entry.0 == author {
            entry.1 += 1;
            return;
        }
    }
    entries.push((author, 1));
}

/// 从 git_log 结果里抽 (作者, 时间秒, 小时) 三元组（受控形状的极简解析）。
fn extract_commits(raw: &str) -> Vec<(String, i64, i64)> {
    let mut out = Vec::new();
    let mut rest = raw;
    loop {
        let Some(pos) = rest.find("\"author\":") else {
            break;
        };
        let after = &rest[pos + "\"author\":\"".len()..];
        let Some(author_end) = after.find('"') else {
            break;
        };
        let author = after[..author_end].to_owned();
        let tail = &after[author_end..];
        // 时间在相邻字段："time":<秒>
        let time = tail
            .find("\"time\":")
            .and_then(|pos| {
                let digits: String = tail[pos + "\"time\":".len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                digits.parse::<i64>().ok()
            })
            .unwrap_or(0);
        let hour = if time > 0 { time % 86_400 / 3600 } else { 0 }; // UTC 小时（避免引入时区依赖）
        out.push((author, time, hour));
        rest = tail;
    }
    out
}

fn extract_field(raw: &str, field: &str) -> Option<String> {
    let marker = format!("\"{}\":\"", field);
    let pos = raw.find(&marker)? + marker.len();
    let tail = &raw[pos..];
    let end = tail.find('"')?;
    Some(tail[..end].to_owned())
}
