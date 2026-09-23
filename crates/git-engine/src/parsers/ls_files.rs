//! `git ls-files -u -z` 的解析器（未解决的冲突条目）。
//!
//! # 为什么需要它（M3 的提前准备）
//!
//! M3 的冲突解析器刻意**不读工作区里的冲突标记**（`<<<<<<<`）：标记符会被用户的
//! 编辑器或格式化工具改动，且无法表达"删除/修改"这类没有标记的冲突。
//! 权威来源是**索引里的三个 stage**，而 `git ls-files -u` 正是它的机器可读导出。
//!
//! # 格式（实测 git 2.54）
//!
//! ```text
//! <mode> <oid> <stage>\t<path>\0
//! ```
//!
//! 注意分隔符的混用：模式/oid/stage 之间是**空格**，stage 与路径之间是**制表符**，
//! 记录之间是 **NUL**。用空格切分整条记录会把含空格的路径切碎。
//!
//! 同一个文件会出现 0–3 条记录（缺 stage 是正常的：删除/修改类冲突只有两个 stage），
//! 因此本解析器返回的是**扁平列表**，按文件聚合由调用方决定
//! （`forgedesk_domain::git::status::ConflictStages` 是聚合后的形态）。

use forgedesk_domain::git::{RepoPath, UnmergedEntry, UnmergedStage};

use super::common::{parse_decimal, parse_mode, parse_oid, trim_record_separators};

/// 解析 `git ls-files -u -z` 的输出。
///
/// 畸形记录（缺字段、stage 越界、模式或 oid 全零）会被跳过，不 panic。
pub fn parse_ls_files_stage(input: &[u8]) -> Vec<UnmergedEntry> {
    input
        .split(|byte| *byte == 0)
        .filter_map(parse_record)
        .collect()
}

/// 解析单条记录：`<mode> <oid> <stage>\t<path>`。
fn parse_record(record: &[u8]) -> Option<UnmergedEntry> {
    let record = trim_record_separators(record);
    if record.is_empty() {
        return None;
    }

    // 用**第一个**制表符切分：路径本身可以含制表符（Git 允许）
    let tab = record.iter().position(|byte| *byte == b'\t')?;
    let meta = &record[..tab];
    let path = &record[tab + 1..];
    // 空路径说明记录被截断或格式不符：git 不会输出空路径
    if path.is_empty() {
        return None;
    }

    let mut parts = meta.splitn(3, |byte| *byte == b' ');
    let mode = parse_mode(parts.next()?)?;
    let oid = parse_oid(parts.next()?)?;
    let stage = u32::try_from(parse_decimal(parts.next()?)?).ok()?;
    let stage = UnmergedStage::from_number(stage)?;

    Some(UnmergedEntry {
        path: RepoPath::from_bytes(path.to_vec()),
        stage,
        mode,
        oid,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::parse_ls_files_stage;
    use forgedesk_domain::git::UnmergedStage;

    #[test]
    fn empty_input_yields_no_entries() {
        assert!(parse_ls_files_stage(b"").is_empty());
    }

    #[test]
    fn three_stages_of_one_file_are_parsed_with_their_oids() {
        let input = b"100644 df967b96a579e45a18b8251732d16804b2e56a55 1\tc.txt\x00100644 ba2906d0666cf726c7eaadd2cd3db615dedfdf3a 2\tc.txt\x00100644 e45c9c2666d44e0327c1f9c239a74c508336053e 3\tc.txt\0";
        let entries = parse_ls_files_stage(input);

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].stage, UnmergedStage::Base);
        assert_eq!(entries[1].stage, UnmergedStage::Ours);
        assert_eq!(entries[2].stage, UnmergedStage::Theirs);
        assert_eq!(entries[0].path.to_string(), "c.txt");
        assert_eq!(entries[0].mode, 0o100644);
        assert_eq!(entries[2].oid, "e45c9c2666d44e0327c1f9c239a74c508336053e");
    }

    #[test]
    fn missing_stage_is_not_invented() {
        // "删除/修改"冲突只有 stage 1 与 2
        let input = b"100644 7ee84ceb10f09d5b57b94ac7d1e091a12e0d60e6 1\td.txt\x00100644 1d4e73e595740cc8377762fccd3290ea99b52f00 2\td.txt\0";
        let entries = parse_ls_files_stage(input);

        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .all(|entry| entry.stage != UnmergedStage::Theirs));
    }

    #[test]
    fn paths_with_spaces_are_not_split() {
        let input = b"100644 df967b96a579e45a18b8251732d16804b2e56a55 1\ta b c.txt\0";
        let entries = parse_ls_files_stage(input);

        assert_eq!(entries[0].path.to_string(), "a b c.txt");
    }

    #[test]
    fn out_of_range_stage_number_is_skipped() {
        let input = b"100644 df967b96a579e45a18b8251732d16804b2e56a55 4\tc.txt\0";

        assert!(parse_ls_files_stage(input).is_empty());
    }

    #[test]
    fn zero_oid_and_zero_mode_are_skipped() {
        let zero_oid = b"100644 0000000000000000000000000000000000000000 1\tc.txt\0";
        let zero_mode = b"000000 df967b96a579e45a18b8251732d16804b2e56a55 1\tc.txt\0";

        assert!(parse_ls_files_stage(zero_oid).is_empty());
        assert!(parse_ls_files_stage(zero_mode).is_empty());
    }

    #[test]
    fn records_without_a_tab_are_skipped() {
        assert!(
            parse_ls_files_stage(b"100644 df967b96a579e45a18b8251732d16804b2e56a55 1\0").is_empty()
        );
    }

    #[test]
    fn raw_bytes_in_the_path_are_preserved() {
        let mut input = b"100644 df967b96a579e45a18b8251732d16804b2e56a55 1\t".to_vec();
        input.extend_from_slice(&[0xFF, b'.', b't', b'x', b't', 0]);
        let entries = parse_ls_files_stage(&input);

        assert_eq!(entries[0].path.as_bytes(), &[0xFF, b'.', b't', b'x', b't']);
    }
}
