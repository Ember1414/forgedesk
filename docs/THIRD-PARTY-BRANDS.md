# 第三方品牌资源登记（Third-Party Brands）

> 依据 [`AGENTS.md`](../AGENTS.md) 红线 R2 / R4：**应用图标必须 100% 原创**，
> 不得使用 Git / GitHub / Tauri 官方 Logo、Octocat 及其变体；产品名与包名不得包含
> "Git" / "GitHub" 字样。本文件记录**确需引用**的第三方品牌资源（如有）。
>
> 维护规则：引入任何第三方品牌资源（名称、图标、徽章、字体商标等）时，
> 先在此登记"用途 / 来源 / 许可依据"，PR 描述中说明必要性；未登记不得合入。

## 当前登记

**无。** 本仓库当前不包含任何第三方品牌资源：

| 类别 | 现状 |
| --- | --- |
| 应用图标与界面图形 | 100% 原创（源文件与设计说明见 [`BRAND.md`](./BRAND.md)、`docs/brand/`） |
| 竞品 UI / 配色 / 文案 | 未复制任何竞品（红线 R3；`pnpm compliance` 与人工评审把关，流程见 `.github/COMPETITOR-REVIEW.md`） |
| 第三方 Logo | 未使用 Git / GitHub / Tauri 官方 Logo，也未使用 Octocat 及其变体 |

## 名称的合规用法（与 [`README.md`](../README.md) 免责声明一致）

| 用法 | 允许 | 说明 |
| --- | --- | --- |
| 描述语 ``ForgeDesk — A Git client`` / ``ForgeDesk for Git`` | ✅ | 唯一允许的产品描述形式；对 "Git" 为**指称性使用**（说明兼容对象），不暗示官方关联 |
| 产品名 / 包名 / 可执行文件包含 "Git" 或 "GitHub" | ❌ | 红线 R4（如包管理器标识用 `ForgeDesk.ForgeDesk` / `forgedesk`） |
| 在文档中提及 Git 命令、GitHub API、Tauri 框架 | ✅ | 技术性提及属于正常指称，不构成品牌使用 |
| "GitHub" 兼容说明 | ✅ | 表述为"集成 GitHub / 与 GitHub API 交互"，禁止写"GitHub 官方""经 GitHub 认证"等暗示背书 |

## 相关文件

- 品牌与图标： [`BRAND.md`](./BRAND.md)、`docs/brand/`
- 合规检查（自动）：`pnpm compliance`（名称 / 免责声明 / 图标相似度 / 依赖许可 / AI 依赖）
- 竞品视觉人工评审：`.github/COMPETITOR-REVIEW.md`
- 免责声明全文：[`README.md`](../README.md) 的 Disclaimer 章节
