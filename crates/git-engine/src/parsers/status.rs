//! `git status --porcelain=v2 -z --branch` 的解析器。
//!
//! # 为什么是 porcelain v2 + `-z`
//!
//! - `v2` 才有 `XY` 双状态位（索引侧 / 工作区侧）与每个条目的模式、oid，
//!   这正是状态面板分组与"行级暂存"按钮的判定依据；
//! - `-z` 才有**不做引号转义**的路径。默认输出会对含空格/非 ASCII 的路径加引号并
//!   用 C 风格转义，解析它等于自己实现一遍 unquote，而 `-z` 直接给原始字节。
//!
//! **调用方必须传 `-z`**：本解析器按"NUL 结束记录、重命名条目紧跟第二个 NUL 字段"
//! 的约定工作。传非 `-z` 输出不会报错，但重命名条目会解析出错误的路径。
//!
//! # 格式（实测 git 2.54）
//!
//! ```text
//! # branch.oid <oid> | (initial)              \0
//! # branch.head <branch> | (detached)         \0
//! # branch.upstream <upstream>                \0
//! # branch.ab +<ahead> -<behind>              \0
//! 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>\0
//! 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>\0<origPath>\0
//! u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>\0
//! ? <path>\0
//! ! <path>\0
//! ```
//!
//! 注意：`-z` 模式下**头记录同样是 NUL 结束**（不是换行）。这一点容易记错，
//! 按换行切分会把整个输出当成一条记录。

use forgedesk_domain::git::{
    BranchInfo, ChangeKind, ConflictStages, EntryKind, FileChange, RepoPath, StageEntry,
    StatusReport, SubmoduleState,
};

use super::common::{find_byte, parse_ahead_behind, parse_mode, parse_oid, skip_record_separators};

/// 解析 `git status --porcelain=v2 -z --branch` 的输出。
///
/// 无法识别的记录会被**跳过**（不 panic、不中断）：Git 未来版本新增记录类型时，
/// 界面应该少显示一条，而不是整块空白。
pub fn parse_status_porcelain_v2(input: &[u8]) -> StatusReport {
    let mut report = StatusReport::default();
    let mut cursor = skip_record_separators(input, 0);

    while cursor < input.len() {
        if input[cursor] == b'#' {
            let end = find_record_end(input, cursor);
            parse_header(&input[cursor..end], &mut report.branch);
            cursor = skip_record_separators(input, end);
            continue;
        }

        let prefix = input[cursor];
        let end = find_byte(input, cursor, 0).unwrap_or(input.len());
        let record = &input[cursor..end];
        let next = if end < input.len() { end + 1 } else { end };

        // 重命名/复制条目的来源路径是紧随其后的第二个 NUL 字段
        let (original_path, after_entry) = if prefix == b'2' {
            let original_end = find_byte(input, next, 0).unwrap_or(input.len());
            let original = &input[next..original_end];
            let after = if original_end < input.len() {
                original_end + 1
            } else {
                original_end
            };
            (Some(original), after)
        } else {
            (None, next)
        };

        if let Some(change) = parse_entry(prefix, record, original_path) {
            report.entries.push(change);
        }
        cursor = skip_record_separators(input, after_entry);
    }

    report
}

/// 头记录（`# ...`）的结束位置（分隔符所在下标，或输入末尾）。
fn find_record_end(input: &[u8], from: usize) -> usize {
    let mut cursor = from;
    while let Some(byte) = input.get(cursor) {
        if matches!(*byte, 0 | b'\n' | b'\r') {
            break;
        }
        cursor += 1;
    }
    cursor
}

/// 解析 `# branch.*` 头。
///
/// 分支名与上游名走 lossy：它们只用于展示与 i18n 文案，不参与文件系统操作。
/// 需要精确 ref 名的场景（分支管理）在 M2 走 `git branch --format`，
/// 那里同样是字节级解析。
fn parse_header(record: &[u8], branch: &mut BranchInfo) {
    let Some(rest) = record.strip_prefix(b"# ") else {
        return;
    };
    let mut parts = rest.splitn(2, |byte| *byte == b' ');
    let Some(key) = parts.next() else {
        return;
    };
    let value = parts.next().unwrap_or(b"");

    match key {
        b"branch.oid" => {
            // `(initial)` 表示还没有任何提交，与"oid 解析失败"是两件事：
            // 前者要引导首次提交，后者要报错。这里用 None 表达"没有提交"。
            branch.oid = if value == b"(initial)" {
                None
            } else {
                parse_oid(value)
            };
        }
        b"branch.head" => {
            if value == b"(detached)" {
                branch.detached = true;
                branch.head = None;
            } else {
                branch.head = Some(String::from_utf8_lossy(value).into_owned());
            }
        }
        b"branch.upstream" => {
            branch.upstream = Some(String::from_utf8_lossy(value).into_owned());
        }
        b"branch.ab" => {
            if let Some((ahead, behind)) = parse_ahead_behind(value) {
                branch.ahead = Some(ahead);
                branch.behind = Some(behind);
            }
        }
        _ => {}
    }
}

/// 解析一条变更记录。
fn parse_entry(prefix: u8, record: &[u8], original_path: Option<&[u8]>) -> Option<FileChange> {
    let kind = EntryKind::from_prefix(prefix)?;

    // `?` / `!` 只有"前缀 + 路径"，没有模式与 oid
    if matches!(kind, EntryKind::Untracked | EntryKind::Ignored) {
        let path = record.splitn(2, |byte| *byte == b' ').nth(1)?;
        if path.is_empty() {
            return None;
        }
        return Some(FileChange {
            kind,
            path: RepoPath::from_bytes(path.to_vec()),
            original_path: None,
            index_status: ChangeKind::Unmodified,
            worktree_status: ChangeKind::Unmodified,
            similarity: None,
            mode_head: None,
            mode_index: None,
            mode_worktree: None,
            oid_head: None,
            oid_index: None,
            stages: None,
            submodule: SubmoduleState::NONE,
            // 二进制/LFS/大小由引擎在解析后富化（解析层只认 porcelain 输出）
            is_binary: false,
            is_lfs: false,
            size_bytes: None,
        });
    }

    let field_count = match kind {
        EntryKind::Ordinary => 9,
        EntryKind::RenamedOrCopied => 10,
        EntryKind::Unmerged => 11,
        EntryKind::Untracked | EntryKind::Ignored => unreachable!("已在上方提前返回"),
    };
    // splitn 让最后一段吃掉剩余内容，因此路径里的空格不会被切开
    let fields: Vec<&[u8]> = record.splitn(field_count, |byte| *byte == b' ').collect();
    if fields.len() != field_count {
        tracing::debug!(
            fields = fields.len(),
            expected = field_count,
            "porcelain v2 记录字段数不符，已跳过"
        );
        return None;
    }

    let status_pair = fields[1];
    if status_pair.len() != 2 {
        tracing::debug!("porcelain v2 的 XY 字段长度不为 2，已跳过");
        return None;
    }
    let index_status = ChangeKind::from_byte(status_pair[0]);
    let worktree_status = ChangeKind::from_byte(status_pair[1]);
    let submodule = SubmoduleState::parse(fields[2]);

    let mut change = FileChange {
        kind,
        path: RepoPath::from_bytes(Vec::new()),
        original_path: original_path.map(|path| RepoPath::from_bytes(path.to_vec())),
        index_status,
        worktree_status,
        similarity: None,
        mode_head: None,
        mode_index: None,
        mode_worktree: None,
        oid_head: None,
        oid_index: None,
        stages: None,
        submodule,
        is_binary: false,
        is_lfs: false,
        size_bytes: None,
    };

    match kind {
        EntryKind::Ordinary => {
            change.mode_head = parse_mode(fields[3]);
            change.mode_index = parse_mode(fields[4]);
            change.mode_worktree = parse_mode(fields[5]);
            change.oid_head = parse_oid(fields[6]);
            change.oid_index = parse_oid(fields[7]);
            change.path = RepoPath::from_bytes(fields[8].to_vec());
        }
        EntryKind::RenamedOrCopied => {
            change.mode_head = parse_mode(fields[3]);
            change.mode_index = parse_mode(fields[4]);
            change.mode_worktree = parse_mode(fields[5]);
            change.oid_head = parse_oid(fields[6]);
            change.oid_index = parse_oid(fields[7]);
            change.similarity = parse_similarity(fields[8]);
            change.path = RepoPath::from_bytes(fields[9].to_vec());
        }
        EntryKind::Unmerged => {
            // 冲突条目的 mW 是工作区模式；三个 stage 的模式与 oid 单独收进 stages
            change.mode_worktree = parse_mode(fields[6]);
            change.stages = Some(ConflictStages {
                base: stage_entry(fields[3], fields[7]),
                ours: stage_entry(fields[4], fields[8]),
                theirs: stage_entry(fields[5], fields[9]),
            });
            change.path = RepoPath::from_bytes(fields[10].to_vec());
        }
        EntryKind::Untracked | EntryKind::Ignored => unreachable!("已在上方提前返回"),
    }

    // 空路径说明记录被截断或格式不符：git 不会输出空路径
    if change.path.as_bytes().is_empty() {
        return None;
    }

    Some(change)
}

/// 解析重命名/复制条目的 `<X><score>` 字段（如 `R100`、`C087`）。
///
/// 只取数字部分：`X` 与 XY 字段的 `X` 重复，以 XY 为准，避免两个来源不一致时
/// 界面显示的状态与实际状态对不上。
fn parse_similarity(field: &[u8]) -> Option<u8> {
    let (_, score) = field.split_first()?;
    std::str::from_utf8(score).ok()?.parse::<u8>().ok()
}

/// 组合一个 stage 记录；模式或 oid 缺失（全零）时返回 `None`。
///
/// 为什么两个都要有：删除/修改类冲突只有 base 与 ours，
/// 此时 stage 3 的模式与 oid 都是零——那是"没有这个版本"，不是"模式为 0 的版本"。
fn stage_entry(mode: &[u8], oid: &[u8]) -> Option<StageEntry> {
    Some(StageEntry {
        mode: parse_mode(mode)?,
        oid: parse_oid(oid)?,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::parse_status_porcelain_v2;
    use forgedesk_domain::git::{ChangeKind, EntryKind};

    #[test]
    fn empty_input_yields_an_empty_report() {
        let report = parse_status_porcelain_v2(b"");

        assert!(report.is_clean());
        assert_eq!(report.branch.oid, None);
        assert!(!report.branch.detached);
        assert!(report.branch.is_initial());
    }

    #[test]
    fn initial_repository_is_distinguished_from_detached_head() {
        let initial = parse_status_porcelain_v2(b"# branch.oid (initial)\0# branch.head main\0");
        let detached = parse_status_porcelain_v2(
            b"# branch.oid 1f14e032be7bd5478594b90bd85e172d2d8fc371\0# branch.head (detached)\0",
        );

        assert!(initial.branch.is_initial());
        assert_eq!(initial.branch.head.as_deref(), Some("main"));

        assert!(!detached.branch.is_initial());
        assert!(detached.branch.detached);
        assert_eq!(detached.branch.head, None);
    }

    #[test]
    fn rename_record_reads_the_second_nul_field_as_the_source_path() {
        let input = b"2 R. N... 100644 100644 100644 b2f931a67315c95c5daab3aac6de62e534808476 b2f931a67315c95c5daab3aac6de62e534808476 R100 renamed.txt\0a.txt\0";
        let report = parse_status_porcelain_v2(input);

        assert_eq!(report.entries.len(), 1);
        let entry = &report.entries[0];
        assert_eq!(entry.kind, EntryKind::RenamedOrCopied);
        assert_eq!(entry.path.to_string(), "renamed.txt");
        assert_eq!(
            entry.original_path.as_ref().map(|path| path.to_string()),
            Some("a.txt".to_owned())
        );
        assert_eq!(entry.similarity, Some(100));
    }

    #[test]
    fn unmerged_record_keeps_three_stages_with_their_oids() {
        let input = b"u UU N... 100644 100644 100644 100644 df967b96a579e45a18b8251732d16804b2e56a55 ba2906d0666cf726c7eaadd2cd3db615dedfdf3a e45c9c2666d44e0327c1f9c239a74c508336053e c.txt\0";
        let report = parse_status_porcelain_v2(input);

        let entry = &report.entries[0];
        assert!(entry.is_conflicted());
        let stages = entry.stages.as_ref().unwrap();
        assert_eq!(stages.base.as_ref().unwrap().mode, 0o100644);
        assert_eq!(
            stages.ours.as_ref().unwrap().oid,
            "ba2906d0666cf726c7eaadd2cd3db615dedfdf3a"
        );
        assert_eq!(
            stages.theirs.as_ref().unwrap().oid,
            "e45c9c2666d44e0327c1f9c239a74c508336053e"
        );
        assert_eq!(entry.index_status, ChangeKind::Unmerged);
        assert_eq!(entry.worktree_status, ChangeKind::Unmerged);
    }

    #[test]
    fn deleted_by_them_conflict_has_no_theirs_stage() {
        // 删除/修改类冲突只有 base 与 ours，stage 3 的模式与 oid 全是零
        let input = b"u DU N... 100644 100644 000000 000000 7ee84ceb10f09d5b57b94ac7d1e091a12e0d60e6 1d4e73e595740cc8377762fccd3290ea99b52f00 0000000000000000000000000000000000000000 d.txt\0";
        let report = parse_status_porcelain_v2(input);

        let stages = report.entries[0].stages.as_ref().unwrap();
        assert!(stages.base.is_some());
        assert!(stages.ours.is_some());
        assert!(stages.theirs.is_none());
    }

    #[test]
    fn untracked_and_ignored_records_carry_no_index_details() {
        let report = parse_status_porcelain_v2(b"? untracked.txt\0! build/\0");

        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.entries[0].kind, EntryKind::Untracked);
        assert_eq!(report.entries[0].path.to_string(), "untracked.txt");
        assert_eq!(report.entries[0].mode_index, None);
        assert_eq!(report.entries[1].kind, EntryKind::Ignored);
        assert_eq!(report.entries[1].path.to_string(), "build/");
    }

    #[test]
    fn unknown_record_prefix_is_skipped_without_losing_the_rest() {
        let input = b"X something new\0# branch.head main\0? ok.txt\0";
        let report = parse_status_porcelain_v2(input);

        assert_eq!(report.branch.head.as_deref(), Some("main"));
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].path.to_string(), "ok.txt");
    }

    #[test]
    fn truncated_record_is_skipped_instead_of_panicking() {
        // 缺字段（只有前缀与 XY）
        let report = parse_status_porcelain_v2(b"1 .M\0");

        assert!(report.is_clean());
    }

    #[test]
    fn malformed_xy_field_is_skipped() {
        let report = parse_status_porcelain_v2(b"1 M N... 100644 100644 100644 a a f.txt\0");

        assert!(report.is_clean());
    }

    #[test]
    fn header_without_value_does_not_break_the_report() {
        let report = parse_status_porcelain_v2(b"# branch.head\0# branch.ab\0? a.txt\0");

        assert_eq!(report.branch.head, Some(String::new()));
        assert_eq!(report.branch.ahead, None);
        assert_eq!(report.entries.len(), 1);
    }

    #[test]
    fn paths_keep_their_raw_bytes() {
        // 0xFF 不是合法 UTF-8 起始字节
        let mut input = b"? ".to_vec();
        input.extend_from_slice(&[0xFF, 0xFE, b'.', b't', b'x', b't', 0]);
        let report = parse_status_porcelain_v2(&input);

        assert_eq!(report.entries.len(), 1);
        assert_eq!(
            report.entries[0].path.as_bytes(),
            &[0xFF, 0xFE, b'.', b't', b'x', b't']
        );
        assert!(!report.entries[0].path.is_utf8());
    }
}
