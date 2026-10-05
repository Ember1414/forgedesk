//! 路径规范化与比较（T6.9）。
//!
//! # 为什么"路径相等"不是一个简单判断
//!
//! 三条平台差异会让 `Path::eq` 给出错误答案：
//!
//! 1. **Windows 大小写不敏感**：`C:\Repo` 与 `c:\repo` 是同一个目录，
//!    但字符串/组件比较认为它们不同——重复打开检测、仓库去重会失效。
//! 2. **Windows 长路径与 UNC 前缀**：`\\?\C:\a\b` 与 `C:\a\b` 指向同一文件；
//!    启用长路径支持后某些 API 会返回带前缀的形式，不剥离就无法比较。
//! 3. **macOS 文件系统保存 NFD**：`café` 在 APFS/HFS+ 上是 `e` + 组合附加符，
//!    而程序内部通常写 NFC；不做 NFC 归一化，"同一个文件"比较会静默失败。
//!
//! 因此比较永远走 [`PathNormalizer::canonical_key`]：分隔符统一为 `/`、
//! NFC 归一化（可选）、大小写折叠（可选）、剥离长路径前缀。
//! 生成"真正传给操作系统"的路径用 [`PathNormalizer::ensure_extended_length`]。

use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

/// 平台路径策略。用 [`PathNormalizer::for_current_platform`] 取当前平台的配置；
/// 测试与跨平台逻辑可以手工构造任意配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathNormalizer {
    /// 大小写不敏感（Windows / macOS 默认文件系统）。
    pub case_insensitive: bool,
    /// 比较前做 Unicode NFC 归一化（macOS）。
    pub normalize_unicode: bool,
    /// 支持长路径前缀（Windows `\\?\`）。
    pub long_path_prefix: bool,
}

/// Windows 上超过此长度的路径建议加 `\\?\` 前缀。
///
/// MAX_PATH 是 260；留出余量，避免在 255 左右反复横跳。
pub const EXTENDED_PATH_THRESHOLD: usize = 240;

impl PathNormalizer {
    /// 当前平台的策略。
    pub fn for_current_platform() -> Self {
        Self {
            case_insensitive: cfg!(windows),
            normalize_unicode: cfg!(target_os = "macos"),
            long_path_prefix: cfg!(windows),
        }
    }

    /// Windows 策略（测试用）。
    pub fn windows() -> Self {
        Self {
            case_insensitive: true,
            normalize_unicode: false,
            long_path_prefix: true,
        }
    }

    /// macOS 策略（测试用）。
    pub fn macos() -> Self {
        Self {
            case_insensitive: false,
            normalize_unicode: true,
            long_path_prefix: false,
        }
    }

    /// Linux / BSD 策略（测试用）：大小写敏感、无前缀。
    pub fn linux() -> Self {
        Self {
            case_insensitive: false,
            normalize_unicode: false,
            long_path_prefix: false,
        }
    }

    /// 生成用于**比较**的规范键（不要拿它当真实路径传给文件系统）。
    pub fn canonical_key(&self, path: &Path) -> String {
        let text = path.to_string_lossy().replace('\\', "/");
        let stripped = if self.long_path_prefix {
            strip_extended_prefix(&text)
        } else {
            text.as_str()
        };
        let normalized = if self.normalize_unicode {
            stripped.nfc().collect::<String>()
        } else {
            stripped.to_owned()
        };
        if self.case_insensitive {
            // 大小写折叠用 to_lowercase：路径比较要的是"看起来同一个目录"，
            // 不需要严格 Unicode 大小写等价（简单折叠与 NTFS 的语义一致）
            normalized.to_lowercase()
        } else {
            normalized
        }
    }

    /// 为超长路径加 Windows 扩展前缀（非 Windows 或不需要时原样返回）。
    ///
    /// 为什么不总是加前缀：`\\?\` 会关闭路径规范化（`..`、`/` 不再被展开），
    /// 短路径加了反而容易踩坑——只在接近 MAX_PATH 时才加。
    pub fn ensure_extended_length(&self, path: &Path) -> PathBuf {
        if !self.long_path_prefix || !cfg!(windows) {
            return path.to_path_buf();
        }
        let text = path.as_os_str().to_string_lossy();
        if text.starts_with(r"\\?\") || text.starts_with(r"\\.\") {
            return path.to_path_buf();
        }
        if text.chars().count() < EXTENDED_PATH_THRESHOLD {
            return path.to_path_buf();
        }
        if let Some(unc) = text.strip_prefix(r"\\") {
            // UNC：\\server\share\... → \\?\UNC\server\share\...
            return PathBuf::from(format!(r"\\?\UNC\{unc}"));
        }
        PathBuf::from(format!(r"\\?\{text}"))
    }
}

/// 剥离 `\\?\` / `\\?\UNC\` / `\\.\` 前缀与裸 UNC 的 `//` 开头
/// （输入已统一为 `/` 分隔符；UNC `//server/share` 与 `//?/UNC/server/share`
/// 必须落到同一个比较键）。
fn strip_extended_prefix(text: &str) -> &str {
    if let Some(rest) = text.strip_prefix("//?/UNC/") {
        return rest;
    }
    if let Some(rest) = text.strip_prefix("//?/") {
        return rest;
    }
    let without_device = text.strip_prefix("//./").unwrap_or(text);
    // 剩下以 // 开头的只可能是 UNC 根（驱动器路径形如 C:/...，不带前导 //）
    without_device.strip_prefix("//").unwrap_or(without_device)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_are_case_insensitive_and_separator_agnostic() {
        let normalizer = PathNormalizer::windows();
        assert_eq!(
            normalizer.canonical_key(Path::new(r"C:\Repo\Sub")),
            normalizer.canonical_key(Path::new(r"c:/repo/sub"))
        );
    }

    #[test]
    fn the_extended_prefix_is_stripped_before_comparison() {
        let normalizer = PathNormalizer::windows();
        assert_eq!(
            normalizer.canonical_key(Path::new(r"\\?\C:\Repo\Sub")),
            normalizer.canonical_key(Path::new(r"C:\Repo\Sub"))
        );
        assert_eq!(
            normalizer.canonical_key(Path::new(r"\\?\UNC\server\share\a")),
            normalizer.canonical_key(Path::new(r"\\server\share\a"))
        );
    }

    #[test]
    fn macos_paths_compare_equal_across_nfc_and_nfd() {
        let normalizer = PathNormalizer::macos();
        // NFC：U+00E9（é 单字符）；NFD：e + U+0301（组合符）
        let nfc = Path::new("caf\u{00e9}.txt");
        let nfd = Path::new("cafe\u{0301}.txt");
        assert_ne!(nfc, nfd, "原始 Path 必须不等，否则测试没有意义");
        assert_eq!(normalizer.canonical_key(nfc), normalizer.canonical_key(nfd));
    }

    #[test]
    fn linux_paths_stay_case_sensitive() {
        let normalizer = PathNormalizer::linux();
        assert_ne!(
            normalizer.canonical_key(Path::new("/repo/README.md")),
            normalizer.canonical_key(Path::new("/repo/readme.md"))
        );
    }

    #[test]
    fn only_long_paths_get_the_extended_prefix() {
        let normalizer = PathNormalizer::windows();
        let short = Path::new(r"C:\Repo");
        assert_eq!(
            normalizer.ensure_extended_length(short),
            short.to_path_buf()
        );

        let deep = PathBuf::from(format!(r"C:\{}", "a".repeat(250)));
        let extended = normalizer.ensure_extended_length(&deep);
        assert!(extended.to_string_lossy().starts_with(r"\\?\"));

        // 已经带前缀的不再重复加
        assert_eq!(
            normalizer.ensure_extended_length(&extended),
            extended,
            "重复加前缀会破坏路径"
        );
    }

    #[test]
    fn unc_paths_use_the_unc_form_of_the_prefix() {
        let normalizer = PathNormalizer::windows();
        let unc = PathBuf::from(format!(r"\\server\share\{}", "a".repeat(240)));
        let extended = normalizer.ensure_extended_length(&unc);
        assert!(extended.to_string_lossy().starts_with(r"\\?\UNC\server"));
    }

    #[test]
    fn non_windows_normalizer_never_touches_the_path() {
        let normalizer = PathNormalizer::linux();
        let deep = PathBuf::from(format!("/{}", "a".repeat(250)));
        assert_eq!(normalizer.ensure_extended_length(&deep), deep);
    }
}
