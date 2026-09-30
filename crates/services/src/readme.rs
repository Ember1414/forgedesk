//! README 的安全渲染（T4.6）：Markdown → 白名单化的 HTML。
//!
//! # 为什么在 Rust 侧渲染而不是前端
//!
//! README 是**不可信输入**（任何人都能往公开仓库里塞一段 `<script>`）。
//! 在后端用白名单清洗成"只含安全标签"的 HTML，前端拿到的东西从根上
//! 就不可能执行脚本——这比"前端拿到原始 Markdown 再自己想办法防 XSS"
//! 少一层信任假设，而且清洗规则可以在 Rust 单测里穷举攻击样本
//! （M4 验收：README 中恶意 HTML/脚本不执行）。
//!
//! # 白名单策略（docs/PLAN.md T4.6）
//!
//! - 保留：标题、段落、强调、列表、引用、代码块/行内代码、表格、
//!   链接与图片（http/https）；ammonia 的默认标签/属性白名单之上
//!   加 `img[src|alt|title]` 与表格类标签，`class` 只留给代码块的语言标记；
//! - 清除：`script`/`style`/`iframe`/`object`/`form` 等一切活动内容、
//!   所有 `on*` 事件属性、`javascript:`/`data:` 协议的 URL
//!   （ammonia 默认协议表即 http/https/mailto）；
//! - 链接：统一加 `rel="noopener noreferrer"`（防 reverse tabnabbing），
//!   前端对外链的点击经委托拦截复制，不在 webview 内导航。
//!
//! # 为什么不用 `data:` 图片
//!
//! data URI 可以绕过 referer/审计，且是构造型攻击的常见载体；
//! CSP 虽然放行 `img-src data:`（打包器需要），渲染层仍一律拒绝。

use ammonia::Builder;
use pulldown_cmark::{html, Options, Parser};

/// 把不可信的 Markdown 渲染成**白名单化**的 HTML 片段（无 `<html>` 外壳）。
#[must_use]
pub fn render_readme(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let raw = {
        let parser = Parser::new_ext(markdown, options);
        let mut out = String::with_capacity(markdown.len() * 3 / 2);
        html::push_html(&mut out, parser);
        out
    };
    builder().clean(&raw).to_string()
}

/// 白名单构建器（集中一处，测试与渲染共用同一套规则）。
fn builder() -> Builder<'static> {
    let mut builder = Builder::default();
    // 代码高亮类只保留语言标记（如 class="language-rust"）：
    // allowed_classes 会整体替换 class 策略，其他标签的 class 一律剥掉
    let mut code_classes = std::collections::HashSet::new();
    code_classes.insert("language-");
    let mut classes = std::collections::HashMap::new();
    classes.insert("code", code_classes);
    builder.allowed_classes(classes);
    builder.add_tag_attributes("img", ["alt", "title", "width", "height"]);
    builder.add_generic_attributes(["align"]);
    builder.url_relative(ammonia::UrlRelative::PassThrough);
    builder.link_rel(Some("noopener noreferrer"));
    builder
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::render_readme;

    /// M4 验收的 XSS 用例集合：每一条都必须被"无害化"，
    /// 断言方式是"输出里不出现攻击载荷"，而不是"输出等于某个快照"。
    #[test]
    fn active_content_is_stripped_from_every_attack_sample() {
        let samples = [
            ("<script>alert('xss')</script>", "<script"),
            ("<img src=x onerror=alert(1)>", "onerror"),
            ("<a href=\"javascript:alert(1)\">click</a>", "javascript:"),
            ("<iframe src=\"https://evil.example\"></iframe>", "<iframe"),
            ("<object data=\"https://evil.example\"></object>", "<object"),
            ("<svg onload=alert(1)></svg>", "onload"),
            (
                "<form action=\"https://evil.example\"><input></form>",
                "<form",
            ),
            (
                "<style>body{background:url(javascript:alert(1))}</style>",
                "<style",
            ),
            (
                "<a href=\"data:text/html,<script>alert(1)</script>\">d</a>",
                "data:",
            ),
            (
                "<img src=\"https://ok.example/x.png\" onmouseover=\"alert(1)\">",
                "onmouseover",
            ),
        ];
        for (markdown, payload) in samples {
            let html = render_readme(markdown);
            assert!(
                !html
                    .to_ascii_lowercase()
                    .contains(&payload.to_ascii_lowercase()),
                "样本 {markdown:?} 的输出泄露了载荷 {payload:?}：{html}"
            );
        }
    }

    #[test]
    fn normal_markdown_survives_with_expected_tags() {
        let html = render_readme(
            "# ForgeDesk\n\n一些 **加粗** 与 [链接](https://github.com)。\n\n\
             ```rust\nfn main() {}\n```\n",
        );

        assert!(html.contains("<h1>"));
        assert!(html.contains("<strong>加粗</strong>"));
        assert!(html.contains("href=\"https://github.com\""));
        assert!(html.contains("rel=\"noopener noreferrer\""));
        assert!(html.contains("<pre>"), "代码块保留：{html}");
    }

    #[test]
    fn tables_and_images_render_but_image_urls_are_scheme_limited() {
        let html = render_readme(
            "| a | b |\n| - | - |\n| 1 | 2 |\n\n\
             ![徽标](https://img.example/badge.svg)\n\n\
             ![注入](javascript:alert(1))\n",
        );

        assert!(html.contains("<table>"), "表格支持：{html}");
        assert!(html.contains("src=\"https://img.example/badge.svg\""));
        // javascript: 协议在 ammonia 里连同属性一起被剥掉
        assert!(!html.contains("javascript:"));
    }

    #[test]
    fn relative_urls_pass_through_and_get_the_safe_rel() {
        let html = render_readme("[文档](docs/README.zh.md)");

        assert!(html.contains("href=\"docs/README.zh.md\""), "{html}");
        assert!(html.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn empty_and_plain_text_inputs_are_tolerated() {
        assert_eq!(render_readme(""), "");
        let html = render_readme("只是一段话");
        assert!(html.contains("<p>只是一段话</p>"), "{html}");
    }
}
