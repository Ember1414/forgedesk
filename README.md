# ForgeDesk

**ForgeDesk — A Git client**（可写作 **ForgeDesk for Git**）

一个开源、跨平台、可自由分发的 Git 图形化桌面工作台：把复杂的 Git 与 GitHub 终端操作
变成可视化界面、向导与可交互图表。

[![CI](https://github.com/Ember1414/forgedesk/actions/workflows/ci.yml/badge.svg)](https://github.com/Ember1414/forgedesk/actions/workflows/ci.yml)
![License](https://img.shields.io/badge/license-Apache--2.0-blue)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)

> **项目状态**：里程碑 M0–M6 已完成（本地 Git 闭环 → 历史 DAG → 冲突/rebase/快照 →
> 代码托管集成 → 终端/诊断/编辑器 → 插件/主题/多平台/i18n）。
> **M7 进行中**：自动更新、安装包与发布流水线、文档与官网。
> 发布策略为 **Windows 优先**；macOS / Linux 真机验证按
> [`docs/adr/ADR-005`](docs/adr/ADR-005-defer-cross-platform-verification.md) 暂缓。
> 进度与计划见 [`docs/M7-PLAN.md`](docs/M7-PLAN.md)，逐版本变更见 [`CHANGELOG.md`](CHANGELOG.md)。

---

## 一句话价值

Git 的命令行能力很强，但**理解**成本很高：为什么这个提交不见了？这次 `rebase` 到底改了什么？

ForgeDesk 的重点不是把命令排成按钮，而是**把 Git 的内部状态变成可看、可推演、可回滚的东西**：

- **可推演**：破坏性操作先给计划预览与等价命令，再执行；
- **可回滚**：写操作前自动快照，失败可一键还原；
- **可理解**：冲突、rebase、历史以图形与向导呈现，而不是一堆报错文本。

技术栈：**Tauri 2 + Rust**（后端）· **React + TypeScript + Vite + Tailwind**（前端）。
三平台（Windows / macOS / Linux）同源构建，**零付费服务依赖**。

---

## 功能概览（M0–M6 已实现）

| 领域          | 能力                                                                                                      |
| ------------- | --------------------------------------------------------------------------------------------------------- |
| 仓库与工作区  | 打开 / 克隆 / 初始化、最近仓库、状态分组（已暂存/未暂存/未跟踪）、树与平铺视图、批量操作、万行虚拟列表    |
| Diff 与暂存   | 统一 / 并排 diff、hunk 折叠、行级与块级暂存（结果与 `git diff --cached` 逐字节一致）                      |
| 提交          | 两段式提交（预览 + 等价 git 命令 + 索引指纹防串改）、amend、钩子清单与拒绝时可跳过重试、提交前自动快照    |
| 历史          | DAG 提交图（Canvas、分支泳道、命中检测、性能面板）、提交详情、筛选与搜索、迷你地图                        |
| 分支与同步    | 分支 / 标签管理、远端管理、fetch / pull / push（被拒走 `--force-with-lease`，**无裸 `--force`**）         |
| 高级操作      | stash、cherry-pick、revert、reset（含计划预览）、reflog 恢复                                              |
| 冲突与 rebase | 三栏冲突解决向导（逐块采用/自定义）、拖拽式交互 rebase（reword/squash/fixup/drop/edit）                   |
| 安全网        | 操作快照 v2（含未跟踪文件备份）、一键回滚与回滚校验、操作历史时间线、破坏性操作安全测试矩阵               |
| 代码托管      | OAuth Device Flow 与 PAT 登录、多账号、仓库面板、拉取请求审查与合并、议题、流水线日志、限流降级           |
| 终端与编辑器  | 内嵌终端（xterm.js）、危险命令拦截、Git 命令解释词典、错误诊断引擎、Monaco 编辑器、命令面板与自定义快捷键 |
| 扩展与外观    | 插件沙箱（wasmi + 权限白名单）、插件管理页、示例插件与 SDK、主题导入导出、中英双语、代理与 SSH/GPG 设置   |
| 工程底座      | 统一错误模型、SQLite + 版本化迁移、脱敏文件日志、i18n、明暗主题与 WCAG AA 对比度门禁                      |

---

## 截图

<!-- 原创截图占位：完整的界面截图将在 M7 打包发布时补齐，此处引用我们自己的原创图标。 -->

![ForgeDesk 应用图标（原创设计）](docs/brand/icon-1024.png)

---

## 快速开始（从源码构建）

```bash
git clone https://github.com/Ember1414/forgedesk.git
cd forgedesk
pnpm install
pnpm dev              # 浏览器预览前端（含设计系统页 /#/__dev__/design）
pnpm tauri dev        # 完整桌面应用
pnpm tauri build      # 打包（本机平台；产物在 src-tauri/target/release/bundle）
```

依赖与三平台差异见 [`docs/DEV-ENV.md`](docs/DEV-ENV.md)（含 Windows 工具链脚本、国内镜像、常见陷阱）。

---

## 安装

**预编译发布版尚未提供**（M7 进行中，目标 Windows 首发）。发布后会给出：

- GitHub Releases 下载页与 SHA256/GPG 校验；
- Windows 安装包（NSIS/MSI）与**便携版 zip**、Scoop / Winget；
- macOS / Linux 分发随 M8 推进。

已提供的说明：[`docs/install/windows.md`](docs/install/windows.md)（含校验命令与 SmartScreen 指引）。

---

## 文档

| 文档                                                 | 内容                                 |
| ---------------------------------------------------- | ------------------------------------ |
| [`docs/PLAN.md`](docs/PLAN.md)                       | 项目计划书（范围、里程碑、验收标准） |
| [`docs/M7-PLAN.md`](docs/M7-PLAN.md)                 | M7 启动准备（Windows 优先）          |
| [`docs/manual/README.md`](docs/manual/README.md)     | 用户手册（核心工作流）               |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)       | 分层架构与依赖规则                   |
| [`docs/API.md`](docs/API.md)                         | 前后端 IPC 契约（命令与事件）        |
| [`docs/CODING_STYLE.md`](docs/CODING_STYLE.md)       | 编码风格                             |
| [`docs/PLUGIN-API.md`](docs/PLUGIN-API.md)           | 插件清单、权限与宿主 API             |
| [`docs/FAQ.md`](docs/FAQ.md)                         | 常见问题                             |
| [`docs/TROUBLESHOOTING.md`](docs/TROUBLESHOOTING.md) | 故障排查                             |
| [`docs/PRIVACY.md`](docs/PRIVACY.md)                 | 隐私政策与数据清单                   |
| [`docs/RELEASE.md`](docs/RELEASE.md)                 | 发布流程（密钥、步骤、回滚）         |
| [`docs/SIGNING.md`](docs/SIGNING.md)                 | 提交签名与验证                       |

---

## 隐私

- **无 AI 功能**：应用不做任何模型推理，也不需要联网即可处理本地 Git 操作。
- **无遥测 SDK**：当前版本不收集、不上传任何使用数据。
- **凭据**：只存系统钥匙串（keyring），明文永不落盘；日志与错误详情在**写出前**统一脱敏。
- **日志**：本地文件，用户可随时打开目录查看或删除；绝不自动上传。

完整说明（数据清单、对外请求、用户权利）见 [`docs/PRIVACY.md`](docs/PRIVACY.md)。

---

## 参与

- 贡献方式、提交规范与 PR 流程：[`CONTRIBUTING.md`](CONTRIBUTING.md)
- 行为准则：[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)
- 安全策略与漏洞报告：[`SECURITY.md`](SECURITY.md)
- 架构与分层规则：[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- 编码风格：[`docs/CODING_STYLE.md`](docs/CODING_STYLE.md)
- 前后端契约：[`docs/API.md`](docs/API.md)
- 代理/自动化约定（本仓库的工程红线与质量门禁）：[`AGENTS.md`](AGENTS.md)

本项目规模较大，**由人类维护者负责方向与审批、AI 编码代理负责执行**；
欢迎以 issue（Bug / 功能建议 / RFC / 插件问题）形式参与讨论。

---

## 许可证

[Apache License 2.0](LICENSE)。第三方依赖的许可证清单与合规策略见
[`docs/PLAN.md`](docs/PLAN.md) §9.4（`cargo-deny` 与许可证审计会在 CI 中强制执行）。

---

## Disclaimer / 免责声明

ForgeDesk is an independent, community-driven project. It is **not affiliated with,
endorsed by, or sponsored by** the Git project, the Software Freedom Conservancy,
GitHub, Inc., or the Tauri project.

- "Git" is a trademark of the Software Freedom Conservancy. ForgeDesk for Git is an
  independent Git client and is not produced or endorsed by the Git project.
- "GitHub" and the Octocat are trademarks of GitHub, Inc. ForgeDesk integrates with
  GitHub but is not affiliated with GitHub, Inc.
- "Tauri" is a project of the Tauri Programme within The Commons Conservancy.
  ForgeDesk is built with Tauri but is an independent project.

ForgeDesk contains **no AI features**. All code in this repository is developed with
AI-assisted tooling, but the shipped application performs no machine-learning
inference and works fully offline for local Git operations.
