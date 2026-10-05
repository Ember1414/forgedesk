//! `git blame --line-porcelain` 的解析器。
//!
//! # 为什么用 `--line-porcelain`
//!
//! porcelain 格式的每个字段都是 `键 值` 的独立逻辑行，**每一行重复完整
//! header**（这正是 porcelain 与默认格式的区别），因此解析是流式的状态机：
//! 见到 `<sha> <orig> <final> [<count>]` 就开新记录，见到 `author`/
//! `summary` 等键就填字段，`\t` 开头的行是文件内容并终止该记录。
//! 不需要 `-z`：内容行以 `\t` 为界，`filename` 值里的空格没有歧义
//! （ porcelain 里它是最后一个 header，其值到行尾为止）。
//!
//! # 未提交行与边界
//!
//! - 全零 SHA = 工作区里**未提交**的行（`is_uncommitted`）；
//! - `boundary` 标记 = 历史边界（浅历史的第一代），照常解析。
//!
//! # 重命名检测
//!
//! `previous <sha> <file>` 只在 blame 跟随了重命名（`-C`/`-M`）且该提交
//! 之前文件名不同的时候出现——`FileBlameLine.previous_path` 如实携带，
//! 调用方（文件历史）用它标注"来自重命名"。

use serde::{Deserialize, Serialize};

/// 一个 blame 记录（对应文件中的一行）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameLine {
    /// 该行归属的提交（40 位 hex；全零 = 未提交）。
    pub oid: String,
    /// 8 位短哈希（未提交行为全零的 8 位）。
    pub short_oid: String,
    /// 文件中的行号（1 起）。
    pub line_no: u32,
    /// 作者名（未提交行 = 工作区用户）。
    pub author: String,
    /// 作者邮箱。
    pub author_mail: String,
    /// 作者时间（Unix 秒）。
    pub author_time: i64,
    /// 提交标题。
    pub summary: String,
    /// 是否未提交（全零 SHA）。
    pub is_uncommitted: bool,
    /// blame 跟随重命名时，该提交之前的文件路径。
    pub previous_path: Option<String>,
}

/// 解析 `git blame --line-porcelain <path>` 的完整输出。
///
/// 解析器不做 IO、不校验仓库状态；垃圾输入产出空结果或尽力解析的记录
/// （与 `log` 解析器同口径：格式不符的字段跳过，不猜测）。
#[must_use]
pub fn parse_blame_porcelain(input: &str) -> Vec<BlameLine> {
    let mut lines: Vec<BlameLine> = Vec::new();
    let mut current: Option<BlameLine> = None;

    for raw in input.lines() {
        if let Some(content) = raw.strip_prefix('\t') {
            // 内容行终止一条记录（porcelain 每行重复 header，所以一个记录
            // 就是一行文件的完整归属信息）
            if let Some(mut record) = current.take() {
                record.oid = record.oid.to_string();
                record.summary = record.summary.clone();
                let _ = content; // 行内容前端从文件自身取，blame 只负责归属
                lines.push(record);
            }
            continue;
        }

        if raw.len() >= 41 && raw.as_bytes()[40] == b' ' {
            // 新记录头：`<40sha> <orig> <final> [<count>]`
            let sha = &raw[..40];
            if sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                if let Some(previous) = current.take() {
                    lines.push(previous);
                }
                let is_uncommitted = sha.bytes().all(|b| b == b'0');
                current = Some(BlameLine {
                    oid: sha.to_string(),
                    short_oid: sha[..8].to_string(),
                    line_no: raw
                        .split_whitespace()
                        .nth(2)
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(0),
                    author: String::new(),
                    author_mail: String::new(),
                    author_time: 0,
                    summary: String::new(),
                    is_uncommitted,
                    previous_path: None,
                });
                continue;
            }
        }

        if let Some(record) = current.as_mut() {
            if let Some((key, value)) = raw.split_once(' ') {
                match key {
                    "author" => record.author = value.to_string(),
                    "author-mail" => {
                        record.author_mail = value
                            .trim_start_matches('<')
                            .trim_end_matches('>')
                            .to_string();
                    }
                    "author-time" => record.author_time = value.parse().unwrap_or(0),
                    "summary" => record.summary = value.to_string(),
                    "previous" => {
                        // `previous <sha> <old-path>`：old-path 可含空格，取第二个空格之后
                        if let Some((_prev_sha, path)) = value.split_once(' ') {
                            record.previous_path = Some(path.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(last) = current.take() {
        lines.push(last);
    }
    lines
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::parse_blame_porcelain;

    const SAMPLE: &str = "\
a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8a1b2c3d4 1 1 2
author Alice
author-mail <alice@example.com>
author-time 1696000000
author-tz +0800
committer Alice
committer-mail <alice@example.com>
committer-time 1696000000
committer-tz +0800
summary add main module

        fn main() {}
a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8a1b2c3d4 2 2
author Alice
author-mail <alice@example.com>
author-time 1696000000
author-tz +0800
committer Alice
committer-mail <alice@example.com>
committer-time 1696000000
committer-tz +0800
summary add main module

        let x = 1;
0000000000000000000000000000000000000000 3 3
author Work Dir
author-mail <work@local>
author-time 1700000000
author-tz +0800
committer Work Dir
committer-mail <work@local>
committer-time 1700000000
committer-tz +0800
summary Uncommitted changes

        let y = 2;
b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8a1b2c3d4e5 4 4 1
author Bob
author-mail <bob@example.com>
author-time 1696100000
author-tz +0800
committer Bob
committer-mail <bob@example.com>
committer-time 1696100000
committer-tz +0800
summary rename: src/lib.rs
previous 9876543210987654321098765432109876543210 src/old.rs

        fn helper()
";

    /// 完整解析：四条记录、字段齐全、顺序保持。
    #[test]
    fn parses_full_porcelain_sample_in_order() {
        let lines = parse_blame_porcelain(SAMPLE);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].oid, "a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8a1b2c3d4");
        assert_eq!(lines[0].short_oid, "a1b2c3d4");
        assert_eq!(lines[0].line_no, 1);
        assert_eq!(lines[0].author, "Alice");
        assert_eq!(lines[0].author_mail, "alice@example.com");
        assert_eq!(lines[0].author_time, 1_696_000_000);
        assert_eq!(lines[0].summary, "add main module");
    }

    /// 未提交行：全零 SHA → is_uncommitted。
    #[test]
    fn marks_all_zero_sha_as_uncommitted() {
        let lines = parse_blame_porcelain(SAMPLE);
        assert!(lines[2].is_uncommitted);
        assert!(!lines[0].is_uncommitted);
        assert_eq!(lines[2].author, "Work Dir");
    }

    /// 重命名检测：`previous` 携带旧路径。
    #[test]
    fn carries_previous_path_for_renames() {
        let lines = parse_blame_porcelain(SAMPLE);
        assert_eq!(lines[3].previous_path.as_deref(), Some("src/old.rs"));
        assert_eq!(lines[3].summary, "rename: src/lib.rs");
        assert!(lines[0].previous_path.is_none());
    }

    /// 垃圾输入：产出空结果，不 panic。
    #[test]
    fn garbage_input_yields_empty() {
        assert!(parse_blame_porcelain("").is_empty());
        assert!(parse_blame_porcelain("not a blame output\nat all").is_empty());
    }
}
