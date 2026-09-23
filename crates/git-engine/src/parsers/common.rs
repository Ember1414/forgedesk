//! 各解析器共用的字节级小工具。

/// 解析 porcelain 的八进制文件模式。
///
/// 全零（`000000`）在 Git 的输出里表示"该侧不存在"，因此返回 `None` 而不是 `0`：
/// `0` 会被下游当成"模式为 0 的文件"，而正确的语义是"没有这个版本"。
/// 非法字符或溢出同样返回 `None`（畸形输入不 panic）。
pub(super) fn parse_mode(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || bytes.iter().all(|byte| *byte == b'0') {
        return None;
    }

    let mut value: u32 = 0;
    for byte in bytes {
        let digit = char::from(*byte).to_digit(8)?;
        value = value.checked_mul(8)?.checked_add(digit)?;
    }
    Some(value)
}

/// 解析十六进制 oid。
///
/// 全零表示"不存在"（Git 用零 oid 占位），返回 `None`。非十六进制字符视为畸形。
pub(super) fn parse_oid(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.iter().all(|byte| *byte == b'0') {
        return None;
    }
    if !bytes.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

/// 解析十进制整数（用于时间戳、stage 编号、增删行数）。
pub(super) fn parse_decimal(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() {
        return None;
    }
    // 逐字节解析而不是先转字符串：`from_utf8` 在这里既慢又多余。
    let text = std::str::from_utf8(bytes).ok()?;
    text.parse::<i64>().ok()
}

/// 解析 `+<ahead> -<behind>` 形式的领先/落后计数。
///
/// 返回 `(ahead, behind)`；任一侧畸形时整体返回 `None`——只解析一半会让界面显示
/// "领先 3、落后 0"，而真相是"落后数字没解析出来"。
pub(super) fn parse_ahead_behind(bytes: &[u8]) -> Option<(i64, i64)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut parts = text.split(' ');
    let ahead = parts.next()?.strip_prefix('+')?.parse::<i64>().ok()?;
    let behind = parts.next()?.strip_prefix('-')?.parse::<i64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((ahead, behind))
}

/// 找出 `needle` 从 `from` 开始的第一个位置。
pub(super) fn find_byte(haystack: &[u8], from: usize, needle: u8) -> Option<usize> {
    haystack
        .get(from..)?
        .iter()
        .position(|byte| *byte == needle)
        .map(|offset| from + offset)
}

/// 跳过记录之间的分隔残留（NUL / LF / CR）。
///
/// 为什么需要：`-z` 模式下 git 用 NUL 结束记录，但管道拼接、日志回放、
/// 手工构造的样本里常混入换行。把换行当成"一条新记录"会产生一堆空条目。
pub(super) fn skip_record_separators(input: &[u8], mut cursor: usize) -> usize {
    while let Some(byte) = input.get(cursor) {
        if matches!(*byte, 0 | b'\n' | b'\r') {
            cursor += 1;
        } else {
            break;
        }
    }
    cursor
}

/// 去掉字节切片首尾的记录分隔残留。
pub(super) fn trim_record_separators(record: &[u8]) -> &[u8] {
    let start = skip_record_separators(record, 0);
    let mut end = record.len();
    while end > start && matches!(record[end - 1], 0 | b'\n' | b'\r') {
        end -= 1;
    }
    &record[start..end]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{parse_ahead_behind, parse_decimal, parse_mode, parse_oid, trim_record_separators};

    #[test]
    fn all_zero_mode_means_absent_instead_of_mode_zero() {
        assert_eq!(parse_mode(b"000000"), None);
        assert_eq!(parse_mode(b""), None);
        assert_eq!(parse_mode(b"100644"), Some(0o100644));
        assert_eq!(parse_mode(b"160000"), Some(0o160000));
    }

    #[test]
    fn malformed_mode_is_rejected_without_panicking() {
        assert_eq!(parse_mode(b"10064x"), None);
        assert_eq!(parse_mode(b"99999999999999999999999"), None);
    }

    #[test]
    fn all_zero_oid_means_absent() {
        assert_eq!(parse_oid(b"0000000000000000000000000000000000000000"), None);
        assert_eq!(parse_oid(b""), None);
        assert_eq!(
            parse_oid(b"b2f931a67315c95c5daab3aac6de62e534808476").as_deref(),
            Some("b2f931a67315c95c5daab3aac6de62e534808476")
        );
        assert_eq!(parse_oid(b"zzzz"), None);
    }

    #[test]
    fn decimal_parsing_rejects_empty_and_non_numeric() {
        assert_eq!(parse_decimal(b"1704164645"), Some(1_704_164_645));
        assert_eq!(parse_decimal(b""), None);
        assert_eq!(parse_decimal(b"12a"), None);
    }

    #[test]
    fn ahead_behind_requires_both_sides_to_be_well_formed() {
        assert_eq!(parse_ahead_behind(b"+1 -1"), Some((1, 1)));
        assert_eq!(parse_ahead_behind(b"+0 -0"), Some((0, 0)));
        assert_eq!(parse_ahead_behind(b"+1"), None);
        assert_eq!(parse_ahead_behind(b"1 -1"), None);
        assert_eq!(parse_ahead_behind(b"+1 -1 +2"), None);
    }

    #[test]
    fn trailing_separators_are_trimmed() {
        assert_eq!(
            trim_record_separators(b"# branch.head main\0"),
            b"# branch.head main"
        );
        assert_eq!(trim_record_separators(b"\n\r\0"), b"");
    }
}
