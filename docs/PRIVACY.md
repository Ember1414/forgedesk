# 隐私政策（Privacy）

> 适用范围：ForgeDesk 桌面客户端。本文档是**完整版**；应用内「设置 → 隐私」提供同一承诺的速览与本文档的直达入口。
> 核心承诺：**本地优先、默认无遥测、无 AI 推理、凭据不出本机。**

---

## 1. 一句话摘要

ForgeDesk 处理本地 Git 仓库与你自己配置的代码托管服务（如 GitHub）。
它**不收集**你的代码内容，**不上传**任何使用数据，**不包含**任何 AI/机器学习推理功能，
在没有网络时本地 Git 功能 100% 可用。

---

## 2. 数据清单

| 数据 | 存放位置 | 用途 | 是否离开本机 |
| --- | --- | --- | --- |
| Git 对象与引用 | 你的仓库 `.git/`（不复制） | 唯一真实来源 | 否（除非你 push） |
| 仓库列表、设置、审计记录、快照元数据、API 缓存 | 应用数据目录（SQLite） | 需要查询与持久化 | 否 |
| 访问令牌（OAuth / PAT） | 系统凭据库（Windows 凭据管理器 / macOS Keychain / Linux Secret Service） | 向托管服务认证 | 仅作为请求头发往你配置的托管服务 |
| 应用日志 | 应用数据目录（本地文件） | 排查问题 | 否（**绝不自动上传**） |
| 快照的工作区大文件备份 | 应用缓存目录（可配置、可清理） | 支持一键回滚 | 否 |

**不收集**：代码内容、文件路径汇总、联系方式、设备指纹、使用行为统计。

---

## 3. 对外网络请求

应用只在以下情况发起网络请求，且目标由**你**指定或确认：

| 场景 | 目标 | 数据 |
| --- | --- | --- |
| Git 远端操作（fetch/pull/push） | 你的 remote 地址 | Git 协议数据 |
| 代码托管 API（仓库/PR/Issue/流水线） | 你配置的托管服务主机（默认 `api.github.com`） | 认证头 + 查询参数 |
| 自动更新检查 | 更新清单地址（项目托管在 Cloudflare Pages / `*.pages.dev`） | 当前版本、平台、架构 |
| 代理连通性测试 | 你配置的代理与目标 | 仅连通性探测 |

- 请求头**不包含**用户身份信息（除认证所必需）。
- Token 在写入日志前统一脱敏；`detail`/`stderr` 中的令牌模式会被剥离。
- SSH 使用系统 `~/.ssh` 与 ssh-agent，私钥**不复制、不存储**。

---

## 4. 遥测

- **默认关闭**，且当前版本**不含任何遥测/分析 SDK**（红线 R6）。
- 若未来引入自建遥测：必须默认关闭、可随时关闭、开启前展示"将发送的字段"预览，
  且不含唯一设备标识，仅粗粒度版本/平台信息。

---

## 5. 用户权利

| 权利 | 如何行使 |
| --- | --- |
| 查看数据 | 设置页可查看日志目录、账号、快照与审计记录 |
| 导出数据 | 审计/设置支持导出（JSON/CSV） |
| 删除数据 | 卸载或用设置页清理，或直接删除应用数据目录 |
| 断开云端 | 移除账号（删除 keyring 密文，不影响远端与已推送内容） |

---

## 6. 第三方

- 托管服务（GitHub 等）的数据处理受其各自隐私政策约束；ForgeDesk 只是**客户端**。
- 第三方依赖与许可证清单见 [`LICENSE-AUDIT.md`](LICENSE-AUDIT.md)。

---

## 7. 与产品边界的关系

ForgeDesk **不提供任何人工智能功能**：不做模型推理、不生成提交信息、不自动解决冲突。
开发过程使用 AI 辅助编码，但这不影响产品本身的零 AI、可离线特性。

---

## 8. 变更与联系

本政策如有变更会随版本更新并在 `CHANGELOG.md` 记录。问题请通过仓库 issue 反馈
（安全相关问题请走 [`../SECURITY.md`](../SECURITY.md)）。

---

## 9. Disclaimer / 免责声明

ForgeDesk is an independent, community-driven project. It is **not affiliated with,
endorsed by, or sponsored by** the Git project, the Software Freedom Conservancy,
GitHub, Inc., or the Tauri project.

- "Git" is a trademark of the Software Freedom Conservancy. ForgeDesk for Git is an
  independent Git client and is not produced or endorsed by the Git project.
- "GitHub" and the Octocat are trademarks of GitHub, Inc. ForgeDesk integrates with
  GitHub but is not affiliated with GitHub, Inc.
- "Tauri" is a project of the Tauri Programme within The Commons Conservancy.
  ForgeDesk is built with Tauri but is an independent project.

ForgeDesk contains **no AI features**: the shipped application performs no
machine-learning inference and works fully offline for local Git operations.
