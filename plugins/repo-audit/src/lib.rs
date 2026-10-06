//! 仓库巡检插件（T6.5 示例三）——全部**只读**。
//!
//! 检查项（本期可落地的四项 + 一项说明）：
//! 1. 疑似密钥文件：`.env` / `.env.local` / `secrets.pem` 等存在于仓库根；
//! 2. 未忽略的构建产物：状态列表里出现 `target/`、`node_modules/`、`dist/` 前缀
//!    （被 .gitignore 正确忽略时不会出现在状态里）；
//! 3. 大小写冲突风险：仓库根存在仅大小写不同的文件名；
//! 4. 提交信息规范符合率：最近 100 条中符合 `type(scope): ` / `type: ` 前缀的比例；
//! 5. 超大文件（>100MB 建议 LFS）：当前宿主接口无文件大小查询，本期不实现
//!    （README 与报告里如实标注）。
//!
//! 报告以面板 DSL 输出；`copy-report` 命令返回 Markdown 供界面复制/导出。

#![no_std]

extern crate alloc;

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use forgedesk_plugin_sdk::{host_call, jstr, log, pack_result, read_args};

const OP_GET_STATUS: i32 = 2;
const OP_LIST_DIR: i32 = 4;
const OP_GET_GIT_LOG: i32 = 9;

const SUSPICIOUS_FILES: [&str; 5] = [
    ".env",
    ".env.local",
    "secrets.pem",
    "id_rsa",
    "credentials.json",
];
const BUILD_PREFIXES: [&str; 4] = ["target/", "node_modules/", "dist/", "build/"];
/// 提交信息规范：Conventional Commits 前缀。
const CONVENTIONAL_PREFIXES: [&str; 10] = [
    "feat", "fix", "chore", "docs", "test", "refactor", "perf", "ci", "build", "style",
];

#[no_mangle]
pub extern "C" fn fd_activate() -> i32 {
    let _ = host_call(
        13,
        &format!(
            "{{\"id\":{},\"title\":{},\"location\":\"repo-tab\"}}",
            jstr("audit"),
            jstr("仓库巡检")
        ),
    );
    log(1, "repo-audit activated");
    0
}

#[no_mangle]
pub extern "C" fn fd_deactivate() -> i32 {
    0
}

#[no_mangle]
pub extern "C" fn fd_render_panel(ptr: i32, len: i32) -> i64 {
    let _ = read_args(ptr, len);
    match audit_and_render() {
        Ok(dsl) => pack_result(&dsl),
        Err(error) => {
            log(3, &format!("audit failed: {error}"));
            pack_result(
                "[{\"type\":\"text\",\"text\":\"巡检失败（需要打开一个仓库）。\",\"tone\":\"danger\"}]",
            )
        }
    }
}

/// `copy-report` 命令：返回 Markdown 报告。
#[no_mangle]
pub extern "C" fn fd_invoke(ptr: i32, len: i32) -> i64 {
    let payload = read_args(ptr, len);
    let command = extract_field(&payload, "command").unwrap_or_default();
    if command.ends_with("copy-report") {
        // 审计失败（如没有打开的仓库）也产出报告骨架——降级不缺标题
        let markdown = audit_markdown().unwrap_or_else(|error| {
            format!(
                "# 仓库巡检报告

- 生成失败：{error}
"
            )
        });
        pack_result(&format!("{{\"markdown\":{}}}", jstr(&markdown)))
    } else {
        pack_result("{\"error\":\"unknown command\"}")
    }
}

/// 巡检结果：`None` 项 = 通过（不在 DSL 里渲染）。
struct Findings {
    secrets: Vec<String>,
    build_outputs: Vec<String>,
    case_conflicts: Vec<String>,
    conventional_rate: Option<(usize, usize)>,
}

fn audit() -> Result<Findings, String> {
    let status_raw = host_call(OP_GET_STATUS, "{}")?;
    let entries = extract_paths(&status_raw);

    // 1. 疑似密钥文件
    let mut secrets = Vec::new();
    for candidate in SUSPICIOUS_FILES {
        // read_file 命中即存在（内容不落日志——红线 R8）
        if host_call(3, &format!("{{\"path\":{}}}", jstr(candidate))).is_ok() {
            secrets.push(candidate.to_owned());
        }
    }

    // 2. 未忽略的构建产物
    let build_outputs: Vec<String> = entries
        .iter()
        .filter(|path| BUILD_PREFIXES.iter().any(|prefix| path.starts_with(prefix)))
        .cloned()
        .take(10)
        .collect();

    // 3. 大小写冲突（仓库根）
    let root_raw = host_call(OP_LIST_DIR, "{\"path\":\".\"}")?;
    let root_names = extract_names(&root_raw);
    let mut case_conflicts = Vec::new();
    for (index, name) in root_names.iter().enumerate() {
        let lower = name.to_lowercase();
        if let Some(other) = root_names
            .iter()
            .enumerate()
            .take(index)
            .find(|(_, other)| other.to_lowercase() == lower)
            .map(|(_, other)| other.clone())
        {
            case_conflicts.push(format!("{other} vs {name}"));
        }
    }

    // 4. 提交信息规范符合率
    let log_raw = host_call(OP_GET_GIT_LOG, "{\"limit\":100}")?;
    let subjects = extract_subjects(&log_raw);
    let conventional_rate = (!subjects.is_empty()).then(|| {
        let hit = subjects
            .iter()
            .filter(|subject| {
                let Some((prefix, rest)) = subject.split_once(':') else {
                    return false;
                };
                let head = prefix.split('(').next().unwrap_or(prefix).trim();
                CONVENTIONAL_PREFIXES.contains(&head) && rest.starts_with(' ')
            })
            .count();
        (hit, subjects.len())
    });

    Ok(Findings {
        secrets,
        build_outputs,
        case_conflicts,
        conventional_rate,
    })
}

fn audit_markdown() -> Result<String, String> {
    let findings = audit()?;
    let mut md = String::from("# 仓库巡检报告\n\n");
    if findings.secrets.is_empty() {
        md.push_str("- 疑似密钥文件：未发现 ✓\n");
    } else {
        md.push_str("- **疑似密钥文件**（建议加入 .gitignore 并从历史移除）：\n");
        for secret in &findings.secrets {
            md.push_str(&format!("  - {secret}\n"));
        }
    }
    if findings.build_outputs.is_empty() {
        md.push_str("- 未忽略的构建产物：未发现 ✓\n");
    } else {
        md.push_str("- **未忽略的构建产物**（建议补 .gitignore）：\n");
        for output in &findings.build_outputs {
            md.push_str(&format!("  - {output}\n"));
        }
    }
    if findings.case_conflicts.is_empty() {
        md.push_str("- 大小写冲突风险：未发现 ✓\n");
    } else {
        md.push_str("- **大小写冲突风险**（跨平台检出会撞名）：\n");
        for conflict in &findings.case_conflicts {
            md.push_str(&format!("  - {conflict}\n"));
        }
    }
    match findings.conventional_rate {
        None => md.push_str("- 提交信息规范：样本为空\n"),
        Some((hit, total)) => md.push_str(&format!(
            "- 提交信息规范符合率：{hit}/{total}（{}%）\n",
            hit * 100 / total.max(1)
        )),
    }
    md.push_str("\n> 超大文件（>100MB 建议 LFS）检查需要宿主的文件大小接口，本期未实现。\n");
    Ok(md)
}

fn render_dsl(findings: &Findings) -> String {
    let mut blocks = String::from("[{\"type\":\"heading\",\"text\":\"仓库巡检\"},");
    let ok = |text: &str| {
        format!(
            "{{\"type\":\"text\",\"text\":{},\"tone\":\"success\"}},",
            jstr(text)
        )
    };
    let warn = |text: &str| {
        format!(
            "{{\"type\":\"text\",\"text\":{},\"tone\":\"danger\"}},",
            jstr(text)
        )
    };
    if findings.secrets.is_empty() {
        blocks.push_str(&ok("疑似密钥文件：未发现 ✓"));
    } else {
        blocks.push_str(&warn(&format!(
            "疑似密钥文件：{}",
            findings.secrets.join(", ")
        )));
    }
    if findings.build_outputs.is_empty() {
        blocks.push_str(&ok("未忽略的构建产物：未发现 ✓"));
    } else {
        blocks.push_str(&format!(
            "{{\"type\":\"list\",\"items\":[{}]}},",
            findings
                .build_outputs
                .iter()
                .map(|output| jstr(output))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if findings.case_conflicts.is_empty() {
        blocks.push_str(&ok("大小写冲突风险：未发现 ✓"));
    } else {
        blocks.push_str(&warn(&format!(
            "大小写冲突：{}",
            findings.case_conflicts.join(", ")
        )));
    }
    if let Some((hit, total)) = findings.conventional_rate {
        let value = hit * 100 / total.max(1);
        blocks.push_str(&format!(
            "{{\"type\":\"progress\",\"label\":{},\"value\":{}}},",
            jstr(&format!("提交信息规范符合率（{hit}/{total}）")),
            value
        ));
    }
    blocks.push_str("{\"type\":\"button\",\"command\":\"com.example.repo-audit.copy-report\",\"label\":\"复制报告（Markdown）\"}]");
    blocks
}

fn audit_and_render() -> Result<String, String> {
    let findings = audit()?;
    Ok(render_dsl(&findings))
}

/// 极简 `"path":"…"` 抽取。
fn extract_paths(raw: &str) -> Vec<String> {
    extract_string_array(raw, "\"path\":\"")
}

/// 极简 `"name":"…"` 抽取。
fn extract_names(raw: &str) -> Vec<String> {
    extract_string_array(raw, "\"name\":\"")
}

/// 极简 `"summary":"…"` 抽取。
fn extract_subjects(raw: &str) -> Vec<String> {
    extract_string_array(raw, "\"summary\":\"")
}

fn extract_string_array(raw: &str, marker: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = raw;
    while let Some(pos) = rest.find(marker) {
        let tail = &rest[pos + marker.len()..];
        if let Some(end) = tail.find('"') {
            out.push(tail[..end].to_owned());
            rest = &tail[end..];
        } else {
            break;
        }
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
