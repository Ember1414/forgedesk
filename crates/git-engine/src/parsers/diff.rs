//! `git diff --numstat -z` 的解析器。
//!
//! # 格式（实测 git 2.54）
//!
//! ```text
//! <add>\t<del>\t<path>\0
//! <add>\t<del>\t\0<oldPath>\0<newPath>\0     # 重命名 / 复制
//! -\t-\t<path>\0                              # 二进制文件
//! ```
//!
//! 三个容易踩的点：
//!
//! 1. **重命名条目的路径字段以 NUL 开头**：即第三个制表符后面紧跟的是一个空字段，
//!    然后才是"来源路径 NUL 目标路径 NUL"。按"第三段就是路径"解析会把来源路径
//!    读成空字符串，而目标路径变成下一条记录的垃圾输入。
//! 2. **顺序是"来源在前、目标在后"**（与 `git diff` 人类可读输出里的
//!    `old => new` 一致），不是反过来的。
//! 3. **二进制是 `-` 而不是 `0`**：`0\t0` 表示"内容变了但行数没变"（如模式变更），
//!    把它当成二进制会让界面显示错误的分组。

use forgedesk_domain::git::{FileStat, RepoPath};

use super::common::{find_byte, skip_record_separators};

/// 解析 `git diff --numstat -z` 的输出。
///
/// 畸形记录（缺字段、被截断的管道）会被跳过，不 panic。
pub fn parse_diff_numstat(input: &[u8]) -> Vec<FileStat> {
    let mut stats = Vec::new();
    let mut cursor = skip_record_separators(input, 0);

    while cursor < input.len() {
        let Some(added_end) = find_byte(input, cursor, b'\t') else {
            break;
        };
        let Some(deleted_end) = find_byte(input, added_end + 1, b'\t') else {
            break;
        };

        let additions = parse_count(&input[cursor..added_end]);
        let deletions = parse_count(&input[added_end + 1..deleted_end]);
        let third_field_start = deleted_end + 1;

        // 重命名 / 复制：第三个字段是空的，随后是"来源 NUL 目标 NUL"
        let (original_path, path, next) = if input.get(third_field_start) == Some(&0) {
            let old_start = third_field_start + 1;
            let Some(old_end) = find_byte(input, old_start, 0) else {
                break;
            };
            let new_start = old_end + 1;
            let new_end = find_byte(input, new_start, 0).unwrap_or(input.len());
            let next = if new_end < input.len() {
                new_end + 1
            } else {
                new_end
            };
            (
                Some(RepoPath::from_bytes(input[old_start..old_end].to_vec())),
                RepoPath::from_bytes(input[new_start..new_end].to_vec()),
                next,
            )
        } else {
            let end = find_byte(input, third_field_start, 0).unwrap_or(input.len());
            let next = if end < input.len() { end + 1 } else { end };
            (
                None,
                RepoPath::from_bytes(input[third_field_start..end].to_vec()),
                next,
            )
        };

        // 空路径说明记录被截断或格式不符：git 不会输出空路径，
        // 放它过去会让界面上出现一个"点不动的幽灵文件"
        if path.as_bytes().is_empty() {
            break;
        }

        stats.push(FileStat {
            path,
            original_path,
            additions,
            deletions,
            // 二进制由 git 用 `-` 标记；两侧都不是数字才算二进制
            binary: additions.is_none() && deletions.is_none(),
        });

        cursor = skip_record_separators(input, next);
    }

    stats
}

/// 解析增删行数。`-`（二进制）或畸形内容返回 `None`。
fn parse_count(field: &[u8]) -> Option<u64> {
    std::str::from_utf8(field).ok()?.parse::<u64>().ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::parse_diff_numstat;

    #[test]
    fn empty_input_yields_no_stats() {
        assert!(parse_diff_numstat(b"").is_empty());
    }

    #[test]
    fn plain_records_are_parsed_with_their_counts() {
        let stats = parse_diff_numstat(b"2\t1\ta.txt\x000\t1\tb.txt\0");

        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].path.to_string(), "a.txt");
        assert_eq!(stats[0].additions, Some(2));
        assert_eq!(stats[0].deletions, Some(1));
        assert!(!stats[0].binary);
        assert!(!stats[0].is_rename_or_copy());
        assert_eq!(stats[1].path.to_string(), "b.txt");
        assert_eq!(stats[1].additions, Some(0));
    }

    #[test]
    fn rename_records_take_the_source_path_from_the_leading_empty_field() {
        let stats = parse_diff_numstat(b"1\t0\t\0old name.txt\0new name.txt\0");

        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].path.to_string(), "new name.txt");
        assert_eq!(
            stats[0].original_path.as_ref().map(|path| path.to_string()),
            Some("old name.txt".to_owned())
        );
        assert_eq!(stats[0].changed_lines(), Some(1));
    }

    #[test]
    fn binary_records_are_marked_without_fake_line_counts() {
        let stats = parse_diff_numstat(b"-\t-\tbin.dat\0");

        assert_eq!(stats.len(), 1);
        assert!(stats[0].binary);
        assert_eq!(stats[0].additions, None);
        assert_eq!(stats[0].deletions, None);
        assert_eq!(stats[0].changed_lines(), None);
    }

    #[test]
    fn zero_zero_counts_are_not_treated_as_binary() {
        // 模式变更 / 纯重命名会给出 0 0，它不是二进制
        let stats = parse_diff_numstat(b"0\t0\tmode.txt\0");

        assert!(!stats[0].binary);
        assert_eq!(stats[0].changed_lines(), Some(0));
    }

    #[test]
    fn paths_keep_spaces_and_raw_bytes() {
        let mut input = b"1\t1\t".to_vec();
        input.extend_from_slice(&[0xFF, b' ', b'a', b'.', b't', b'x', b't', 0]);
        let stats = parse_diff_numstat(&input);

        assert_eq!(stats.len(), 1);
        assert_eq!(
            stats[0].path.as_bytes(),
            &[0xFF, b' ', b'a', b'.', b't', b'x', b't']
        );
        assert!(!stats[0].path.is_utf8());
    }

    #[test]
    fn truncated_input_stops_without_panicking() {
        assert!(parse_diff_numstat(b"1\t").is_empty());
        assert!(parse_diff_numstat(b"1\t0\t").is_empty());
        assert!(parse_diff_numstat(b"1\t0\t\0old.txt\0").is_empty());
    }
}
