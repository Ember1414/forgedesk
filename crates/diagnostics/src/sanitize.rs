//! 日志与错误详情的脱敏。
//!
//! 依据 AGENTS.md 红线 R8：Token / 密码 / 私钥**不得**进入日志、崩溃报告或遥测。
//! 大量真实泄漏不是"主动打印密码"，而是把 git / HTTP 的原始输出整段贴进日志——
//! 远端 URL、`Authorization` 头、报错回显里都可能夹带凭据。因此所有出境的文本
//! （tracing 日志、`AppError.detail`、`logs_tail` 的返回）都必须先经过本模块。
//!
//! 设计取舍：
//!
//! - **保留前缀**：`ghp_xxxx` 变成 `ghp_«redacted»`。完全抹掉会让用户与支持者
//!   无法判断"被脱敏的到底是哪一类凭据"，而前缀本身不构成机密。
//! - **不引入正则依赖**：脱敏是安全关键路径，手写扫描器的行为可以被逐条单测固定下来，
//!   且避免"正则回溯"这类难以预期的性能问题。规则见下方常量。
//! - **幂等**：`sanitize_log(sanitize_log(x)) == sanitize_log(x)`。
//!   日志可能被多次处理（写入前、读取时），不幂等会导致重复脱敏把文本改得面目全非。

/// 被脱敏内容的占位符。刻意使用非 ASCII 字符，避免被再次当作"值"匹配。
pub const REDACTED: &str = "«redacted»";

/// 形如 `key=value` / `key: value` 的敏感键（大小写不敏感）。
const SENSITIVE_KEYS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api_key",
    "apikey",
    "access_token",
    "refresh_token",
    "id_token",
    "client_secret",
    "private_key",
    "passwd",
    "password",
    "secret",
    "token",
];

/// 凭据前缀：保留前缀，抹掉后面的凭据本体。
const TOKEN_PREFIXES: &[&str] = &[
    "ghp_",        // GitHub personal access token
    "gho_",        // GitHub OAuth token
    "ghu_",        // GitHub user-to-server token
    "ghs_",        // GitHub server-to-server token
    "ghr_",        // GitHub refresh token
    "github_pat_", // GitHub fine-grained PAT
    "glpat-",      // GitLab personal access token
    "xoxb-",       // Slack bot token
    "xoxp-",       // Slack user token
    "AKIA",        // AWS access key id
];

/// 这些词只是凭据的"类型说明"，真正的凭据跟在它们后面，必须一并脱敏。
const CREDENTIAL_SCHEMES: &[&str] = &["bearer", "token", "basic", "digest"];

/// 值结束字符：出现这些字符说明凭据值已经结束。
fn ends_value(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '"' | '\'' | ',' | ';' | '&' | '}' | ')' | ']' | '<' | '>'
        )
}

/// 脱敏入口：对任意文本做全部规则处理。
pub fn sanitize_log(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }

    let without_pem = redact_pem_blocks(input);
    let without_url_credentials = redact_url_credentials(&without_pem);
    let without_keyed_values = redact_after_keys(&without_url_credentials);
    redact_prefixed_tokens(&without_keyed_values)
}

/// 抹掉 PEM 私钥块的内容（保留首尾标记，便于判断"这里曾有一把私钥"）。
fn redact_pem_blocks(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut inside_block = false;

    for segment in input.split_inclusive('\n') {
        let trimmed = segment.trim_end_matches(['\r', '\n']);
        let is_begin = trimmed.contains("-----BEGIN ") && trimmed.contains("PRIVATE KEY-----");
        let is_end = trimmed.contains("-----END ") && trimmed.contains("PRIVATE KEY-----");

        if is_begin {
            inside_block = true;
            output.push_str(segment);
            continue;
        }
        if inside_block && is_end {
            inside_block = false;
            output.push_str(segment);
            continue;
        }
        if inside_block {
            // 用等长占位保持行结构（便于日志按行对齐阅读），只替换非空行
            if trimmed.is_empty() {
                output.push_str(segment);
            } else {
                output.push_str(REDACTED);
                if segment.ends_with('\n') {
                    output.push('\n');
                }
            }
            continue;
        }
        output.push_str(segment);
    }

    output
}

/// 抹掉 `scheme://user:password@host` 中的 userinfo。
fn redact_url_credentials(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while let Some(offset) = input[cursor..].find("://") {
        let authority_start = cursor + offset + 3;
        // authority 段结束于空白、'/'、'?'、'#' 或 ')'（markdown 里常见的包裹）
        let authority_end = input[authority_start..]
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '/' | '?' | '#' | ')' | '"' | '\''))
            .map_or(input.len(), |relative| authority_start + relative);

        let authority = &input[authority_start..authority_end];
        match authority.rfind('@') {
            Some(at) => {
                let userinfo_start = authority_start;
                output.push_str(&input[cursor..userinfo_start]);
                output.push_str(REDACTED);
                cursor = authority_start + at; // 保留 '@' 及其后的 host
            }
            None => {
                cursor = authority_end;
            }
        }
    }

    output.push_str(&input[cursor..]);
    output
}

/// 抹掉 `key=value` / `key: value` 形式的值。
fn redact_after_keys(input: &str) -> String {
    let lowercase = input.to_ascii_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    let mut search_from = 0usize;

    while let Some((key_start, key_len)) = find_first_key(&lowercase, search_from) {
        let after_key = key_start + key_len;

        // 键之后必须（允许空白与一个收尾引号）紧接 `=` 或 `:`，
        // 否则只是普通词汇里的巧合子串。
        //
        // 关键细节：JSON 日志里的键是**带引号**的（`"password":"hunter2"`），
        // 所以键后面会先出现一个引号再出现冒号。早期版本漏掉这一点，
        // 结果我们自己的文件日志（JSON）里的密码字段完全没有被脱敏——
        // 这正是"端到端断言整条日志"才能发现、而单测某条规则发现不了的问题。
        let mut separator_index = skip_whitespace(input, after_key);
        if let Some(quote @ ('"' | '\'')) = input[separator_index..].chars().next() {
            separator_index = skip_whitespace(input, separator_index + quote.len_utf8());
        }
        let separator = input[separator_index..].chars().next();
        if !matches!(separator, Some('=' | ':')) {
            search_from = after_key;
            continue;
        }

        let value_start =
            skip_whitespace(input, separator_index + separator.map_or(0, char::len_utf8));

        // 值可能被引号包裹（`password='hunter2'`）。引号本身保留，
        // 只抹掉里面的内容：既让日志结构可读，也避免把引号一起删掉后无法判断原文边界。
        let quote_len = match input[value_start..].chars().next() {
            Some(quote @ ('"' | '\'')) => quote.len_utf8(),
            _ => 0,
        };
        let inner_start = value_start + quote_len;

        let value_end = scan_value_end(input, inner_start);
        if value_end == inner_start {
            search_from = after_key;
            continue;
        }

        // `Authorization: Bearer <token>`：值本身只是类型说明，继续吃掉后面的凭据
        let primary = &input[inner_start..value_end];
        let final_end = if CREDENTIAL_SCHEMES.contains(&primary.to_ascii_lowercase().as_str()) {
            let mut next_start = value_end;
            while let Some(ch) = input[next_start..].chars().next() {
                if ch.is_whitespace() {
                    next_start += ch.len_utf8();
                } else {
                    break;
                }
            }
            scan_value_end(input, next_start)
        } else {
            value_end
        };

        // 幂等：已经是占位符时原样跳过，避免二次处理继续"吞"后面的文本
        if &input[inner_start..final_end] == REDACTED {
            search_from = final_end;
            continue;
        }

        output.push_str(&input[cursor..inner_start]);
        output.push_str(REDACTED);
        cursor = final_end;
        search_from = final_end;
    }

    output.push_str(&input[cursor..]);
    output
}

/// 在 `lowercase` 中找出最早的敏感键，返回（字节起点，字节长度）。
fn find_first_key(lowercase: &str, from: usize) -> Option<(usize, usize)> {
    SENSITIVE_KEYS
        .iter()
        .filter_map(|key| {
            lowercase[from..]
                .find(key)
                .map(|relative| (from + relative, key.len()))
        })
        .min_by_key(|(start, _)| *start)
}

/// 跳过空白，返回第一个非空白字符的字节位置。
fn skip_whitespace(input: &str, from: usize) -> usize {
    let mut index = from;
    while let Some(ch) = input[index..].chars().next() {
        if ch.is_whitespace() {
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    index
}

/// 从 `start` 起扫描一个值，返回值的结束位置。
fn scan_value_end(input: &str, start: usize) -> usize {
    let mut end = start;
    for ch in input[start..].chars() {
        if ends_value(ch) {
            break;
        }
        end += ch.len_utf8();
    }
    end
}

/// 抹掉已知凭据前缀后面的令牌本体。
fn redact_prefixed_tokens(input: &str) -> String {
    let lowercase = input.to_ascii_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    let mut search_from = 0usize;

    while let Some((prefix_start, prefix_len)) = TOKEN_PREFIXES
        .iter()
        .filter_map(|prefix| {
            let needle = prefix.to_ascii_lowercase();
            lowercase[search_from..]
                .find(&needle)
                .map(|relative| (search_from + relative, prefix.len()))
        })
        .min_by_key(|(start, _)| *start)
    {
        let body_start = prefix_start + prefix_len;
        let mut body_end = body_start;
        for ch in input[body_start..].chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                body_end += ch.len_utf8();
            } else {
                break;
            }
        }

        if body_end == body_start {
            // 前缀后面没有令牌本体（例如已被脱敏过）：跳过，保证幂等
            search_from = body_start;
            continue;
        }

        output.push_str(&input[cursor..body_start]);
        output.push_str(REDACTED);
        cursor = body_end;
        search_from = body_end;
    }

    output.push_str(&input[cursor..]);
    output
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{sanitize_log, REDACTED};

    /// T0.6 验收要求：覆盖 8 种敏感模式。
    #[test]
    fn redacts_eight_sensitive_patterns() {
        let cases: [(&str, &str); 8] = [
            // 1. Authorization 头 + Bearer 令牌
            (
                "Authorization: Bearer ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
                "ghp_",
            ),
            // 2. GitHub 细粒度 PAT（单独出现时保留前缀、抹掉本体）
            (
                "使用 github_pat_11ABCDEFG0abcdefghijklmnop 访问",
                "11ABCDEFG0abcdefghijklmnop",
            ),
            // 3. GitLab PAT
            ("token=glpat-Zx9Yy8Ww7Vv6Uu5Tt4Ss", "glpat-"),
            // 4. 查询串里的 access_token
            ("https://api.example.com/x?access_token=abcdef123456&page=2", "abcdef123456"),
            // 5. 基本认证（userinfo）
            ("fatal: unable to access 'https://alice:s3cr3t@example.com/a/b.git/'", "s3cr3t"),
            // 6. Basic 认证（base64）
            ("Authorization: Basic YWxpY2U6czNjcjN0", "YWxpY2U6czNjcjN0"),
            // 7. 密码键值对
            ("password='hunter2' user=admin", "hunter2"),
            // 8. PEM 私钥块
            (
                "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----",
                "b3BlbnNzaC1rZXktdjEAAAAA",
            ),
        ];

        for (input, secret) in cases {
            let output = sanitize_log(input);
            assert!(!output.contains(secret), "未脱敏：{input}\n输出：{output}");
            assert!(output.contains(REDACTED), "应出现占位符：{output}");
        }
    }

    #[test]
    fn keeps_token_prefix_for_diagnosability() {
        assert_eq!(
            sanitize_log("remote: invalid token ghp_ABCDEFGH"),
            format!("remote: invalid token ghp_{REDACTED}")
        );
    }

    #[test]
    fn redacts_aws_access_key_id() {
        let output = sanitize_log("AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE");
        assert!(!output.contains("AKIAIOSFODNN7EXAMPLE"), "输出：{output}");
    }

    #[test]
    fn is_idempotent() {
        let inputs = [
            "Authorization: Bearer ghp_ABCDEFGH",
            "https://alice:s3cr3t@example.com/a.git",
            "password=hunter2 token=glpat-ABCDEFG",
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEow\n-----END RSA PRIVATE KEY-----",
            "普通文本，没有凭据",
        ];

        for input in inputs {
            let once = sanitize_log(input);
            let twice = sanitize_log(&once);
            assert_eq!(once, twice, "脱敏不幂等：{input}");
        }
    }

    #[test]
    fn leaves_ordinary_text_untouched() {
        let text = "error: Your local changes would be overwritten by merge";
        assert_eq!(sanitize_log(text), text);
    }

    #[test]
    fn handles_empty_and_unicode_without_panicking() {
        assert_eq!(sanitize_log(""), "");
        // 多字节字符必须原样保留（扫描以字节定位，切片点必须落在字符边界上）
        let output = sanitize_log("提交信息：修复中文 token=秘密值 的问题");
        assert!(output.contains("提交信息"), "中文被破坏：{output}");
        assert!(!output.contains("秘密值"));
    }

    /// JSON 日志（我们自己的文件格式）里的字段必须被脱敏。
    ///
    /// 这是 T0.8 的回归测试：早期实现要求键后紧跟分隔符，
    /// 而 JSON 里键是带引号的（`"password":"hunter2"`），于是密码字段整体漏过。
    #[test]
    fn redacts_json_style_key_value_pairs() {
        let cases = [
            (r#"{"password":"hunter2"}"#, "hunter2"),
            (r#"{"token": "glpat-ABCDEFG"}"#, "glpat-ABCDEFG"),
            (
                r#"{"api_key":"AKIAIOSFODNN7EXAMPLE"}"#,
                "AKIAIOSFODNN7EXAMPLE",
            ),
            (r#"{"client_secret":"s3cr3t-value"}"#, "s3cr3t-value"),
            (r#"{'password':'single-quoted'}"#, "single-quoted"),
        ];

        for (input, secret) in cases {
            let output = sanitize_log(input);
            assert!(
                !output.contains(secret),
                "JSON 字段未脱敏：{input} -> {output}"
            );
            assert!(output.contains(REDACTED), "应出现占位符：{output}");
            // 引号与结构必须保留，否则日志不再是合法 JSON
            assert!(
                output.contains('"') || output.contains('\''),
                "结构被破坏：{output}"
            );
        }
    }

    /// 键与分隔符之间有空白的写法同样要覆盖（JSON 格式化器与手工日志都可能这么写）。
    #[test]
    fn redacts_keys_separated_by_whitespace() {
        for (input, secret) in [
            (r#"{"password" : "hunter2"}"#, "hunter2"),
            ("password = hunter2", "hunter2"),
            ("password\t:\thunter2", "hunter2"),
        ] {
            let output = sanitize_log(input);
            assert!(!output.contains(secret), "未脱敏：{input} -> {output}");
        }
    }

    #[test]
    fn redacts_multiple_credentials_in_one_line() {
        let output = sanitize_log("token=aaaa1111 and password: bbbb2222");
        assert!(!output.contains("aaaa1111"));
        assert!(!output.contains("bbbb2222"));
    }
}
