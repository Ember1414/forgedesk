//! 内存中的明文凭据包装。
//!
//! # 为什么不能直接用 `String`
//!
//! 明文一旦是 `String`，它就会出现在所有"顺手写一行"的地方：
//! `tracing::debug!("{:?}", credential)`、`unwrap_or_else` 里的 panic 消息、
//! 测试断言失败的 diff。红线 R8 的两次真实事故都来自这类"没人以为会泄露"的路径。
//!
//! [`Secret`] 做了两件事：
//!   1. `Debug` 只输出 `<redacted>`（结构体派生 Debug 也安全）；
//!   2. Drop 时清零内存（`secrecy` 的 `ZeroizeOnDrop`）。
//!
//! 唯一读取明文的入口是 [`Secret::expose`]，它把"我知道自己在拿明文"变成一次显式调用，
//! 便于 code review 与将来的静态检查。

use std::fmt;

use secrecy::{ExposeSecret, SecretString};

/// 内存中的明文凭据。
///
/// `Clone` 是刻意保留的：注入 git 进程时需要把它放进环境变量/参数，
/// 而 `expose` 返回的是借用，无法跨越进程边界。
#[derive(Clone)]
pub struct Secret(SecretString);

impl Secret {
    /// 包装一段明文。
    pub fn new(value: impl Into<String>) -> Self {
        Self(SecretString::from(value.into()))
    }

    /// 仅供**真正要用**的地方读取明文：写 keyring、注入 git 进程、计算速率限制的键。
    ///
    /// 不要把它用在日志、错误消息或界面上——那正是本类型要防的事。
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    /// 是否为空（例如用户清空了输入框，不该写进 keyring）。
    pub fn is_empty(&self) -> bool {
        self.0.expose_secret().is_empty()
    }

    /// 明文字节长度（诊断用，不泄露内容）。
    pub fn len(&self) -> usize {
        self.0.expose_secret().len()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 连长度都不打印：长度也是信息（可以用来缩小枚举范围）
        formatter.write_str("Secret(<redacted>)")
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self::new(value.to_owned())
    }
}

/// 调试字符串（例如测试里拼断言消息）也必须脱敏。
impl fmt::Display for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_never_print_the_plaintext() {
        let secret = Secret::new("ghp_supersecret");

        let debug = format!("{secret:?}");
        let display = format!("{secret}");

        assert_eq!(debug, "Secret(<redacted>)");
        assert_eq!(display, "<redacted>");
        assert!(!debug.contains("supersecret"));
        assert!(!display.contains("supersecret"));
    }

    #[test]
    fn a_struct_that_derives_debug_still_redacts_the_nested_secret() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            login: String,
            secret: Secret,
        }

        let text = format!(
            "{:?}",
            Holder {
                login: "octocat".to_owned(),
                secret: Secret::new("ghp_supersecret"),
            }
        );

        assert!(text.contains("octocat"));
        assert!(!text.contains("supersecret"), "{text}");
    }

    #[test]
    fn debug_output_does_not_even_leak_the_length() {
        let short = format!("{:?}", Secret::new("a"));
        let long = format!("{:?}", Secret::new("aaaaaaaaaaaaaaaaaaaaaaaa"));

        assert_eq!(short, long);
    }

    #[test]
    fn only_expose_returns_the_plaintext() {
        let secret = Secret::new("ghp_supersecret");

        assert_eq!(secret.expose(), "ghp_supersecret");
        assert_eq!(secret.len(), "ghp_supersecret".len());
        assert!(!secret.is_empty());
        assert!(Secret::new(String::new()).is_empty());
    }
}
