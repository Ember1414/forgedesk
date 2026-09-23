# 更新日

本项目的版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)；
格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)

## [[0.0.]] - 2026-09-2

M0 里程碑：工程地基与首个可打包的空壳应用。此版本仅供开发与验收，不对外分发

### 新

- 工程骨架：Tauri 2 + React 19 + TypeScript + Vite + Tailwind 4 前端，
  Rust workspace（12 个分层 crate）`pnpm tauri dev`` 可直接启动桌面窗口
- 应用外壳：顶栏（仓库切换器 / 全局搜索 / 账号 / 更新位）、可折叠侧栏、状态栏，
  20 条路由覆盖仪表盘、仓库五页、代码托管、插件与设置；全部页面文案走 i18n（中/英）
- 设计系统：语义 token（色彩 / 字号 / 间距 / 圆角）+ 明暗主题 + WCAG AA 对比度校验，
  29 个基于 Radix 原语的基础组件与组件展示页
- 统一错误模型：稳定错误码、脱敏详情、可执行修复动作与统一 Toast 呈现
- 本地存储：SQLite（bundled）+ 版本化迁移（迁移前备份、可回滚）+ 设置持久化
- 日志系统：JSON 文件日志（写入前脱敏）、按天与 10MB 双阈值轮转、保留 7 天、
  崩溃留档（panic 日志）与会话标记；设置页可查看日志目录与最近日志
- 工程门禁：lint / i18n / 类型 / 单测 / 对比度 / workflow / 仓库一致性 / 文档链接 /
  合规红线（名称、免责声明、图标、依赖许可、AI 依赖）共 12 项，全部接入本地与 CI
- CI：Linux 质量门禁随每次推送；三平台打包矩阵在打 tag 时触发，
  产物按 ForgeDesk_版本_平台_架构 归一化命名并附 SHA256SUMS
- 文档：ARCHITECTURE / API / CODING_STYLE / CONTRIBUTING / DEV-ENV / README（含免责声明）

### 安

- 令牌 / 密码 / 私钥写入日志前统一脱敏（写入层强制，含 JSON 字段与 panic 报告）
- 固定已知的传递依赖漏洞：ansi-regex@<5.0.1 升至 5.0.1（GHSA-93q8-gq69-wqmw）

[0.0.1]: https://github.com/Ember1414/forgedesk/releases/tag/v0.0.1
