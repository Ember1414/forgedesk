# ForgeDesk

**ForgeDesk — A Git client**（可写作 **ForgeDesk for Git**）

一个开源、跨平台、可自由分发的 Git 图形化桌面工作台：把复杂的 Git 与 GitHub 终端操作
变成可视化界面、向导与可交互图表。

[![CI](https://github.com/Ember1414/forgedesk/actions/workflows/ci.yml/badge.svg)](https://github.com/Ember1414/forgedesk/actions/workflows/ci.yml)
![License](https://img.shields.io/badge/license-Apache--2.0-blue)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)

> 项目仍在**早期开发阶段**（M0：工程底座）。下面的"当前可用"是**已经能在本地跑起来的功能**，
> 其余都在路线图里，尚未实现。我们不会把"计划做的"写成"已经有的"。

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

## 截图

<!-- 原创截图占位：本阶段引用我们自己的图标与设计系统预览页；完整的界面截图在 M1 出现可用界面后补齐。 -->

![ForgeDesk 应用图标（原创设计）](docs/brand/icon-1024.png)

当前可查看自己的界面：

```bash
pnpm install && pnpm dev          # 浏览器预览前端（含设计系统页 /#/__dev__/design）
pnpm tauri dev                    # 完整桌面应用（外壳 + 本地设置 + 日志）
```

---

## 当前可用（M0）

| 能力         | 说明                                                                                                                      |
| ------------ | ------------------------------------------------------------------------------------------------------------------------- |
| 应用外壳     | 顶栏（仓库切换/全局搜索/账号/更新位）+ 可折叠侧栏 + 状态栏；20 条路由；键盘可达（跳转链接、`Ctrl/Cmd+K`、`Esc` 关闭浮层） |
| 组件库       | 基于 Radix 原语与语义 token 的一套界面组件（按钮/表单/浮层/表格/虚拟列表/空态…）                                          |
| 设计系统     | 明暗主题 + WCAG AA 对比度校验（`pnpm check:contrast`）                                                                    |
| 国际化       | 中英双语、命名空间词条、硬编码文案会被 `pnpm i18n:lint` 拦住                                                              |
| 统一错误模型 | 稳定错误码 + 可执行修复动作 + 脱敏详情；错误提示可直达相关日志                                                            |
| 本地存储     | SQLite（bundled）+ 版本化迁移（迁移前备份、可回滚）+ 设置持久化                                                           |
| 日志         | 文件日志（JSON、**已脱敏**）、按天与 10MB 双阈值轮转、保留 7 天；崩溃留档与会话标记                                       |
| 工程门禁     | lint / i18n / 类型 / 单测 / 对比度 / workflow / 仓库一致性 / 文档链接 / Rust fmt+clippy+test                              |

**尚未实现**（按里程碑推进，见 [`docs/PLAN.md`](docs/PLAN.md)）：Git 核心操作（提交、分支、同步）、
历史与图表、冲突与 rebase 向导、快照回滚、GitHub 集成、终端与编辑器、插件系统、自动更新。

---

## 安装

**发布版尚未提供。** 安装包（GitHub Releases、Scoop/Homebrew/Winget/Flathub 等）会在 M7/M8 完成后给出，
届时的安装说明将放在 `docs/install/`（现在还没有这份文档，所以这里不写"命令"）。

### 从源码构建

```bash
git clone https://github.com/Ember1414/forgedesk.git
cd forgedesk
pnpm install
pnpm tauri dev        # 开发运行
pnpm tauri build      # 打包（产物在 src-tauri/target/release/bundle）
```

依赖与三平台差异见 [`docs/DEV-ENV.md`](docs/DEV-ENV.md)（含 Windows 工具链脚本、国内镜像、常见陷阱）。

---

## 隐私

- **无 AI 功能**：应用不做任何模型推理，也不需要联网即可处理本地 Git 操作。
- **无遥测 SDK**：当前版本不收集、不上传任何使用数据。
- **凭据**：只存系统钥匙串（keyring），明文永不落盘；日志与错误详情在**写出前**统一脱敏。
- **日志**：本地文件，用户可随时打开目录查看或删除；绝不自动上传。

---

## 参与

- 贡献方式、提交规范与 PR 流程：[`CONTRIBUTING.md`](CONTRIBUTING.md)
- 架构与分层规则：[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- 编码风格：[`docs/CODING_STYLE.md`](docs/CODING_STYLE.md)
- 前后端契约：[`docs/API.md`](docs/API.md)
- 代理/自动化约定（本仓库的工程红线与质量门禁）：[`AGENTS.md`](AGENTS.md)

本项目规模较大，**由人类维护者负责方向与审批、AI 编码代理负责执行**；
欢迎以 issue（Bug / 功能建议 / RFC）形式参与讨论。

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
