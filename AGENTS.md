# AGENTS.md — ForgeDesk 编码代理公约

> 本文件是**所有 AI 编码代理进入本仓库后必须首先阅读并遵守**的最高优先级工程约定。
> 人类维护者在本项目中只负责**审批**与**方向决策**，其余执行工作由代理完成。
> 完整计划见 `docs/PLAN.md`；逐任务提示词见 `docs/AGENT-PROMPTS.md`。

---

## 1. 项目身份（一句话）

ForgeDesk 是一个**开源、跨平台、可自由分发的 Git/GitHub 图形化桌面工作台**，把复杂的 Git 与 GitHub 终端操作变成可视化界面、向导与可交互图表。

技术栈：**Tauri 2 + Rust**（后端） / **React + TypeScript + Vite + Tailwind + shadcn/ui**（前端）。

---

## 2. 不可违反的红线（违反即视为任务失败）

| # | 红线 | 说明 |
| --- | --- | --- |
| R1 | **产品不得包含任何 AI / 机器学习推理功能** | 不引入 LLM、模型权重、推理服务；不实现"AI 生成提交信息""AI 解决冲突"等。AI 只用于开发阶段写代码。 |
| R2 | **不得使用 Git / GitHub / Tauri 官方 Logo 或 Octocat 及其变体** | 应用图标必须 100% 原创。第三方品牌资源使用需登记到 `docs/THIRD-PARTY-BRANDS.md`。 |
| R3 | **不得复制任何竞品（GitHub Desktop / GitKraken / Sourcetree / Fork / Git-cola）的 UI 布局、配色、图标、文案** | 所有界面原创设计。禁止反编译或复制竞品代码。 |
| R4 | **产品名与包名不得包含 "Git" / "GitHub" 字样** | 统一描述语：`ForgeDesk — A Git client` 或 `ForgeDesk for Git`。 |
| R5 | **不得使用任何付费服务或付费账号** | 本项目为**零成本方案**：不用付费代码签名证书、不用付费商店、不用付费 CDN/域名。 |
| R6 | **不得添加任何第三方分析/遥测 SDK** | 遥测默认关闭且必须自建、可关闭、可预览发送内容（M7 前不做遥测）。 |
| R7 | **破坏性 Git 操作必须有安全网** | 一律经"计划预览 → 快照 → 执行 → 可回滚"；`push` 只允许 `--force-with-lease`，**禁止裸 `--force`**。 |
| R8 | **Token / 密码 / 私钥不得写入日志、崩溃报告、遥测或仓库** | 仅存系统 keyring；日志必须脱敏。 |

---

## 3. 每次任务的固定动作

```text
① 读：AGENTS.md（本文件）+ docs/PLAN.md 对应章节 + docs/ARCHITECTURE.md + docs/API.md（若存在）
② 计划：先给出 3–8 步执行计划与"假设"清单（若需求含糊，做最小合理假设并标注，不要停下来等）
③ 实现：小步提交，Conventional Commits（feat/fix/chore/docs/test/refactor）
④ 自检：运行下方"质量门禁"全部命令，必须全绿
⑤ 回报：按第 5 节格式输出，标注是否需要人类审批
```

---

## 4. 质量门禁（每次提交前必须全部通过）

```bash
pnpm lint                 # 前端 lint
pnpm typecheck            # TS 类型检查
pnpm test                 # Vitest
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

补充要求：

- 新增的 Tauri Command **必须**登记到 `docs/API.md`，且参数在后端二次校验。
- 新增的 Rust 领域逻辑**必须**有单测；新增核心前端组件**必须**有 Vitest 测试。
- 覆盖率底线：Rust 整体 ≥ 60%、`domain` crate ≥ 85%、前端关键模块 ≥ 70%。
- 前端 E2E 必须断言 `window.__errs` 为空（未捕获错误集合）。

---

## 5. 回报格式（每次任务结束必须按此输出）

```markdown
## 任务：<任务ID 与标题>
### 状态
完成 / 部分完成（说明剩余） / 阻塞（说明原因）
### 变更
- <文件路径>：<做了什么>
### 验证证据
- `pnpm lint` → 通过
- `cargo test -p git-engine` → 42 passed
- 手工/自动化验证：<具体操作与结果>
### 假设（若有）
- 假设：<内容>，原因：<原因>，影响：<影响范围>
### 需要人类审批（若有）
- [ ] <审批项 1：说明需要决定什么，选项 A/B 与建议>
### 风险与后续
- <未覆盖的边缘情况 / 建议的下一步>
```

---

## 6. 架构约定（不得擅自破坏）

| 约定 | 说明 |
| --- | --- |
| 前后端严格分层 | 前端**不得**直接访问文件系统或执行命令，一律经 Tauri Command |
| 领域层纯函数化 | `crates/domain` 不依赖 IO，全部可单测；IO 放 `crates/*/infra` |
| Git 引擎可替换 | 业务层只依赖 `GitEngine` trait；读走 libgit2、写走系统 git CLI |
| 单一写入口 | 所有改仓库状态的操作必须经 `SnapshotManager` + `AuditLog` |
| 长任务可取消 | > 500ms 的操作必须走 `JobRunner`，支持进度事件与 `CancellationToken` |
| 单一真相源 | Git 状态不进 Zustand；服务端状态用 TanStack Query，UI 状态用 Zustand |
| 前端零信任 | 所有外部输入（路径、URL、API 响应、插件输出）后端必须校验 |
| 文案全 i18n | 所有用户可见文案走 i18n key，中英文同步；禁止硬编码中文/英文文案 |

---

## 7. CLI 调用规范（Git 相关）

- 一律使用参数数组调用（`Command::args`），**禁止** shell 字符串拼接。
- 环境固定：`LC_ALL=C`、`GIT_TERMINAL_PROMPT=0`，非交互执行。
- 输出必须使用机器可解析格式：
  - `git status --porcelain=v2 -z`
  - `git diff --numstat -z`
  - `git log --format=...%x1f...%x1e`
  - 禁止解析人类可读输出。
- 所有调用需超时与取消支持，stderr 需捕获并交由 `crates/diagnostics` 解析。

---

## 8. 依赖与供应链

- 禁止引入 GPL/AGPL（除 libgit2 的链接例外）依赖；`cargo-deny` 必须通过。
- 锁定 `Cargo.lock` 与 `pnpm-lock.yaml`；GitHub Actions 一律固定到 commit SHA。
- 新增依赖需在 PR 描述中说明理由、体积影响与替代方案。

---

## 9. 免费方案特别约定（R5 展开）

| 领域 | 允许（免费） | 禁止（付费） |
| --- | --- | --- |
| CI/CD | GitHub Actions（公共仓库免费额度） | 付费 CI、付费构建机 |
| 分发 | GitHub Releases、GitHub Pages、Homebrew/Scoop/Winget/Flathub/Snap/AUR、Itch.io | Microsoft Store、Steam、付费 CDN |
| macOS 信任 | ad-hoc 签名 + 去隔离指引 + 首次启动引导 | Apple Developer Program 公证 |
| Windows 信任 | SHA256 + GPG + 便携版 + 包管理器分发 + 申请 OSS 免费签名计划 | OV/EV 付费证书 |
| 域名 | `*.github.io` / `*.pages.dev` | 付费域名（除非人类另行决定） |
| 更新 | 自签名 Ed25519（minisign）清单托管在 Pages | 付费更新服务 |

---

## 10. 何时必须停下来请求人类审批

只有以下情况才阻塞并请求审批（其余一律自行决策并标注假设后继续）：

1. 需要花钱（域名、证书、商店、云资源）。
2. 需要注册/登录第三方账号，或向第三方仓库（如 `microsoft/winget-pkgs`、`homebrew`、`flathub`）提交 PR。
3. 需要修改 `docs/PLAN.md` 中的范围定义（新增功能、改动里程碑、突破"不做清单"）。
4. 需要引入新的技术栈或替换核心选型（框架、Git 引擎、可视化方案）。
5. 需要执行不可逆操作（删除分支/tag、覆盖发布产物、强推远程仓库）。
6. 发现可能损伤用户仓库数据的问题（P0 级 Bug）。
