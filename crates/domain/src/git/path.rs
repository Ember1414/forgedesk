//! 仓库内路径：保留 Git 输出的原始字节。
//!
//! 为什么不用 `String`/`PathBuf` 直接承载：Git 的路径是**字节串**，
//! 只有在 `core.quotepath=false` 且用户恰好用了 UTF-8 时才等价于字符串。
//! 真实世界里"文件名不是合法 UTF-8"的仓库（GBK 文件名、日文 Shift-JIS 文件名、
//! 甚至故意构造的畸形名字）并不罕见，而 `String::from_utf8_lossy` 会把这些字节
//! 换成 `U+FFFD`。一旦用假路径去调用文件系统或 `git apply`，得到的是
//! "文件不存在"或"补丁应用到错误文件"，排查成本极高。
//!
//! 因此本类型**只做存储与展示分离**：存的是字节，展示时才 lossy。

use std::borrow::Cow;
use std::fmt;

/// 仓库内路径（相对仓库根，或 Git 输出的绝对路径，视调用场景而定）。
///
/// 原始字节可通过 [`RepoPath::as_bytes`] 取出；展示用 [`RepoPath::to_string_lossy`]。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoPath {
    bytes: Vec<u8>,
}

impl RepoPath {
    /// 用原始字节构造。
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }

    /// 原始字节。
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 是否为合法 UTF-8。
    ///
    /// 调用方据此决定"能否直接当字符串用"（例如写进日志、拼进等价命令），
    /// 而不是先 lossy 再猜。
    pub fn is_utf8(&self) -> bool {
        std::str::from_utf8(&self.bytes).is_ok()
    }

    /// 供展示的 lossy 文本。非法字节会被替换为 `U+FFFD`。
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }

    /// 是否为合法 UTF-8 的路径，若是则返回借用，否则返回 `None`。
    ///
    /// 与 [`RepoPath::is_utf8`] 的区别：需要真值而不只是判断时用这个，
    /// 避免"先判断再 unwrap"的写法。
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes).ok()
    }
}

impl From<&str> for RepoPath {
    fn from(value: &str) -> Self {
        Self::from_bytes(value.as_bytes().to_vec())
    }
}

impl From<String> for RepoPath {
    fn from(value: String) -> Self {
        Self::from_bytes(value.into_bytes())
    }
}

impl From<&std::path::Path> for RepoPath {
    fn from(value: &std::path::Path) -> Self {
        Self::from_bytes(value.to_string_lossy().as_bytes().to_vec())
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display 必须是 lossy 的：`fmt` 只能写字符。需要字节时用 `as_bytes`。
        f.write_str(&self.to_string_lossy())
    }
}

impl serde::Serialize for RepoPath {
    // 手写 Serialize 而不是 derive：derive 会把 `bytes: Vec<u8>` 序列化成
    // JSON 数字数组，而前端契约（`MergeOutcome.conflicts` 等）按**字符串**定义
    // （文件路径对界面是展示数据，UTF-8 之外的字节已无意义）。T3.1 接入
    // ConflictState 时顺手修正了这个 T2.6 起就存在的前后端形状不一致。
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string_lossy())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::RepoPath;

    #[test]
    fn bytes_that_are_not_valid_utf8_are_preserved_verbatim() {
        // 0xE4 0xB8 0xAD 是"中"，单独截断的 0xFF 不是合法 UTF-8 序列
        let raw = vec![0xE4, 0xB8, 0xAD, 0xFF, b'.', b't'];
        let path = RepoPath::from_bytes(raw.clone());

        assert_eq!(path.as_bytes(), raw.as_slice());
        assert!(!path.is_utf8());
        assert!(path.as_str().is_none());
        assert!(path.to_string_lossy().contains('\u{FFFD}'));
    }

    #[test]
    fn valid_utf8_round_trips_through_as_str() {
        let path = RepoPath::from("中文 目录/文件.txt");

        assert!(path.is_utf8());
        assert_eq!(path.as_str(), Some("中文 目录/文件.txt"));
        assert_eq!(path.to_string(), "中文 目录/文件.txt");
    }

    #[test]
    fn serde_serializes_paths_as_strings_not_byte_arrays() {
        // 前端契约按字符串定义（MergeOutcome.conflicts / ConflictFile.path）；
        // 派生 Serialize 会给出数字数组（T2.6 起的形状不一致，T3.1 修正）
        let path = RepoPath::from("src/a.ts");
        assert_eq!(
            serde_json::to_value(&path).unwrap(),
            serde_json::Value::String("src/a.ts".to_owned())
        );

        // 非 UTF-8 字节：序列化是 lossy 的（U+FFFD）——界面上无意义的原始字节
        // 换成替代符，总好过一串数字
        let raw = RepoPath::from_bytes(vec![0xE4, 0xB8, 0xAD, 0xFF, b'.', b't']);
        assert_eq!(
            serde_json::to_value(&raw).unwrap(),
            serde_json::Value::String("中\u{FFFD}.t".to_owned())
        );
    }
}
