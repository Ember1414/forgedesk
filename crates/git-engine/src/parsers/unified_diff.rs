//! `git diff --no-color -U<n>` 统一补丁的解析器（T1.5）。
//!
//! # 为什么不解析 `diff --git` 行里的路径
//!
//! `diff --git a/X b/Y` 在路径含空格时**天然有歧义**（git 文本输出的已知缺陷：
//! 默认不对含空格路径加引号）。可靠的做法有两个：
//!
//! 1. `--- a/路径` / `+++ b/路径` 行没有歧义（路径是前缀之后的全部），
//!    但**二进制文件没有这两行**；
//! 2. **段顺序对齐**：同一次 `git diff` 的 numstat 与补丁输出的文件顺序一致，
//!    因此解析器只需按顺序产出"段"，由调用方（CLI diff 实现）与 numstat
//!    的结果按下标拼合——路径以 numstat 为准（字节精确），补丁段只贡献 hunk。
//!
//! # 为什么按头部声明的行数"精确消费"
//!
//! hunk 体里**内容行自己可能以 `---` 开头**（删除了一行 `--` 的文件会渲染出
//! `---`）。靠"看到 `--- ` 就当新文件"会切碎 hunk；头部 `@@ -a,b +c,d @@`
//! 声明了两侧行数，按它消费恰好干净：`\ No newline` 标记不占行数。

use forgedesk_domain::git::{
    DiffHunk, DiffLine, DiffLineKind, RepoPath, MAX_DIFF_BYTES_PER_FILE, MAX_DIFF_LINES_PER_FILE,
};

/// 一个文件段解析出的行级内容（统计与变更类别由 numstat/name-status 提供）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedPatchSection {
    /// 目标路径（尽力提取，仅用于一致性检查；**权威路径来自 numstat**）。
    pub path: Option<RepoPath>,
    /// 来源路径（重命名的 `rename from`）。
    pub original_path: Option<RepoPath>,
    /// 是否为二进制（`Binary files … differ` / `GIT binary patch`）。
    pub binary: bool,
    /// 解析出的 hunk。
    pub hunks: Vec<DiffHunk>,
    /// 是否因超过预算被截断。
    pub truncated: bool,
}

/// 截断预算（行数与 patch 字节数，按文件段计）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchLimits {
    /// 单文件段允许的最大行级行数。
    pub max_lines: usize,
    /// 单文件段允许的最大 patch 字节数。
    pub max_bytes: usize,
}

impl PatchLimits {
    /// 默认预算。
    pub fn defaults() -> Self {
        Self {
            max_lines: MAX_DIFF_LINES_PER_FILE,
            max_bytes: MAX_DIFF_BYTES_PER_FILE,
        }
    }
}

/// 解析统一补丁，按文件顺序返回"段"。
///
/// 畸形行（无法识别的前缀）被跳过而不是让整个解析失败：
/// diff 输出可能混入第三方 textconv/filter 的产物，鲁棒性优先。
pub fn parse_unified_diff(
    input: &[u8],
    limits: &PatchLimits,
    force_full: bool,
) -> Vec<ParsedPatchSection> {
    let text = String::from_utf8_lossy(input);
    let lines: Vec<&str> = text.split('\n').collect();
    let mut sections = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        if !lines[index].starts_with("diff --git ") {
            index += 1;
            continue;
        }
        index += 1;

        let mut section = ParsedPatchSection::default();
        let mut consumed_lines = 0_usize;
        // ---- 头部：直到 @@ / Binary / 下一个 diff --git
        while index < lines.len() {
            let line = lines[index];
            if line.starts_with("diff --git ") {
                break;
            }
            if line.starts_with("@@ ") {
                // 头部结束：当前行是第一个 hunk 头，交给 hunk 循环处理。
                // 忘了这条 break 时，@@ 行会被当作杂项吞掉，整个文件的 hunk 全部丢失
                // （单测抓到的真实 bug：三个三文件用例全过、单文件用例全挂，
                //   因为多文件样本的断言先死在别处，掩盖了路径）。
                break;
            }
            if let Some(rest) = line.strip_prefix("--- ") {
                if rest != "/dev/null" {
                    section.path = Some(RepoPath::from(strip_prefix_and_unquote(rest, "a/")));
                }
            } else if let Some(rest) = line.strip_prefix("+++ ") {
                if rest != "/dev/null" {
                    section.path = Some(RepoPath::from(strip_prefix_and_unquote(rest, "b/")));
                }
            } else if let Some(rest) = line.strip_prefix("rename from ") {
                section.original_path = Some(RepoPath::from(unquote(rest)));
            } else if let Some(rest) = line.strip_prefix("rename to ") {
                section.path = Some(RepoPath::from(unquote(rest)));
            } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
                section.binary = true;
                index += 1;
                // GIT binary patch 的载荷（zlib/base85 行）没有结构价值，跳到下一个段
                while index < lines.len() && !lines[index].starts_with("diff --git ") {
                    index += 1;
                }
                break;
            }
            index += 1;
        }

        // ---- hunk 体
        while index < lines.len() {
            let line = lines[index];
            if line.starts_with("diff --git ") {
                break;
            }
            let Some(hunk) = parse_hunk_header(line) else {
                // 头部区域的杂项行（index/mode/similarity…）直接跳过
                index += 1;
                continue;
            };
            index += 1;

            let mut hunk = hunk;
            let mut taken_old = 0_u32;
            let mut taken_new = 0_u32;
            while index < lines.len() {
                let body = lines[index];
                let needs_more = taken_old < hunk.old_lines || taken_new < hunk.new_lines;
                if !needs_more {
                    break;
                }
                let kind = match body.as_bytes().first() {
                    Some(b' ') => DiffLineKind::Context,
                    Some(b'+') => DiffLineKind::Added,
                    Some(b'-') => DiffLineKind::Removed,
                    Some(b'\\') => DiffLineKind::NoNewlineMarker,
                    _ => break, // 不是合法的 hunk 体行（例如被截断的输出）
                };

                let old_lineno = match kind {
                    DiffLineKind::Context | DiffLineKind::Removed => {
                        taken_old += 1;
                        Some(hunk.old_start + taken_old - 1)
                    }
                    _ => None,
                };
                let new_lineno = match kind {
                    DiffLineKind::Context | DiffLineKind::Added => {
                        taken_new += 1;
                        Some(hunk.new_start + taken_new - 1)
                    }
                    _ => None,
                };
                hunk.lines.push(DiffLine {
                    kind,
                    content: body[1..].to_owned(),
                    old_lineno,
                    new_lineno,
                });
                index += 1;

                consumed_lines += 1;
                if !force_full && consumed_lines > limits.max_lines {
                    section.truncated = true;
                    break;
                }
            }
            // git 会在计数耗尽后仍输出结尾的 `\ No newline` 标记
            //（两个文件都没有末尾换行时标记出现两次），把它一并收进本 hunk。
            // 标记只可能跟着本 hunk 的内容行（下一个 hunk 以 @@ 开头、下一个文件
            // 以 diff --git 开头），因此贪婪消费是安全的。
            while index < lines.len() && lines[index].starts_with('\\') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::NoNewlineMarker,
                    content: lines[index][1..].to_owned(),
                    old_lineno: None,
                    new_lineno: None,
                });
                index += 1;
            }
            section.hunks.push(hunk);

            if section.truncated {
                // 跳过该文件剩余内容，直到下一个文件段
                while index < lines.len() && !lines[index].starts_with("diff --git ") {
                    index += 1;
                }
                break;
            }
        }

        sections.push(section);
    }

    sections
}

/// 解析 `@@ -a[,b] +c[,d] @@ header`。
///
/// git 在"���数为一"时省略计数（`@@ -1 +1 @@`），空文件用 `0,0`。
fn parse_hunk_header(line: &str) -> Option<DiffHunk> {
    let rest = line.strip_prefix("@@ ")?;
    let (range_part, header) = rest.split_once(" @@")?;
    let (old_part, new_part) = range_part.split_once(' ')?;
    let old_range = old_part.strip_prefix('-')?;
    let new_range = new_part.strip_prefix('+')?;

    let (old_start, old_lines) = parse_range(old_range)?;
    let (new_start, new_lines) = parse_range(new_range)?;

    Some(DiffHunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        header: header.trim_start().to_owned(),
        lines: Vec::new(),
    })
}

/// 解析 `a` 或 `a,b`；b 缺省为 1。
fn parse_range(range: &str) -> Option<(u32, u32)> {
    match range.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}

/// 去掉 `a/` / `b/` 前缀；若路径带 C 引号则**先解引再剥前缀**
/// （引号包住的是"前缀+路径"整体：`"b/with \"quote\".txt"`）。
fn strip_prefix_and_unquote(path: &str, prefix: &str) -> String {
    let trimmed = path.trim();
    let inner = if trimmed.starts_with('"') {
        unquote(trimmed)
    } else {
        trimmed.to_owned()
    };
    inner.strip_prefix(prefix).unwrap_or(&inner).to_owned()
}

/// git 的 C 风格引号解码（`\"` → `"`、`\\` → `\`、`\t`/`\n` 等常见转义）。
///
/// 只处理路径里真实会出现的转义；不需要完整的 C 词法器。
fn unquote(path: &str) -> String {
    let trimmed = path.trim();
    if !(trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2) {
        return trimmed.to_owned();
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::git::DiffLineKind;

    use super::{parse_unified_diff, PatchLimits};

    const SAMPLE: &str = concat!(
        "diff --git a/src/app.ts b/src/app.ts\n",
        "index 1111111..2222222 100644\n",
        "--- a/src/app.ts\n",
        "+++ b/src/app.ts\n",
        "@@ -1,4 +1,5 @@ fn main() {\n",
        " context\n",
        "-old line\n",
        "+new line\n",
        "+added line\n",
        " context2\n",
        "diff --git a/deleted.txt b/deleted.txt\n",
        "deleted file mode 100644\n",
        "index 3333333..0000000\n",
        "--- a/deleted.txt\n",
        "+++ /dev/null\n",
        "@@ -1,2 +0,0 @@\n",
        "-bye\n",
        "-bye2\n",
        "diff --git a/bin.dat b/bin.dat\n",
        "index 4444444..5555555 100644\n",
        "Binary files a/bin.dat and b/bin.dat differ\n",
    );

    #[test]
    fn parses_hunks_with_line_numbers_in_file_order() {
        let sections = parse_unified_diff(SAMPLE.as_bytes(), &PatchLimits::defaults(), false);

        assert_eq!(sections.len(), 3);
        assert_eq!(
            sections[0].path.as_ref().map(|p| p.to_string()),
            Some("src/app.ts".to_owned())
        );
        assert!(!sections[0].binary);
        assert_eq!(sections[0].hunks.len(), 1);

        let hunk = &sections[0].hunks[0];
        assert_eq!((hunk.old_start, hunk.old_lines), (1, 4));
        assert_eq!((hunk.new_start, hunk.new_lines), (1, 5));
        assert_eq!(hunk.header, "fn main() {");
        assert_eq!(hunk.lines.len(), 5);
        assert_eq!(hunk.lines[0].kind, DiffLineKind::Context);
        assert_eq!(hunk.lines[0].old_lineno, Some(1));
        assert_eq!(hunk.lines[0].new_lineno, Some(1));
        assert_eq!(hunk.lines[1].kind, DiffLineKind::Removed);
        assert_eq!(hunk.lines[1].old_lineno, Some(2));
        assert_eq!(hunk.lines[1].new_lineno, None);
        assert_eq!(hunk.lines[2].content, "new line");
        assert_eq!(hunk.lines[2].old_lineno, None);
        assert_eq!(hunk.lines[2].new_lineno, Some(2));
    }

    #[test]
    fn deleted_files_keep_the_old_path_and_only_removed_lines() {
        let sections = parse_unified_diff(SAMPLE.as_bytes(), &PatchLimits::defaults(), false);

        let deleted = &sections[1];
        assert_eq!(
            deleted.path.as_ref().map(|p| p.to_string()),
            Some("deleted.txt".to_owned())
        );
        assert_eq!(deleted.hunks.len(), 1);
        assert!(deleted.hunks[0]
            .lines
            .iter()
            .all(|l| l.kind == DiffLineKind::Removed));
        assert_eq!(deleted.hunks[0].lines.len(), 2);
    }

    #[test]
    fn binary_sections_carry_no_hunks() {
        let sections = parse_unified_diff(SAMPLE.as_bytes(), &PatchLimits::defaults(), false);

        assert!(sections[2].binary);
        assert!(sections[2].hunks.is_empty());
        assert!(!sections[2].truncated);
    }
}

#[test]
fn content_looking_like_a_header_does_not_split_the_hunk() {
    // 删除了一行 "--- 危险内容"：hunk 体里的 `---` 不能被当成新文件的头
    let patch = concat!(
        "diff --git a/tricky.txt b/tricky.txt\n",
        "--- a/tricky.txt\n",
        "+++ b/tricky.txt\n",
        "@@ -1,2 +1,2 @@\n",
        "--- 危险内容\n",
        "+safe now\n",
    );
    let sections = parse_unified_diff(patch.as_bytes(), &PatchLimits::defaults(), false);

    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].hunks.len(), 1);
    assert_eq!(sections[0].hunks[0].lines.len(), 2);
    assert_eq!(sections[0].hunks[0].lines[0].content, "-- 危险内容");
}

#[test]
fn single_line_hunk_headers_without_counts_are_accepted() {
    let patch = concat!(
        "diff --git a/one.txt b/one.txt\n",
        "--- a/one.txt\n",
        "+++ b/one.txt\n",
        "@@ -2 +2 @@\n",
        "-old\n",
        "+new\n",
    );
    let sections = parse_unified_diff(patch.as_bytes(), &PatchLimits::defaults(), false);

    let hunk = &sections[0].hunks[0];
    assert_eq!((hunk.old_start, hunk.old_lines), (2, 1));
    assert_eq!((hunk.new_start, hunk.new_lines), (2, 1));
}

#[test]
fn no_newline_markers_do_not_consume_line_counts() {
    let patch = concat!(
        "diff --git a/tail.txt b/tail.txt\n",
        "--- a/tail.txt\n",
        "+++ b/tail.txt\n",
        "@@ -1 +1 @@\n",
        "-old without newline\n",
        "\\ No newline at end of file\n",
        "+new without newline\n",
        "\\ No newline at end of file\n",
    );
    let sections = parse_unified_diff(patch.as_bytes(), &PatchLimits::defaults(), false);

    let lines = &sections[0].hunks[0].lines;
    assert_eq!(lines.len(), 4, "标记行应被保留为 NoNewlineMarker");
    assert_eq!(lines[1].kind, DiffLineKind::NoNewlineMarker);
    assert_eq!(lines[1].old_lineno, None, "标记不占行号");
    assert_eq!(lines[3].kind, DiffLineKind::NoNewlineMarker);
}

#[test]
fn oversized_sections_are_truncated_and_the_rest_belongs_to_the_next_file() {
    let patch = concat!(
        "diff --git a/big.txt b/big.txt\n",
        "--- a/big.txt\n",
        "+++ b/big.txt\n",
        "@@ -1,3 +1,3 @@\n",
        "-a\n",
        "-b\n",
        "-c\n",
        "diff --git a/after.txt b/after.txt\n",
        "--- /dev/null\n",
        "+++ b/after.txt\n",
        "@@ -0,0 +1,1 @@\n",
        "+tiny\n",
    );
    let limits = PatchLimits {
        max_lines: 2,
        max_bytes: usize::MAX,
    };
    let sections = parse_unified_diff(patch.as_bytes(), &limits, false);

    assert_eq!(sections.len(), 2);
    assert!(sections[0].truncated);
    assert!(!sections[1].truncated);
    assert_eq!(sections[1].hunks[0].lines[0].content, "tiny");
}

#[test]
fn force_full_ignores_the_budget() {
    let patch = concat!(
        "diff --git a/big.txt b/big.txt\n",
        "--- a/big.txt\n",
        "+++ b/big.txt\n",
        "@@ -1,3 +1,3 @@\n",
        "-a\n",
        "-b\n",
        "-c\n",
    );
    let limits = PatchLimits {
        max_lines: 2,
        max_bytes: usize::MAX,
    };
    let sections = parse_unified_diff(patch.as_bytes(), &limits, true);

    assert!(!sections[0].truncated);
    assert_eq!(sections[0].hunks[0].lines.len(), 3);
}

#[test]
fn quoted_paths_are_unescaped() {
    let patch = concat!(
        "diff --git \"a/with \\\"quote\\\".txt\" \"b/with \\\"quote\\\".txt\"\n",
        "--- \"a/with \\\"quote\\\".txt\"\n",
        "+++ \"b/with \\\"quote\\\".txt\"\n",
        "@@ -1 +1 @@\n",
        "-x\n",
        "+y\n",
    );
    let sections = parse_unified_diff(patch.as_bytes(), &PatchLimits::defaults(), false);

    assert_eq!(
        sections[0].path.as_ref().map(|p| p.to_string()),
        Some("with \"quote\".txt".to_owned())
    );
}

#[test]
fn git_binary_patch_payload_is_skipped() {
    let patch = concat!(
        "diff --git a/blob.bin b/blob.bin\n",
        "index 111..222 100644\n",
        "GIT binary patch\n",
        "literal 10\n",
        "ZcmV-dCGs2f00123456789\n",
        "\n",
        "diff --git a/next.txt b/next.txt\n",
        "--- a/next.txt\n",
        "+++ b/next.txt\n",
        "@@ -1 +1 @@\n",
        "-a\n",
        "+b\n",
    );
    let sections = parse_unified_diff(patch.as_bytes(), &PatchLimits::defaults(), false);

    assert_eq!(sections.len(), 2);
    assert!(sections[0].binary);
    assert!(sections[0].hunks.is_empty());
    assert!(!sections[1].binary);
}
