//! `git config --list -z` 的解析器。
//!
//! # 格式
//!
//! 带 `-z` 时 git 用 `\n` 分隔键与值、用 `\0` 分隔条目：
//!
//! ```text
//! core.repositoryformatversion\n0\0core.filemode\ntrue\0remote.origin.url\nhttps://…\0
//! ```
//!
//! 这样选而不是默认的 `key=value` 逐行输出，是因为**值里可以有换行**
//! （多行别名、带换行的 `filter.*.clean` 脚本），逐行解析会把它拆成两条。
//!
//! # 只解析，不脱敏
//!
//! 值可能内嵌凭据（`core.sshCommand` 里的 URL）。脱敏在
//! `forgedesk-diagnostics`，由调用方在拿到结果后统一处理——解析器保持
//! "字节进、结构出"的纯粹性，才能用固定样本完整覆盖边界。

use forgedesk_domain::git::{ConfigEntry, ConfigScope};

/// 解析 `git config --list -z` 的输出。
///
/// 所有条目的范围都是 [`ConfigScope::Local`]：调用方用 `--local` 取值，
/// 因此范围是已知的，不需要 git 再报一遍。
pub fn parse_config_list(stdout: &[u8]) -> Vec<ConfigEntry> {
    let text = String::from_utf8_lossy(stdout);

    text.split('\0')
        .filter(|record| !record.is_empty())
        .map(|record| {
            // 键值分隔符是**第一个**换行：值里的换行属于值本身
            let (key, value) = match record.split_once('\n') {
                Some((key, value)) => (key, value),
                None => (record, ""),
            };
            ConfigEntry {
                key: key.trim().to_owned(),
                value: value.to_owned(),
                scope: ConfigScope::Local,
            }
        })
        .filter(|entry| !entry.key.is_empty())
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::parse_config_list;
    use forgedesk_domain::git::ConfigScope;

    #[test]
    fn parses_key_value_records() {
        let output = b"core.repositoryformatversion\n0\0core.filemode\ntrue\0";
        let entries = parse_config_list(output);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "core.repositoryformatversion");
        assert_eq!(entries[0].value, "0");
        assert_eq!(entries[1].key, "core.filemode");
        assert_eq!(entries[1].value, "true");
        assert!(entries
            .iter()
            .all(|entry| entry.scope == ConfigScope::Local));
    }

    #[test]
    fn a_value_containing_newlines_stays_in_one_entry() {
        let output = b"alias.multi\n!sh -c 'echo one\necho two'\0core.pager\nless\0";
        let entries = parse_config_list(output);

        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].value, "!sh -c 'echo one\necho two'",
            "值里的换行必须留在值里，否则一条别名会被拆成两条"
        );
        assert_eq!(entries[1].key, "core.pager");
    }

    #[test]
    fn a_valueless_key_yields_an_empty_value() {
        let entries = parse_config_list(b"core.bare\0");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "core.bare");
        assert_eq!(entries[0].value, "");
    }

    #[test]
    fn empty_and_trailing_separators_are_skipped() {
        assert!(parse_config_list(b"").is_empty());
        assert_eq!(parse_config_list(b"\0\0").len(), 0);
        assert_eq!(parse_config_list(b"a.b\n1\0").len(), 1);
    }

    #[test]
    fn random_bytes_do_not_panic() {
        let entries = parse_config_list(&[0xff, 0xfe, b'\n', 0x00, 0x80]);
        assert!(entries.iter().all(|entry| !entry.key.is_empty()));
    }
}
