# ForgeDesk 架构

> 本文件说明**代码实际如何分层、数据如何流动、在哪里扩展**。
>
> 优先级：`AGENTS.md`（最高） > 本文件 > `docs/PLAN.md`（计划与理由）。
> 若本文件与 `AGENTS.md` 冲突，以 `AGENTS.md` 为准并修正本文件。
>
> 逐任务提示词见 `AGENT-PROMPTS.md`，本地开发环境见 `DEV-ENV.md`。

---

## 1. 分层总览

```mermaid
graph TD
  subgraph Frontend["前端 src/（零信任输入，禁止直接碰文件系统）"]
    UI["src/features/*<br/>功能域界面"]
    SHARED["src/app、src/ui、src/lib、src/stores<br/>外壳/设计系统/IPC/i18n/状态"]
    UI --> SHARED
  end

  subgraph Host["桌面宿主 src-tauri"]
    TAURI["窗口、菜单、托盘、更新、命令注册<br/>（极薄：只做接线）"]
  end

  subgraph Rust["Rust workspace crates/"]
    CMD["commands<br/>IPC 边界：参数校验、DTO、能力等级"]
    SVC["services<br/>用例编排"]
    DOM["domain<br/>纯逻辑：状态机、错误模型、规则"]
    INFRA["infra crates<br/>git-engine / provider / snapshot / storage /<br/>diagnostics / credentials / jobs / platform / plugin-host"]
    CMD --> SVC
    SVC --> DOM
    SVC --> INFRA
    INFRA --> DOM
  end

  SHARED -->|"invoke（唯一入口 src/lib/ipc）"| TAURI
  TAURI --> CMD
```

**一句话**：前端只认 IPC 契约；`commands` 只做边界工作；`services` 编排用例；
`domain` 是纯逻辑且不碰 IO；所有与外界打交道的事（进程、数据库、网络、文件、钥匙串、时钟）
都落在 `crates/` 下的 infra crate。

---

## 2. crate 职责与状态

`谁能依赖它`一列是**硬约束**：违反会在评审与 `cargo` 依赖图里暴露，不在运行时才炸。

| crate | 职责 | 谁能依赖它 |
| --- | --- | --- |
| `crates/domain` | 纯逻辑、状态机、补丁、计划与 DAG 布局 | 全部 Rust crate |
| `crates/commands` | IPC 校验、DTO、能力等级与事件接线 | 宿主 |
| `crates/services` | 仓库、提交、同步、冲突、文件、终端、账号用例编排 | commands、plugin-host |
| `crates/git-engine` | GitEngine trait、libgit2 读取、CLI 写入与可取消进程 | services、snapshot、commands |
| `crates/provider` | 托管平台抽象、GitHub API、认证、缓存、限流与重试 | services、commands |
| `crates/snapshot` | 快照、内容备份、恢复、校验与保留策略 | services、commands |
| `crates/diagnostics` | 日志脱敏、stderr 解析、诊断规则与修复建议 | 边界与基础设施层 |
| `crates/storage` | SQLite、迁移、设置、账号元数据与操作记录 | services、commands、宿主 |
| `crates/credentials` | keyring、加密保险库原语、askpass、SSH 盘点 | services、commands、宿主 |
| `crates/jobs` | 任务注册、进度、完成事件与取消 | services、commands |
| `crates/platform` | 路径、日志、会话、文件监听、shell 与系统集成 | commands、宿主及基础设施 |
| `crates/plugin-host` | wasmi 沙箱、插件加载、权限、宿主 API 与资源限制 | commands、宿主 |
| `src-tauri` | 窗口、插件、服务装配、命令注册与启动退出接线 | 顶层 |
| `src/` | React 路由、功能界面、Query、UI 状态、IPC、设计系统与 i18n | 顶层 |

平台范围与验证状态见[维护清单](./MAINTENANCE.md)，命令参数以[API 契约](./API.md)
及 `pnpm check:ipc` 为准，避免在架构文档重复维护过期计数。

**与 `PLAN.md` §5.2 的差异（有意为之）**：PLAN 的模块表未列 `crates/platform`，
而 §5.9 又要求存在一个统一的平台适配层（凭据库、shell 解析、路径规范化、监听器、通知）。
本项目按 §5.9 落地，并以 `crates/platform` 承载：**PLAN 的模块表是早期草表，本文件是实际结构**。
新增 crate 必须同时更新本表与根 `Cargo.toml` 的 `members`。

---

## 3. 依赖方向规则

| 规则 | 说明 | 由什么保证 |
| --- | --- | --- |
| 前端不得直接访问文件系统或执行命令 | 一律经 `src/lib/ipc/index.ts` | ESLint 架构护栏（禁止 `@tauri-apps/api/*` 出现在该文件之外） |
| `domain` 不依赖任何 IO crate | 不许出现 `git2`/`rusqlite`/`reqwest`/`std::process` 等 | `crates/domain/tests/` 的依赖断言 + 根 `Cargo.toml` 的 crate 依赖清单 |
| 依赖只能"由外向内" | `commands → services → domain`，infra 只依赖 `domain` | 代码评审 + `Cargo.toml` 显式依赖（无 `*` 通配） |
| 唯一的 infra → infra 例外 | `git-engine → diagnostics`：git 的参数与输出都可能带凭据，写日志前必须过 `sanitize_log`（红线 R8）。若各自实现一套脱敏规则，迟早出现"一处漏了"的情况 | 评审；新增同类例外必须在此登记理由 |
| workspace 成员必须真实存在于仓库 | 防止"本地有、仓库没有"导致的 CI 失败 | `pnpm check:repo` |
| 能力等级必须声明 | 每个命令标注 `ReadOnly`/`Mutating`/`Network`/`Dangerous` | `docs/API.md` 登记表 + 评审 |
| 只有一条写入口 | 改仓库状态必须经 `SnapshotManager` + `AuditLog`（红线 R7） | 评审 + 破坏性操作测试矩阵（M3） |

---

## 4. 数据流示例

以下三条覆盖了本项目的主要形态：**写操作（有安全网）**、**长任务（有进度与取消）**、
**网络读取（有缓存与限流）**。三条链路均已在 M1–M4 落地。

### 4.1 提交（写操作：预览 → 快照 → 执行 → 可回滚）

```text
UI 填写提交信息
  → commit_prepare(plan)            生成 CommitPlan（含将要执行的等价 git 命令）
  → 预览对话框（用户确认）           SafeOperation：明确列出影响与回滚方式
  → commit_execute(planId)          计划可能已过期 → 重新校验 → PLAN_STALE
  → SnapshotManager.create("pre-commit")     （红线 R7 的安全网）
  → AuditLog.record(操作、参数摘要已脱敏、结果)
  → GitEngine.commit(...)            写操作走系统 git CLI（正确性优先）
  → 成功：事件通知 + 前端 Query 失效重取；失败：Diagnostics.parse(stderr) → AppError
```

**要点**：计划与执行分离（`prepare`/`execute`）让"用户看到的"和"实际执行的"是同一份数据；
快照在**执行之前**创建，因此任何失败都能回滚。

### 4.2 Pull 与冲突（长任务：进度事件 + 可取消 + 可中止）

```text
pull_execute(remote, branch, strategy)
  → 超过 500ms：注册为 Job（返回 jobId），进度经 job:progress 事件推送
  → SnapshotManager.create("pre-pull")
  → GitEngine.fetch()（网络，支持取消令牌）
  → 策略判定：ff-only / merge / rebase
  → 冲突：进入 ConflictWizard（三栏），逐文件 mark_resolved(path)
  → continue_operation()  或  abort_operation()  →  可选 rollback_to_snapshot()
```

**要点**：`git:state-changed` 事件让界面知道"仓库正处于 rebase 中途"——
这是刷新页面后仍要能恢复的**持久状态**，不能只存在前端内存里。

### 4.3 GitHub 拉取请求（网络读取：缓存 + 限流降级）

```text
pr_detail(owner, repo, number)
  → Provider.get_pull_request()
  → CacheStore：带 ETag，命中 304 → 直接用缓存（不消耗配额）
  → 未命中 → 请求 API → 写缓存
  → 403/429 → RateLimitError（含 reset 时间）→ UI 显示缓存 + 明确提示
```

**要点**：网络失败不能让用户看到空白页。缓存是**降级路径**的一部分，
因此"缓存中的数据 + 明确的过期提示"优于"什么都没有"。

缓存身份包含 URL、查询参数、认证及表示头的 SHA-256 摘要；不同账号不会共用私有响应。
缓存回放保留 `Link` 与 `ETag`：304 验证成功标记 `revalidated`，离线/限流回退标记 `stale`。
HTTP 克隆共享可替换的传输层，启动与代理设置变更使用同一配置装配路径；在途请求继续，
后续请求采用新代理，并清理旧缓存。Git CLI 的本地、网络和远端探测入口统一注入代理与绕过项。

### 4.4 前端会话、布局与仓库事件

- `RepoLayout` 为当前路由仓库维护统一 Query 失效订阅；子页面不再各自重复订阅。
  节流窗口累计全部路径与最高变化级别（`large > refs > workspace`），文件树也参与失效。
- `layoutStore` 是文件树宽度、详情位置与尺寸的唯一来源，负责预设、恢复与防抖持久化。
  容器用实际可用尺寸限制面板，历史图的高度沿 flex 链约束到视口；最小窗口的控制区单独滚动。
- `editorStore` 以 `(repoId, path)` 持有草稿、磁盘基线与活动标签，Monaco URI 同样包含仓库身份。
  草稿只在当前应用会话内保留，路由卸载不会丢失；关闭未保存文档需要确认。
- 终端 manager 按 PTY 会话持有最多 1 MiB 的输出回放缓冲。卸载只销毁视图，重新挂载回放，
  关闭标签才释放会话缓冲与输入跟踪器。截断时保留明确标记，不承诺完整终端历史持久化。
- Actions 日志以 owner/repo/job 标识独立视图会话；先订阅再启动，暂存 jobId 返回前的事件，
  关闭时取消后台任务并清理监听及待刷新定时器。同一任务的 props 对象重建不重启下载。
  日志内容进入有界环形缓冲（最多 20,000 行、4 Mi 个 UTF-16 code unit，单行最多 16,384），
  每 50ms 批量发布视图快照，终态立即发布；早到事件队列同样设上限，省略内容时明确提示。
- 工作区等常用入口同步装配；终端、编辑器、GitHub 子页面、插件面板与命令字典使用
  React Router 的路由级 lazy，直接进入时提供本地化加载提示。开发预览页面也按需加载，
  防止它们静态引用终端等模块而破坏生产拆包。

---

## 5. 扩展点

| 扩展点 | 位置 | 新增一类实现时要做什么 |
| --- | --- | --- |
| Git 引擎 | `crates/git-engine` | 实现 `GitEngine` trait；跑**双实现差分一致性测试**（含 merge/分叉/重命名/二进制/大文件/子模块的临时仓库）；新增公开 API 必须有 rustdoc |
| 托管平台 | `crates/provider` | 实现 `HostProvider`；UI **必须依据 capabilities 隐藏/禁用**，禁止按 provider 名字写 if；`ProviderRegistry` 需覆盖 ≥15 种 URL 变体的表驱动单测 |
| 诊断规则 | `crates/diagnostics/rules/*.yaml`（M5） | 规则只写 i18n key、不写文案；`kind=dangerous` 的动作必须走危险确认对话框，不允许一键执行 |
| 插件 | `crates/plugin-host`（M6） | 目录 + `plugin.json` + WASM/JS 入口；权限逐项授权；**插件 API 不提供 AI 推理能力，也不得访问系统 keyring**（红线 R1） |
| 前端页面 | `src/features/<域>/` | 新页面 = 路由条目（`src/app/routes.tsx`）+ 导航项（`src/app/shell/navItems.ts`）+ 只走 i18n 的文案；空页面用 `PlaceholderPage` 并标注归属任务号 |
| UI 组件 | `src/ui/components/` | 通用能力放这里（Radix 原语 + 语义 token），**不要**在功能目录里复制一份 |

---

## 6. 边界与"不在这里做"的事

- **不做 AI 推理**（红线 R1）：仓库里没有模型、没有推理服务；AI 只用于开发阶段写代码。
- **前端不做 Git**：任何 Git 语义（分支、状态判定、rebase 计划）都在 Rust 侧，前端只呈现。
- **DAG 泳道布局在 Rust 侧**（T2.1，与 PLAN §6.2.3"布局用 D3 计算"是有意差异）：
  布局是纯计算，落在 `crates/domain/history/layout`（可缓存、可属性测试、不卡 UI 线程），
  "折叠已合并分支"的判定在 Rust 侧完成，前端只负责隐藏呈现。
T3.6 的 rebase 拖拽面板最终**没有**引入 D3：拖拽用原生 HTML5 DnD（插入线落点指示），
预览用列表而非第二套 DAG——重写结果的确认需要的是"哪些被压/被丢/被改"的清单，
不是又一张图；执行后回历史页看真实 DAG。
- **不在 `commands` 里写业务**：命令层只做参数校验、DTO 与错误转换（见 `crates/commands/src/lib.rs` 的职责说明）。
- **不把 Git 状态放进 Zustand**：Git 状态是"服务端状态"，走 TanStack Query；
  Zustand 只放 UI 状态（侧栏、主题、面板位置、当前仓库 id）。
- **不引入遥测**（M7 前不做，且必须自建、默认关闭、可预览发送内容）。

---

## 7. 相关文档

- 任务顺序与验收标准：[`PLAN.md`](./PLAN.md)
- 逐任务提示词：[`AGENT-PROMPTS.md`](./AGENT-PROMPTS.md)
- IPC 契约（命令与事件登记表）：[`API.md`](./API.md)
- 编码风格：[`CODING_STYLE.md`](./CODING_STYLE.md)
- 本地环境与陷阱：[`DEV-ENV.md`](./DEV-ENV.md)
- 参与方式：[`../CONTRIBUTING.md`](../CONTRIBUTING.md)
- 最高优先级公约：[`../AGENTS.md`](../AGENTS.md)
