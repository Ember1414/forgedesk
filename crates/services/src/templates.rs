//! 初始化仓库时可生成的文件模板（`.gitignore` 与 `LICENSE`）。
//!
//! # 为什么模板随二进制走（`include_str!`）
//!
//! 模板若放在磁盘上，就会出现"用户机器上缺文件"或"模板被改动后与代码不匹配"
//! 两类问题（`crates/storage` 的迁移脚本出于同样的理由用 `include_str!`）。
//! 代价是改模板要重新编译，对随版本发布的固定文本来说完全可接受。
//!
//! # 立场：只生成文件，不替用户做决定
//!
//! `LICENSE` 模板只是把**用户选定的**许可证原文写进文件，并填入年份与版权持有者。
//! 本模块不推荐许可证、不做法律判断，也不修改 `package.json` / `Cargo.toml`
//! 里的许可声明——那些属于用户的决定。

/// 可生成的 `.gitignore` 模板（按语言）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitignoreTemplate {
    /// Rust（`target/` 等）。
    Rust,
    /// Node.js / 前端（`node_modules/`、`dist/` 等）。
    Node,
    /// Python（`__pycache__/`、虚拟环境等）。
    Python,
    /// Go（`/bin/`、`vendor/` 等）。
    Go,
    /// Java / JVM（`target/`、`.gradle/` 等）。
    Java,
}

impl GitignoreTemplate {
    /// 全部模板（前端据此渲染下拉框）。
    pub const ALL: &'static [Self] = &[Self::Rust, Self::Node, Self::Python, Self::Go, Self::Java];

    /// 稳定标识（IPC 参数与 i18n key 都用它）。
    pub const fn id(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Node => "node",
            Self::Python => "python",
            Self::Go => "go",
            Self::Java => "java",
        }
    }

    /// 从外部字符串解析（大小写不敏感）。
    ///
    /// 返回 `None` 时调用方必须报 `VALIDATION`：外部输入一律不可信，
    /// 不能悄悄退化成某个默认模板——那会让用户拿到一份与预期无关的忽略规则。
    pub fn parse(value: &str) -> Option<Self> {
        let needle = value.trim();
        Self::ALL
            .iter()
            .copied()
            .find(|template| template.id().eq_ignore_ascii_case(needle))
    }

    /// 模板内容。
    pub const fn contents(self) -> &'static str {
        match self {
            Self::Rust => include_str!("../templates/gitignore/rust.gitignore"),
            Self::Node => include_str!("../templates/gitignore/node.gitignore"),
            Self::Python => include_str!("../templates/gitignore/python.gitignore"),
            Self::Go => include_str!("../templates/gitignore/go.gitignore"),
            Self::Java => include_str!("../templates/gitignore/java.gitignore"),
        }
    }
}

/// 可生成的许可证模板。
///
/// 只收录**原文固定、无需参数**的常见许可证。像 `GPL-3.0` 那样需要用户在
/// "是否附加 `or later`"上做选择的，等有明确需求时再单独处理——现在给一个
/// 猜出来的版本比不给更糟。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LicenseTemplate {
    /// MIT。
    Mit,
    /// Apache License 2.0。
    Apache2,
    /// BSD 3-Clause。
    Bsd3,
}

impl LicenseTemplate {
    /// 全部模板。
    pub const ALL: &'static [Self] = &[Self::Mit, Self::Apache2, Self::Bsd3];

    /// 稳定标识（同时用作 `spdx` 风格的文件名）。
    pub const fn id(self) -> &'static str {
        match self {
            Self::Mit => "MIT",
            Self::Apache2 => "Apache-2.0",
            Self::Bsd3 => "BSD-3-Clause",
        }
    }

    /// 从外部字符串解析（大小写不敏感）。
    pub fn parse(value: &str) -> Option<Self> {
        let needle = value.trim();
        Self::ALL
            .iter()
            .copied()
            .find(|template| template.id().eq_ignore_ascii_case(needle))
    }

    /// 模板原文（含 `{{year}}` / `{{holder}}` 占位符）。
    pub const fn template(self) -> &'static str {
        match self {
            Self::Mit => include_str!("../templates/licenses/MIT.txt"),
            Self::Apache2 => include_str!("../templates/licenses/Apache-2.0.txt"),
            Self::Bsd3 => include_str!("../templates/licenses/BSD-3-Clause.txt"),
        }
    }

    /// 填入年份与版权持有者。
    ///
    /// `holder` 为空时保留一个明确的占位符（`<copyright holder>`）而不是留空：
    /// 留空会让生成出来的 `LICENSE` 看起来像"没人拥有版权"，
    /// 而占位符会让用户一眼看出还需要填写。
    pub fn render(self, year: u32, holder: &str) -> String {
        let holder = holder.trim();
        let holder = if holder.is_empty() {
            "<copyright holder>"
        } else {
            holder
        };
        self.template()
            .replace("{{year}}", &year.to_string())
            .replace("{{holder}}", holder)
    }
}

/// 把任意用户输入清理成"可以安全写进 LICENSE 的一行"。
///
/// 为什么需要：版权持有者来自 IPC，可能带换行或控制字符。写进 `LICENSE` 后
/// 换行会把一行版权声明拆成两行，而控制字符在编辑器里不可见——
/// 两者都会让这份文件在法律意义上变得可疑。
pub fn sanitize_holder(holder: &str) -> String {
    holder
        .chars()
        .map(|character| match character {
            '\r' | '\n' | '\t' => ' ',
            other if other.is_control() => ' ',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{sanitize_holder, GitignoreTemplate, LicenseTemplate};

    #[test]
    fn gitignore_ids_round_trip_and_are_case_insensitive() {
        for template in GitignoreTemplate::ALL {
            assert_eq!(GitignoreTemplate::parse(template.id()), Some(*template));
            assert_eq!(
                GitignoreTemplate::parse(&template.id().to_uppercase()),
                Some(*template)
            );
        }
        assert_eq!(GitignoreTemplate::parse("cobol"), None);
        assert_eq!(GitignoreTemplate::parse(""), None);
    }

    #[test]
    fn every_gitignore_template_has_content_and_a_trailing_newline() {
        for template in GitignoreTemplate::ALL {
            let contents = template.contents();
            assert!(
                contents.len() > 20,
                "{} 模板内容过短：{contents:?}",
                template.id()
            );
            assert!(
                contents.ends_with('\n'),
                "{} 模板必须以换行结尾（否则 git 会提示文件末尾无换行）",
                template.id()
            );
        }
    }

    #[test]
    fn rust_template_ignores_the_build_directory() {
        assert!(GitignoreTemplate::Rust.contents().contains("/target/"));
    }

    #[test]
    fn license_ids_round_trip() {
        for template in LicenseTemplate::ALL {
            assert_eq!(LicenseTemplate::parse(template.id()), Some(*template));
            assert_eq!(
                LicenseTemplate::parse(&template.id().to_lowercase()),
                Some(*template)
            );
        }
        assert_eq!(LicenseTemplate::parse("WTFPL"), None);
    }

    #[test]
    fn rendering_fills_both_placeholders_and_leaves_none_behind() {
        for template in LicenseTemplate::ALL {
            let rendered = template.render(2026, "ForgeDesk contributors");

            assert!(
                !rendered.contains("{{"),
                "{} 渲染后仍有未替换的占位符",
                template.id()
            );
            assert!(
                rendered.contains("2026"),
                "{} 渲染后应含年份",
                template.id()
            );
            assert!(
                rendered.contains("ForgeDesk contributors"),
                "{} 渲染后应含版权持有者",
                template.id()
            );
        }
    }

    #[test]
    fn an_empty_holder_becomes_an_obvious_placeholder() {
        let rendered = LicenseTemplate::Mit.render(2026, "   ");
        assert!(rendered.contains("<copyright holder>"));
        assert!(!rendered.contains("{{holder}}"));
    }

    #[test]
    fn holder_is_collapsed_to_a_single_safe_line() {
        assert_eq!(sanitize_holder("  Ada  Lovelace \n"), "Ada Lovelace");
        assert_eq!(sanitize_holder("a\nb"), "a b", "换行会把版权行拆成两行");
        assert_eq!(
            sanitize_holder("a\u{0}b"),
            "a b",
            "控制字符不可见，必须去掉"
        );
        assert_eq!(sanitize_holder("\t"), "");
    }
}
