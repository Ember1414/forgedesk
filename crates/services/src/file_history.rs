//! 文件级历史（T5.8）：blame / 文件历史 / 历史版本内容。
//!
//! 解析器在 [`forgedesk_git_engine::parsers::blame`]（纯函数、表驱动单测）；
//! 本层负责用 [`GitProcess`] 执行 git 命令（§7 规范：参数数组、固定环境、
//! 超时、机器可解析格式）并组装结果。
//!
//! # 性能口径（任务书：5000 行 < 2s）
//!
//! `git blame --line-porcelain` 本身是 C 实现的单遍扫描，5000 行文件在
//! 本地仓库上是毫秒级；真正的成本在 IPC 序列化（每行一条记录）。
//! `range` 参数透传 `-L`，前端"降级为可见区域 blame"时用它缩小范围。

use std::path::Path;

use forgedesk_domain::{AppError, ErrorCode};
use forgedesk_git_engine::parsers::parse_blame_porcelain;
use forgedesk_git_engine::process::{GitProcess, GitRunOpts};
use serde::{Deserialize, Serialize};

/// blame 选项。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BlameOptions {
    /// 忽略行尾空白（`-w`）。
    pub ignore_whitespace: bool,
    /// 检测移动/复制的行（`-M`）。
    pub detect_moves: bool,
    /// `-L <start>,<end>`：只 blame 指定行区间（降级模式用）。
    pub range: Option<String>,
}

/// blame 输出格式串（porcelain 不需要格式串，这里仅文档化参数集）。
pub const BLAME_ARGS: &[&str] = &["blame", "--line-porcelain"];

/// 对文件执行 blame，返回逐行归属。
pub async fn git_blame(
    root: &Path,
    rel_path: &str,
    options: &BlameOptions,
) -> Result<Vec<forgedesk_git_engine::parsers::blame::BlameLine>, AppError> {
    let mut args: Vec<String> = vec!["blame".into(), "--line-porcelain".into()];
    if options.ignore_whitespace {
        args.push("-w".into());
    }
    if options.detect_moves {
        args.push("-M".into());
    }
    if let Some(range) = &options.range {
        args.push("-L".into());
        args.push(range.clone());
    }
    args.push("--".into());
    args.push(rel_path.into());

    let output = GitProcess::new().run(&args, GitRunOpts::new(root)).await?;
    if output.exit_code != Some(0) {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let code = if stderr.contains("no such path") || stderr.contains("not found in") {
            ErrorCode::NotFound
        } else {
            ErrorCode::Internal
        };
        return Err(AppError::new(code, "git blame failed").with_detail(stderr));
    }
    let text = if output.stdout_is_utf8 {
        String::from_utf8(output.stdout).unwrap_or_default()
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    Ok(parse_blame_porcelain(&text))
}

/// 文件历史条目（含变更类型 A/M/D/R）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHistoryEntry {
    /// 提交哈希（40 位）。
    pub oid: String,
    /// 作者名。
    pub author: String,
    /// 作者时间（Unix 秒）。
    pub author_time: i64,
    /// 提交标题。
    pub subject: String,
    /// 变更类型（A/M/D/R…，来自 `--name-status`）。
    pub change_kind: String,
    /// 重命名/复制时的旧路径（R/C 才有）。
    pub old_path: Option<String>,
}

/// 文件历史分页结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHistoryPage {
    /// 当前页条目。
    pub items: Vec<FileHistoryEntry>,
    /// 下一页游标（offset）；None = 没有更多。
    pub next_cursor: Option<u64>,
}

/// 字段分隔符（与 parsers::log 的 LOG_FORMAT 同口径）。
const FIELD_SEPARATOR: char = '\u{1f}';
/// 记录分隔符。
const RECORD_SEPARATOR: char = '\u{1e}';

/// `git log --follow --name-status` 的格式串：oid / author / time / subject。
const FILE_HISTORY_FORMAT: &str = "%H%x1f%an%x1f%at%x1f%s";

/// 列出文件的提交历史（`--follow` 跟随重命名）。
///
/// 分页按 offset（cursor）重跑 git log：文件级历史通常很短（几十条），
/// 深分页的 walk 前缀缓存（T2.9）对它没有收益。
pub async fn git_file_history(
    root: &Path,
    rel_path: &str,
    follow: bool,
    limit: u32,
    cursor: u64,
) -> Result<FileHistoryPage, AppError> {
    let mut args: Vec<String> = vec![
        "log".into(),
        "--name-status".into(),
        "--format".into(),
        format!("{FILE_HISTORY_FORMAT}{RECORD_SEPARATOR}"),
    ];
    if follow {
        args.push("--follow".into());
    }
    args.extend([
        "--skip".into(),
        cursor.to_string(),
        "--max-count".into(),
        limit.to_string(),
        "--".into(),
        rel_path.into(),
    ]);

    let output = GitProcess::new().run(&args, GitRunOpts::new(root)).await?;
    if output.exit_code != Some(0) {
        return Err(AppError::new(ErrorCode::Internal, "git log failed")
            .with_detail(String::from_utf8_lossy(&output.stderr).into_owned()));
    }
    let text = if output.stdout_is_utf8 {
        String::from_utf8(output.stdout).unwrap_or_default()
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };

    let mut items = Vec::new();
    for record in text.split(RECORD_SEPARATOR) {
        let record = record.trim_start_matches('\n').trim_end_matches('\n');
        if record.is_empty() {
            continue;
        }
        let mut lines = record.lines();
        let Some(header) = lines.next() else { continue };
        let fields: Vec<&str> = header.split(FIELD_SEPARATOR).collect();
        if fields.len() != 4 {
            continue; // 字段个数不符：跳过（与 log 解析器同口径，不猜）
        }
        // name-status 行：`M\tpath` / `R100\told\tnew`（取第一行类型即可——
        // 一个提交对单文件通常只有一种变更；合并提交可能多行，取字母最大的）
        let mut change_kind = String::new();
        let mut old_path = None;
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split('\t');
            let kind = parts.next().unwrap_or("").to_string();
            if kind.starts_with('R') || kind.starts_with('C') {
                if let Some(old) = parts.next() {
                    old_path = Some(old.to_string());
                }
            }
            if kind.len() > change_kind.len() || change_kind.is_empty() {
                change_kind = kind.chars().next().unwrap_or('M').to_string();
            }
        }
        if change_kind.is_empty() {
            change_kind = "M".into();
        }
        items.push(FileHistoryEntry {
            oid: fields[0].to_string(),
            author: fields[1].to_string(),
            author_time: fields[2].parse().unwrap_or(0),
            subject: fields[3].to_string(),
            change_kind,
            old_path,
        });
    }

    let next_cursor = if items.len() == limit as usize {
        Some(cursor + limit as u64)
    } else {
        None
    };
    Ok(FileHistoryPage { items, next_cursor })
}

/// 读取历史版本的单文件内容（`git show <rev>:<path>`）。
///
/// 返回原始字节与是否二进制；5MB 上限与 [`fs_read`](crate::workspace_fs) 同口径。
pub async fn git_file_at(
    root: &Path,
    rel_path: &str,
    rev: &str,
) -> Result<(Vec<u8>, bool), AppError> {
    // rev 来自前端（提交哈希），先做形状校验再拼参数——零信任。
    if rev.is_empty() || !rev.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(AppError::new(ErrorCode::Validation, "invalid revision"));
    }
    let output = GitProcess::new()
        .run(
            &["show".into(), format!("{rev}:{rel_path}")],
            GitRunOpts::new(root),
        )
        .await?;
    if output.exit_code != Some(0) {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "the file does not exist at that revision",
        )
        .with_detail(String::from_utf8_lossy(&output.stderr).into_owned()));
    }
    if output.stdout.len() > 5 * 1024 * 1024 {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the historical file exceeds the 5MB limit",
        ));
    }
    let is_binary = output.stdout[..output.stdout.len().min(8000)].contains(&0);
    Ok((output.stdout, is_binary))
}
