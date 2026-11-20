# 文档索引

产品介绍与下载见[项目首页](../README.md)。本文按用途列出当前维护的文档。

## 安装与使用

- [Windows 安装与校验](install/windows.md) / [macOS 安装与校验](install/macos.md)
- [用户手册](manual/README.md)：仓库、提交、同步、冲突、恢复、GitHub 与工具
- [常见问题](FAQ.md) / [故障排查](TROUBLESHOOTING.md)
- [提交签名](SIGNING.md) / [隐私政策](PRIVACY.md) / [安全报告](../SECURITY.md)

## 参与开发

- [贡献流程](../CONTRIBUTING.md) / [开发环境](DEV-ENV.md)
- [架构](ARCHITECTURE.md) / [IPC 契约](API.md) / [编码风格](CODING_STYLE.md)
- [插件 API](PLUGIN-API.md) / [国际化术语](I18N-GLOSSARY.md)
- [维护清单](MAINTENANCE.md)：已知限制、待办与验证边界
- [发布流程](RELEASE.md) / [包管理器清单](../packaging/README.md)
- [社区说明](COMMUNITY.md) / [入门任务](GOOD-FIRST-ISSUES.md)

## 决策与审计

- [项目范围与计划](PLAN.md) / [编码代理约定](../AGENTS.md)
- [公开与供应链决策](OPEN-SOURCE-READINESS.md) / [依赖安全审计](SECURITY-AUDIT.md)
- [依赖许可证清单](LICENSE-AUDIT.md) / [品牌约定](BRAND.md) / [第三方品牌登记](THIRD-PARTY-BRANDS.md)
- 架构决策：[CI](adr/ADR-002-private-first-two-phase-ci.md)、[站点托管](adr/ADR-003-cloudflare-instead-of-github-pages.md)、[平台范围](adr/ADR-005-defer-cross-platform-verification.md)

## 历史证据

以下文档记录特定日期的结果，不表示当前版本的验收状态：

- [2026-11-20 项目审查](PROJECT-REVIEW-2026-11-20.md)与[修复记录](FIXES-2026-11-20.md)
- [性能基线](PERF-BASELINE.md) / [Git 引擎差分](GIT-ENGINE-DIFF.md) / [PTY 技术验证](PTY-SPIKE.md)
- [M7 验收](acceptance/M7.md) / [M8 验收及分发剩余项](acceptance/M8.md)，其他里程碑记录在同目录
- [原始任务提示词](AGENT-PROMPTS.md)：仅供回溯任务来源，新增工作遵循当前架构与编码约定

已删除重复的 M7/M8 执行计划及过期的十月审查清单；有效结论迁入验收记录、维护清单和当前文档。
原文可在 Git 历史中查询。
