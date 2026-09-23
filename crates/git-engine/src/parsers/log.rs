//! `git log --format=<LOG_FORMAT>` 的解析器。
//!
//! # 为什么用 `%x1f` / `%x1e` 而不是默认格式
//!
//! 默认的 `git log` 输出是给人看的：作者名与日期在一行、subject 在下一行、缩进与
//! 分隔符随版本变化，而且 subject 里可以出现任何字符（包括换行）。用分隔符把字段
//! 隔开之后，"解析"退化成"切分"，没有歧义。
//!
//! 选 `%x1f`（US，单元分隔符）与 `%x1e`（RS，记录分隔符）的理由：它们是 ASCII 控制字符，
//! 不可能出现在提交信息里（Git 的提交信息是文本），而 `|`、`\t`、`;` 这些常见分隔符
//! 都可能被用户写进 subject 或作者名。
//!
//! # 格式串与解析器必须成对演进
//!
//! [`LOG_FORMAT`] 是**唯一真相源**：调用方用它构造命令，解析器按它切分。少一个
//! `%x1f` 会让字段整体错位——表现为"作者名变成了邮箱"这类不会报错的错误，
//! 因此字段个数不符时只记 debug 日志并丢弃该记录，而不是尽力猜测。
//!
//! # 记录结束符
//!
//! `-z` 模式下 `%x1e` 后面跟 NUL，否则跟换行。两种都在首尾被裁掉，
//! 因此解析器对 `-z` 与否都成立。

use forgedesk_domain::git::{Commit, Signature, SignatureStatus};

/// `git log` 的机器可读格式串。
///
/// 字段顺序：`oid`、`parents`、`author_name`、`author_email`、`author_time`、
/// `committer_name`、`committer_email`、`committer_time`、`refs`、`signature`、`subject`。
///
/// `%s` 放在最后：它是唯一可能包含任意用户文本的字段，放最后可以保证即使
/// 它里面出现了 `%x1f`（理论上不可能，但不必赌）也只影响这一个字段。
pub const LOG_FORMAT: &str =
    "%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%D%x1f%G?%x1f%s%x1e";

/// 字段分隔符 `%x1f`。
const FIELD_SEPARATOR: u8 = 0x1f;
/// 记录分隔符 `%x1e`。
const RECORD_SEPARATOR: u8 = 0x1e;
/// [`LOG_FORMAT`] 声明的字段个数。
const FIELD_COUNT: usize = 11;

/// 解析 `git log --format=<LOG_FORMAT>` 的输出。
///
/// 字段个数不符的记录会被跳过（见模块头的说明）。
pub fn parse_log_format(input: &[u8]) -> Vec<Commit> {
    input
        .split(|byte| *byte == RECORD_SEPARATOR)
        .filter_map(parse_record)
        .collect()
}

/// 解析单条记录（已去掉记录分隔符残留）。
fn parse_record(record: &[u8]) -> Option<Commit> {
    let record = super::common::trim_record_separators(record);
    if record.is_empty() {
        return None;
    }

    let fields: Vec<&[u8]> = record.split(|byte| *byte == FIELD_SEPARATOR).collect();
    if fields.len() != FIELD_COUNT {
        tracing::debug!(
            fields = fields.len(),
            expected = FIELD_COUNT,
            "git log 记录字段数不符，已跳过；请检查 LOG_FORMAT 与解析器是否同步"
        );
        return None;
    }

    let oid = lossy(fields[0]);
    if oid.is_empty() {
        return None;
    }

    Some(Commit {
        oid,
        parents: split_parents(fields[1]),
        author: signature(fields[2], fields[3], fields[4]),
        committer: signature(fields[5], fields[6], fields[7]),
        refs: split_refs(fields[8]),
        signature: fields[9].first().map_or(SignatureStatus::Unknown, |byte| {
            SignatureStatus::from_byte(*byte)
        }),
        subject: lossy(fields[10]),
        // 列表页不带正文（`LOG_FORMAT` 不含 `%b`），单条查询由 show() 填充
        body: None,
    })
}

/// 组合作者 / 提交者身份。
fn signature(name: &[u8], email: &[u8], time: &[u8]) -> Signature {
    Signature {
        name: lossy(name),
        email: lossy(email),
        time: super::common::parse_decimal(time),
    }
}

/// 元数据字段的 lossy 转换（见 [`forgedesk_domain::git`] 模块头的取舍说明）。
fn lossy(field: &[u8]) -> String {
    String::from_utf8_lossy(field).into_owned()
}

/// 拆分父提交 oid（空格分隔）。根提交的字段为空 → 空列表。
fn split_parents(field: &[u8]) -> Vec<String> {
    field
        .split(|byte| *byte == b' ')
        .filter(|part| !part.is_empty())
        .map(lossy)
        .collect()
}

/// 拆分 `%D` 的引用列表（`, ` 分隔）。
///
/// 为什么 `, ` 是安全的分隔符：Git 的 ref 名禁止包含空格
/// （`git check-ref-format` 明确拒绝空格、`~`、`^`、`:`、`?`、`*`、`[`、`\`），
/// 因此 ref 名内部不可能出现 `, `。
fn split_refs(field: &[u8]) -> Vec<String> {
    let text = lossy(field);
    text.split(", ")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{parse_log_format, FIELD_SEPARATOR, LOG_FORMAT, RECORD_SEPARATOR};
    use forgedesk_domain::git::SignatureStatus;

    /// 按 [`LOG_FORMAT`] 的顺序拼一条记录，避免测试里手写一堆分隔符。
    fn record(fields: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                out.push(FIELD_SEPARATOR);
            }
            out.extend_from_slice(field.as_bytes());
        }
        out
    }

    fn full_record() -> Vec<u8> {
        record(&[
            "760ba1f4b59dfd7120abea5825030081ba94c12c",
            "63daff978b6cc470c01793c73fd055e32655f2a8",
            "Fixture Author",
            "author@example.com",
            "1704164645",
            "Fixture Committer",
            "committer@example.com",
            "1704164645",
            "HEAD -> main",
            "N",
            "third commit",
        ])
    }

    #[test]
    fn format_string_declares_exactly_the_fields_the_parser_expects() {
        // LOG_FORMAT 与 FIELD_COUNT 是成对的；这条断言防止只改一处
        assert_eq!(LOG_FORMAT.matches("%x1f").count() + 1, super::FIELD_COUNT);
        assert_eq!(LOG_FORMAT.matches("%x1e").count(), 1);
    }

    #[test]
    fn empty_input_yields_no_commits() {
        assert!(parse_log_format(b"").is_empty());
    }

    #[test]
    fn a_full_record_is_parsed_field_by_field() {
        let mut input = full_record();
        input.push(RECORD_SEPARATOR);
        let commits = parse_log_format(&input);

        assert_eq!(commits.len(), 1);
        let commit = &commits[0];
        assert_eq!(commit.oid, "760ba1f4b59dfd7120abea5825030081ba94c12c");
        assert_eq!(
            commit.parents,
            vec!["63daff978b6cc470c01793c73fd055e32655f2a8".to_owned()]
        );
        assert_eq!(commit.author.name, "Fixture Author");
        assert_eq!(commit.author.email, "author@example.com");
        assert_eq!(commit.author.time, Some(1_704_164_645));
        assert_eq!(commit.committer.name, "Fixture Committer");
        assert_eq!(commit.committer.time, Some(1_704_164_645));
        assert_eq!(commit.refs, vec!["HEAD -> main".to_owned()]);
        assert_eq!(commit.signature, SignatureStatus::Unsigned);
        assert_eq!(commit.subject, "third commit");
        assert_eq!(commit.body, None, "列表页不带正文");
        assert!(!commit.is_root());
        assert!(!commit.is_merge());
    }

    #[test]
    fn nul_terminated_records_are_accepted_as_well_as_newline_terminated() {
        let mut nul_terminated = full_record();
        nul_terminated.push(RECORD_SEPARATOR);
        nul_terminated.push(0);
        let mut newline_terminated = full_record();
        newline_terminated.push(RECORD_SEPARATOR);
        newline_terminated.push(b'\n');

        assert_eq!(parse_log_format(&nul_terminated).len(), 1);
        assert_eq!(parse_log_format(&newline_terminated).len(), 1);
    }

    #[test]
    fn root_commit_has_no_parents_and_is_recognised_as_root() {
        let mut input = record(&[
            "26dac129bf5bd91461cc8429e89651e3c3740776",
            "",
            "Fixture Author",
            "author@example.com",
            "1704164645",
            "Fixture Committer",
            "committer@example.com",
            "1704164645",
            "HEAD -> main",
            "N",
            "root commit",
        ]);
        input.push(RECORD_SEPARATOR);

        let commits = parse_log_format(&input);
        assert!(commits[0].parents.is_empty());
        assert!(commits[0].is_root());
    }

    #[test]
    fn merge_commit_keeps_parent_order_and_is_recognised_as_merge() {
        let mut input = record(&[
            "a02887ff94ff86bb072e2293e049a878550de26e",
            "6755f709bbb7b6bfa37719dc066e12e643bc4a00 f747c87500f1ff37f01578cf78e1ebdf1a42d040",
            "Fixture Author",
            "author@example.com",
            "1704164645",
            "Fixture Committer",
            "committer@example.com",
            "1704164645",
            "HEAD -> main",
            "N",
            "merge feature",
        ]);
        input.push(RECORD_SEPARATOR);

        let commits = parse_log_format(&input);
        assert_eq!(commits[0].parents.len(), 2);
        assert_eq!(
            commits[0].parents[0],
            "6755f709bbb7b6bfa37719dc066e12e643bc4a00"
        );
        assert!(commits[0].is_merge());
    }

    #[test]
    fn refs_are_split_on_comma_space_and_trimmed() {
        let mut input = record(&[
            "4a5489a19f976fcf148347841be9ec35d25091e9",
            "ed106f2752ca77e6df37317e1a5fb1fa32ac551a",
            "Fixture Author",
            "author@example.com",
            "1704164645",
            "Fixture Committer",
            "committer@example.com",
            "1704164645",
            "HEAD -> main, side",
            "N",
            "feat: second commit",
        ]);
        input.push(RECORD_SEPARATOR);

        assert_eq!(
            parse_log_format(&input)[0].refs,
            vec!["HEAD -> main".to_owned(), "side".to_owned()]
        );
    }

    #[test]
    fn empty_refs_field_yields_an_empty_list() {
        let mut input = record(&[
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "",
            "A",
            "a@b.c",
            "1",
            "A",
            "a@b.c",
            "1",
            "",
            "N",
            "subject",
        ]);
        input.push(RECORD_SEPARATOR);

        assert!(parse_log_format(&input)[0].refs.is_empty());
    }

    #[test]
    fn unparsable_timestamp_becomes_none_instead_of_epoch() {
        let mut input = record(&[
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "",
            "A",
            "a@b.c",
            "not-a-number",
            "A",
            "a@b.c",
            "1",
            "",
            "N",
            "subject",
        ]);
        input.push(RECORD_SEPARATOR);

        assert_eq!(parse_log_format(&input)[0].author.time, None);
    }

    #[test]
    fn records_with_the_wrong_field_count_are_dropped() {
        let mut input = record(&["oid-only"]);
        input.push(RECORD_SEPARATOR);

        assert!(parse_log_format(&input).is_empty());
    }

    #[test]
    fn invalid_utf8_subject_is_replaced_without_dropping_the_commit() {
        let mut input = full_record();
        // 用非法字节替换 subject 字段
        input.truncate(input.len() - "third commit".len());
        input.extend_from_slice(&[0xFF, 0xFE]);
        input.push(RECORD_SEPARATOR);

        let commits = parse_log_format(&input);
        assert_eq!(commits.len(), 1);
        assert!(commits[0].subject.contains('\u{FFFD}'));
    }
}
