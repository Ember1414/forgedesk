//! git 版本解析与最低版本比较。
//!
//! # 为什么要有最低版本
//!
//! 本产品大量依赖较新的机器可读输出与选项（`status --porcelain=v2`、
//! `rev-parse --is-shallow-repository`、`init -b`、`--force-with-lease` 的现代语义…）。
//! 版本过低时的正确做法是**明确告知**"你的 git 太旧，这些功能不可用"，
//! 而不是让某条命令在运行期以一句难以理解的 `unknown option` 失败。
//!
//! # 解析为什么要容忍后缀
//!
//! `git --version` 的输出五花八门：
//!
//! ```text
//! git version 2.54.0
//! git version 2.39.3 (Apple Git-145)
//! git version 2.54.0.windows.1
//! git version 2.30.0.rc0
//! ```
//!
//! 因此只解析前三个数字段，其余（`windows.1`、`(Apple Git-145)`、`rc0`）原样保留在
//! [`GitVersion::raw`] 里供展示。**不做**"字母后缀视为预发布"这类推断：
//! 用户机器上的 Apple Git 带自己的补丁号，把它当预发布会让所有 macOS 用户被误判。

/// 本产品要求的最低 git 版本。
///
/// 2.30 是 `init -b`（T1.3 的初始化）、`--pathspec-from-file`（T1.1 的非 UTF-8 路径）
/// 都已具备的版本；再往上会引入新选项，届时单独做能力探测，而不是继续抬高底线。
pub const MINIMUM_GIT_VERSION: GitVersion = GitVersion {
    major: 2,
    minor: 30,
    patch: 0,
    raw: String::new(),
};

/// 一个已解析的 git 版本号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitVersion {
    /// 主版本号。
    pub major: u32,
    /// 次版本号。
    pub minor: u32,
    /// 修订号；`2.39`（两段）时按 `0` 处理。
    pub patch: u32,
    /// 原始输出行（用于展示与诊断，例如 `2.54.0.windows.1`）。
    pub raw: String,
}

impl GitVersion {
    /// 从 `git --version` 的输出解析。
    ///
    /// 接受整行（`git version 2.54.0`）或只有版本号（`2.54.0`）。
    /// 解析不出三段数字时返回 `None`——调用方应据此给出"无法识别 git 版本"
    /// 的提示，而不是当成 0.0.0（那会让所有操作都被判为版本过低）。
    pub fn parse(output: &str) -> Option<Self> {
        // 取**第一个形如版本号的词**，而不是"最后一个词"：
        // `git version 2.39.3 (Apple Git-145)` 的最后一个词是 `Git-145)`。
        let token = output
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())?
            .split_whitespace()
            .find(|token| token.starts_with(|c: char| c.is_ascii_digit()) && token.contains('.'))?;

        // `2.54.0.windows.1` → 取前三段；`2.39` → 补 0
        let mut parts = token.split('.');
        let major = parse_number(parts.next()?)?;
        let minor = parse_number(parts.next().unwrap_or("0"))?;
        let patch = parse_number(parts.next().unwrap_or("0"))?;

        Some(Self {
            major,
            minor,
            patch,
            raw: token.to_owned(),
        })
    }

    /// 是否满足最低版本要求。
    pub fn is_supported(&self) -> bool {
        self.is_at_least(&MINIMUM_GIT_VERSION)
    }

    /// 是否不低于 `other`。
    ///
    /// 只比较三段数字：`raw` 里的发行版后缀不参与比较（见模块头）。
    pub fn is_at_least(&self, other: &Self) -> bool {
        (self.major, self.minor, self.patch) >= (other.major, other.minor, other.patch)
    }
}

impl std::fmt::Display for GitVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.raw.is_empty() {
            write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
        } else {
            f.write_str(&self.raw)
        }
    }
}

/// 解析一段十进制数字；带非数字后缀（`0rc1`）时取数字前缀。
fn parse_number(text: &str) -> Option<u32> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{GitVersion, MINIMUM_GIT_VERSION};

    fn version(major: u32, minor: u32, patch: u32) -> GitVersion {
        GitVersion {
            major,
            minor,
            patch,
            raw: String::new(),
        }
    }

    #[test]
    fn parses_plain_and_decorated_output() {
        let plain = GitVersion::parse("git version 2.54.0").unwrap();
        assert_eq!((plain.major, plain.minor, plain.patch), (2, 54, 0));
        assert_eq!(plain.raw, "2.54.0");

        let windows = GitVersion::parse("git version 2.54.0.windows.1").unwrap();
        assert_eq!((windows.major, windows.minor, windows.patch), (2, 54, 0));
        assert_eq!(windows.raw, "2.54.0.windows.1");

        let apple = GitVersion::parse("git version 2.39.3 (Apple Git-145)").unwrap();
        assert_eq!((apple.major, apple.minor, apple.patch), (2, 39, 3));
    }

    #[test]
    fn two_segment_versions_get_a_zero_patch() {
        let two = GitVersion::parse("git version 2.30").unwrap();
        assert_eq!((two.major, two.minor, two.patch), (2, 30, 0));
    }

    #[test]
    fn release_candidates_keep_their_numeric_prefix() {
        // `2.30.0.rc0` 不应被当成"预发布"而降低版本——那会让 macOS 用户被误判
        let rc = GitVersion::parse("git version 2.30.0.rc0").unwrap();
        assert_eq!((rc.major, rc.minor, rc.patch), (2, 30, 0));
        assert!(rc.is_supported());
    }

    #[test]
    fn unparseable_output_returns_none() {
        assert_eq!(GitVersion::parse(""), None);
        assert_eq!(GitVersion::parse("   "), None);
        assert_eq!(GitVersion::parse("git version unknown"), None);
    }

    #[test]
    fn support_threshold_is_two_thirty() {
        assert_eq!(MINIMUM_GIT_VERSION.major, 2);
        assert_eq!(MINIMUM_GIT_VERSION.minor, 30);

        assert!(version(2, 30, 0).is_supported());
        assert!(version(2, 54, 0).is_supported());
        assert!(version(3, 0, 0).is_supported());

        assert!(!version(2, 29, 9).is_supported());
        assert!(!version(1, 9, 0).is_supported());
    }

    #[test]
    fn comparison_is_lexicographic_on_the_three_numbers() {
        assert!(version(2, 30, 1).is_at_least(&version(2, 30, 0)));
        assert!(!version(2, 30, 0).is_at_least(&version(2, 30, 1)));
        assert!(version(2, 31, 0).is_at_least(&version(2, 30, 9)));
    }

    #[test]
    fn display_prefers_the_raw_token() {
        assert_eq!(
            GitVersion::parse("git version 2.54.0").unwrap().to_string(),
            "2.54.0"
        );
        assert_eq!(version(2, 30, 0).to_string(), "2.30.0");
    }
}
