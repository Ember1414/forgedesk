//! 令牌脱敏：provider 错误详情进入 IPC / 日志前的最后一道闸。
//!
//! # 为什么不直接用 `diagnostics::sanitize_log`
//!
//! 依赖方向上 provider 不能依赖 diagnostics（infra → infra 只有
//! git-engine → diagnostics 一个已登记例外），而把它上移到 domain 又要在
//! M3 并行期间改动 diagnostics / git-engine 的代码。因此这里先实现一份
//! **token 专属**的擦除，与 `diagnostics::sanitize_log` 的统一登记为
//! 技术债（M4 收尾时处理）。
//!
//! # 覆盖范围与擦除策略
//!
//! - **GitHub 令牌本体**：`ghp_`/`gho_`/`ghu_`/`ghs_`/`ghr_` 前缀 + 长
//!   base62 体，以及 `github_pat_` 开头的细粒度令牌。擦成
//!   `前缀***`——前缀保留是为了排错时还能认出"这曾是一个 GitHub 令牌"；
//! - **Authorization 头的值**：整行以 `authorization` 开头（大小写不敏感）
//!   时，把行内 `bearer <v>` / `token <v>` 的值擦掉。为什么只认整行：
//!   裸的 "token " 是普通英语（"the token was not accepted"），
//!   全文匹配会毁掉所有正常句子；
//! - **脱离上下文的 `Bearer <v>`**（大小写两种）：这是 HTTP 头风格，
//!   出现在正文里几乎必然是泄漏；
//! - **URL 查询参数** `token=` / `access_token=` / `refresh_token=` /
//!   `client_secret=` / `code=`——`code=` 是 Device Flow 的一次性授权码，
//!   泄露同样等于泄露会话。
//!
//! 原则与 diagnostics 一致：**擦除失败宁可多擦**（擦掉一段正常文本的代价
//! 远小于漏掉一个真令牌）。

/// 擦除后的占位符（与 diagnostics 的 `REDACTED` 语义一致）。
pub const REDACTED: &str = "***";

/// 擦除文本中所有可识别的令牌形态，返回可安全展示/写日志的文本。
#[must_use]
pub fn redact_tokens(text: &str) -> String {
    let mut out = text.to_owned();

    // 1) Authorization 行内的 scheme+值（对 gh 前缀令牌也一样生效，先擦先赢）
    mask_authorization_lines(&mut out);
    // 2) 脱离上下文的 Bearer 头风格（`token ` 一词太常见，不在此列）
    for scheme in ["Bearer ", "bearer "] {
        mask_after(&mut out, scheme, |c| !c.is_whitespace() && !c.is_control());
    }
    // 3) GitHub 令牌本体。github_pat_ 排第一只是习惯：新增前缀时先想清楚
    //    前缀之间的包含关系
    for prefix in ["github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        mask_after(&mut out, prefix, |c| c.is_ascii_alphanumeric() || c == '_');
    }
    // 4) 查询参数
    for param in [
        "token=",
        "access_token=",
        "refresh_token=",
        "client_secret=",
        "code=",
    ] {
        mask_query_param(&mut out, param);
    }
    out
}

/// 把 `prefix + 连续密文体` 替换为 `prefix***`（密文体丢弃）。
///
/// 密文体为空时不擦：避免把恰好以这些前缀开头的普通词整段吞掉。
fn mask_after(text: &mut String, prefix: &str, is_secret_char: fn(char) -> bool) {
    let original = std::mem::take(text);
    let mut rest = original.as_str();
    while let Some(pos) = rest.find(prefix) {
        text.push_str(&rest[..pos + prefix.len()]);
        let after = &rest[pos + prefix.len()..];
        let len = after.chars().take_while(|c| is_secret_char(*c)).count();
        if len > 0 {
            text.push_str(REDACTED);
        }
        rest = &after[len..];
    }
    text.push_str(rest);
}

/// 对整行以 `authorization` 开头（跳过前导空白，大小写不敏感）的行，
/// 擦除行内 `scheme + 空白 + 值` 的值部分。
fn mask_authorization_lines(text: &mut String) {
    let original = std::mem::take(text);
    *text = original
        .split_inclusive('\n')
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.len() >= "authorization".len()
                && trimmed[..("authorization").len()].eq_ignore_ascii_case("authorization")
            {
                // 行首切片是 ASCII 长度，必然落在字符边界上
                mask_scheme_values(line)
            } else {
                line.to_owned()
            }
        })
        .collect();
}

/// 在一行里反复匹配 `bearer|token + 空白 + 值`（大小写不敏感），值擦成占位符。
///
/// 匹配用字节比较完成（needle 都是 ASCII），避免在多字节字符中间切片。
fn mask_scheme_values(line: &str) -> String {
    let Some((head, tail)) = line.split_once(':') else {
        return line.to_owned();
    };
    let mut out = String::with_capacity(line.len());
    out.push_str(head);
    out.push(':');
    let mut rest = tail;
    loop {
        // scheme 前的空白原样保留
        let ws = rest.len() - rest.trim_start().len();
        out.push_str(&rest[..ws]);
        rest = &rest[ws..];

        let matched = ["bearer", "token"].into_iter().find(|scheme| {
            let bytes = rest.as_bytes();
            bytes.len() > scheme.len()
                && rest.as_bytes()[..scheme.len()].eq_ignore_ascii_case(scheme.as_bytes())
                && bytes[scheme.len()].is_ascii_whitespace()
        });
        let Some(scheme) = matched else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..scheme.len()]);
        rest = &rest[scheme.len()..];

        // scheme 与值之间的空白原样保留
        let ws = rest.len() - rest.trim_start().len();
        out.push_str(&rest[..ws]);
        rest = &rest[ws..];

        let len = rest
            .chars()
            .take_while(|c| !c.is_whitespace() && !c.is_control())
            .count();
        if len == 0 {
            out.push_str(rest);
            return out;
        }
        out.push_str(REDACTED);
        rest = &rest[len..];
    }
}

/// 擦除 URL 查询参数形态 `name=value`（值读到 `&` / `#` / 串尾）。
///
/// 参数名前必须是 `?`、`&` 或串首：避免把更长参数名（如 `x_client_secret=`）
/// 的尾巴局部误匹配。
fn mask_query_param(text: &mut String, param: &str) {
    let original = std::mem::take(text);
    let mut rest = original.as_str();
    while let Some(pos) = rest.find(param) {
        let boundary_ok =
            pos == 0 || matches!(rest[..pos].chars().next_back(), Some('?') | Some('&'));
        let value_start = pos + param.len();
        let after = &rest[value_start..];
        let len = after
            .chars()
            .take_while(|c| *c != '&' && *c != '#' && !c.is_control())
            .count();
        text.push_str(&rest[..value_start]);
        if boundary_ok && len > 0 {
            text.push_str(REDACTED);
        } else {
            text.push_str(&after[..len]);
        }
        rest = &after[len..];
    }
    text.push_str(rest);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{redact_tokens, REDACTED};

    const CLASSIC_BODY: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCD";

    #[test]
    fn classic_and_fine_grained_github_tokens_are_erased() {
        let classic = format!("failed for ghp_{CLASSIC_BODY} at /x");
        assert_eq!(
            redact_tokens(&classic),
            format!("failed for ghp_{REDACTED} at /x")
        );

        let fine = "github_pat_11ABCDEFG0abcdefghijklmnopqrstuvwxyz0123456789_\
                    012345678901234567890123456789012345678901234567890123456789 rejected";
        let redacted = redact_tokens(fine);
        assert!(
            redacted.contains("github_pat_"),
            "前缀保留便于排错：{redacted}"
        );
        assert!(
            !redacted.contains("11ABCDEFG0"),
            "密文体必须消失：{redacted}"
        );
    }

    #[test]
    fn every_classic_token_prefix_is_covered() {
        for prefix in ["gho_", "ghu_", "ghs_", "ghr_"] {
            let raw = format!("token {prefix}{CLASSIC_BODY} invalid");
            assert_eq!(
                redact_tokens(&raw),
                format!("token {prefix}{REDACTED} invalid"),
                "{prefix} 未被擦除"
            );
        }
    }

    #[test]
    fn authorization_values_are_erased_in_both_flavours_and_any_case() {
        let bearer = format!("Authorization: Bearer gho_{CLASSIC_BODY}");
        assert_eq!(redact_tokens(&bearer), "Authorization: Bearer ***");

        // `token <jwt 形态>`：GitHub Apps 安装令牌常以 JWT 出现
        let header = "authorization: token eyJhbGciOiJSUzI1NiJ9.secret.part";
        assert_eq!(redact_tokens(header), "authorization: token ***");

        let upper = "AUTHORIZATION: TOKEN abc123def";
        assert_eq!(redact_tokens(upper), "AUTHORIZATION: TOKEN ***");
    }

    #[test]
    fn multiple_authorization_schemes_on_one_line_are_all_erased() {
        let raw = "authorization: Bearer ghp_one token second bearer third";
        let redacted = redact_tokens(raw);
        assert!(
            !redacted.contains("one")
                && !redacted.contains("second")
                && !redacted.contains("third"),
            "{redacted}"
        );
        assert_eq!(redacted.matches(REDACTED).count(), 3);
    }

    #[test]
    fn oauth_query_parameters_are_erased() {
        let url = "https://github.com/login/oauth/access_token?client_id=abc&code=xyz789&\
                   client_secret=sekrit123";
        let redacted = redact_tokens(url);

        assert!(!redacted.contains("sekrit123"), "{redacted}");
        assert!(!redacted.contains("xyz789"), "{redacted}");
        assert!(
            redacted.contains("client_id=abc"),
            "client_id 不是秘密，应保留"
        );
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        // 裸的 "token " 是普通英语：只有 Authorization 行里的 scheme 才擦
        let plain = "token refresh failed; the token was not accepted";
        assert_eq!(redact_tokens(plain), plain);

        // `x_client_secret=` 是一个完整参数名，不应被局部误匹配
        let named = "https://api.example.com/x?x_client_secret=visible&ok=1";
        let redacted = redact_tokens(named);
        assert!(redacted.contains("x_client_secret=visible"), "{redacted}");

        // 无关文本零改动
        let unrelated = "GET /repos/octocat/Hello-World 404 not found";
        assert_eq!(redact_tokens(unrelated), unrelated);
    }

    #[test]
    fn a_prefix_without_a_secret_body_is_not_erased() {
        let raw = "see github_pat_ docs and ghp_ notes";
        assert_eq!(redact_tokens(raw), raw);
    }

    #[test]
    fn multiline_text_only_erases_the_authorization_line() {
        let raw = "request failed\nAuthorization: Bearer supersecret\nstatus: 401";
        let redacted = redact_tokens(raw);
        assert!(redacted.contains("status: 401"), "{redacted}");
        assert!(!redacted.contains("supersecret"), "{redacted}");
    }
}
