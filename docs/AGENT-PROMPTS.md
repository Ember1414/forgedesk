# ForgeDesk — 编码 Agent 提示词全集（全零成本方案）

> 版本：v1.0 ｜ 配套文档：`AGENTS.md`（代理公约）、`docs/PLAN.md`（完整计划书）
> 覆盖范围：**M0–M8 全部阶段、全部任务（T0.1–T8.8，共 95 条提示词）**
> 使用方式：从 §1 开始读，然后按 §7 的执行顺序逐条复制提示词给 AI 编码代理。
> **人类（你）只需要做两件事：审批 + 方向决策。** 每条提示词末尾都标注了「审批点」，未标注的任务代理可自行完成并继续。

---

## 目录

- [1. 使用说明：人类只做审批](#1-使用说明人类只做审批)
  - [1.1 你和代理的分工](#11-你和代理的分工)
  - [1.2 提示词怎么用](#12-提示词怎么用)
  - [1.3 零成本方案硬约束（每条提示词都生效）](#13-零成本方案硬约束每条提示词都生效)
- [2. 通用提示词模板](#2-通用提示词模板)
  - [2.1 GLOBAL-PRE（全局前缀，每次自动附加）](#21-global-pre全局前缀每次自动附加)
  - [2.2 GLOBAL-POST（全局后缀，每次自动附加）](#22-global-post全局后缀每次自动附加)
  - [2.3 审批请求格式](#23-审批请求格式)
- [3. 阶段启动提示词（KICK-OFF，KO-0 ~ KO-8）](#3-阶段启动提示词kick-offko-0--ko-8)
- [4. 逐任务提示词](#4-逐任务提示词)
  - [4.1 M0 地基（T0.1–T0.12）](#41-m0-地基t01t012)
  - [4.2 M1 Git 核心闭环（T1.1–T1.12）](#42-m1-git-核心闭环t11t112)
  - [4.3 M2 历史 DAG / 分支 / 远端同步（T2.1–T2.10）](#43-m2-历史-dag--分支--远端同步t21t210)
  - [4.4 M3 冲突 / Rebase 可视化 / 快照回滚（T3.1–T3.11）](#44-m3-冲突--rebase-可视化--快照回滚t31t311)
  - [4.5 M4 GitHub 集成（T4.1–T4.12）](#45-m4-github-集成t41t412)
  - [4.6 M5 终端 / 诊断 / 编辑器（T5.1–T5.10）](#46-m5-终端--诊断--编辑器t51t510)
  - [4.7 M6 插件 / 主题 / 多平台适配（T6.1–T6.10）](#47-m6-插件--主题--多平台适配t61t610)
  - [4.8 M7 自动更新 / CI / 打包 / 文档 / 发布（T7.1–T7.10）](#48-m7-自动更新--ci--打包--文档--发布t71t710)
  - [4.9 M8 零成本发布加固 / 分发 / 社区（T8.1–T8.8）](#49-m8-零成本发布加固--分发--社区t81t88)
- [5. 通用运维提示词（OPS）](#5-通用运维提示词ops)
- [6. 人类审批点总表](#6-人类审批点总表)
- [7. 一键执行顺序（拷贝顺序）](#7-一键执行顺序拷贝顺序)

---

## 1. 使用说明：人类只做审批

### 1.1 你和代理的分工

| 角色 | 职责 | 投入 |
| --- | --- | --- |
| **你（人类）** | ① 确认待决策项 ② 批准范围变更 ③ 批准需要账号/第三方提交/花钱的动作 ④ 批准发布 ⑤ 最终体验验收 | 每里程碑 1–2 次、每次 10–30 分钟 |
| **AI 编码代理** | 实现、测试、文档、打包、开 PR、自检、修复 CI | 全部执行工作 |

**你不需要做的事**：写代码、读代码细节、调试、写文档、配置 CI、打包、写测试。

**你唯一必须亲自做的事**：
1. 在 M0 开始时确认 §15.6 的 10 项待决策（`docs/PLAN.md`）。
2. 创建 GitHub 公共仓库并设置分支保护（代理无法替你做账号层面的操作）。
3. 在需要向第三方仓库提交 PR（Winget/Flathub/AUR/Homebrew tap）时点击提交。
4. 发布前做一次真实体验验收。
5. 遇到代理请求「审批」时给出决定。

### 1.2 提示词怎么用

每条任务的完整提示词 = **GLOBAL-PRE + 任务提示词正文 + GLOBAL-POST**。

```text
┌─────────────────────────────────────────────┐
│ GLOBAL-PRE（§2.1，固定 3 行，每次复制）        │
├─────────────────────────────────────────────┤
│ 任务提示词正文（§4 中对应任务，如 T0.1）        │
├─────────────────────────────────────────────┤
│ GLOBAL-POST（§2.2，固定，每次复制）            │
└─────────────────────────────────────────────┘
```

**更省事的做法**：直接说「读 `AGENTS.md`，执行 `docs/AGENT-PROMPTS.md` 中的 T0.1」，并把任务提示词正文粘进去。因为 `AGENTS.md` 已被代理自动读取，GLOBAL-PRE/POST 的大部分内容会被覆盖。

**推荐工作流（每个任务）**：

```text
1. 复制 GLOBAL-PRE + 任务提示词 + GLOBAL-POST → 发给代理
2. 代理执行 → 返回 AGENTS.md §5 格式的回报
3. 你只需看三处：① 状态 ② 验证证据 ③ 是否有「需要人类审批」
4. 若代理请求审批 → 按 §2.3 格式回复决定
5. 若无审批项 → 直接进入下一个任务的提示词
6. 每个里程碑结束时 → 执行 KO 提示词 + OPS-6（里程碑收口验收）
```

### 1.3 零成本方案硬约束（每条提示词都生效）

| # | 约束 |
| --- | --- |
| F1 | **不花钱**：不使用任何付费服务、付费账号、付费证书、付费商店 |
| F2 | macOS：**ad-hoc 签名** + 去隔离指引，**不做**付费公证 |
| F3 | Windows：**SHA256 + GPG + 便携版 + Scoop/Winget**，**不做**付费代码签名 |
| F4 | 分发只用 **GitHub Releases / Pages / Homebrew / Scoop / Winget / Flathub / Snap / AUR / Itch.io** |
| F5 | CI 只用 **GitHub Actions（公共仓库免费额度）** |
| F6 | 域名只用 **GitHub Pages / Cloudflare Pages 免费域名** |
| F7 | 更新用 **tauri-plugin-updater + 自生成 Ed25519 密钥 + Pages 托管清单** |
| F8 | 产品**不得包含任何 AI 功能**；不得引入任何推理依赖或第三方分析 SDK |

---

## 2. 通用提示词模板

### 2.1 GLOBAL-PRE（全局前缀，每次自动附加）

```text
你在 ForgeDesk 仓库中作为编码代理工作。
先读 AGENTS.md（必须）与 docs/PLAN.md 中本任务对应的章节，再开始。
严格遵守 AGENTS.md 的全部红线与本次附带的零成本约束；需求含糊时做最小合理假设并标注，不要停下来等待。
```

### 2.2 GLOBAL-POST（全局后缀，每次自动附加）

```text
【本次任务的完成标准】
1. 按 AGENTS.md §4 运行全部质量门禁，必须全绿：
   pnpm lint && pnpm typecheck && pnpm test
   cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --workspace --all-features
2. 新增的 Tauri Command 登记到 docs/API.md；新增用户可见文案走 i18n（中英同步）。
3. 新增领域逻辑必须有单测；新增核心组件必须有 Vitest 测试；E2E 断言 window.__errs 为空。
4. 按 AGENTS.md §5 的格式输出回报，必须包含：状态 / 变更文件 / 验证证据 / 假设 / 需要人类审批 / 风险与后续。
5. 若发现本任务与 docs/PLAN.md 冲突，以 PLAN 为准并在回报中说明。
6. 不要顺手实现本任务范围之外的功能；发现需要新增功能时，只提出建议，不实现。
```

### 2.3 审批请求格式

代理向你请求审批时，统一使用此格式（你只需回复 `批准 A` / `批准 B` / `拒绝，改为…`）：

```markdown
## 需要人类审批
**事项**：<一句话说明需要决定什么>
**背景**：<为什么必须由人类决定，代理无法自行处理的原因>
**选项**：
- A：<方案 A> ｜ 影响：<影响> ｜ 建议：<是否推荐>
- B：<方案 B> ｜ 影响：<影响>
**若不决定会怎样**：<阻塞点>
```

---

## 3. 阶段启动提示词（KICK-OFF，KO-0 ~ KO-8）

> 每个里程碑开始时执行一次。作用：让代理先把阶段拆成 Issue、建立分支、给出执行计划，**由你一次性审批计划**，之后按 §4 逐任务执行即可，无需反复介入。

### KO-0 — M0 启动

```text
【KO-0：启动 M0 里程碑】
1. 通读 AGENTS.md 与 docs/PLAN.md 的 §0、§5、§6、§7.0、M0 章节。
2. 输出一份 M0 执行提案，包含：
   - 12 个任务（T0.1–T0.12）的执行顺序、依赖关系与并行可能性；
   - 每个任务对应的 GitHub Issue 标题与验收标准（可直接创建）；
   - 需要人类先完成的前置事项清单（如创建公共仓库、确认 §15.6 的 10 项待决策、开启分支保护）；
   - 本阶段将建立的文件与目录结构预览。
3. 列出你需要我审批的事项（如有），不要开始编码。
4. 我批准后，你会收到「KO-0 已批准，开始执行 T0.1」的指令。
```

### KO-1 — M1 启动

```text
【KO-1：启动 M1 里程碑】
1. 通读 docs/PLAN.md 的 M1 章节与 §5.2/§5.6/§5.10/§5.12。
2. 输出 M1 执行提案：
   - 12 个任务（T1.1–T1.12）的顺序与依赖（说明为何必须先做 GitProcess 再做 GitEngine）；
   - 需要先固化的接口契约（GitEngine trait 方法签名、AppError 错误码清单、SQLite 表结构）；
   - 端到端验收场景清单（对应 PLAN M1 验收标准，逐条给出可自动化的验证方式）；
   - 风险最高的 2 个任务与你的降级方案。
3. 不要开始编码，等我批准。
```

### KO-2 — M2 启动

```text
【KO-2：启动 M2 里程碑】
1. 通读 docs/PLAN.md 的 M2 章节与 §5.3/§6.2.3。
2. 输出 M2 执行提案：
   - 10 个任务（T2.1–T2.10）的顺序；说明 DAG 布局放在 Rust 侧还是前端侧的最终决策与理由；
   - DAG 正确性的自动化验证方案（如何与 git log --graph 语义比对）；
   - 性能基准方案（如何在 CI 中构造 5 万提交的测试仓库）；
   - 远端同步的五类场景测试矩阵。
3. 不要开始编码，等我批准。
```

### KO-3 — M3 启动

```text
【KO-3：启动 M3 里程碑（差异化核心，风险最高）】
1. 通读 docs/PLAN.md 的 M3 章节与 §5.4/§5.10/§10.6。
2. 输出 M3 执行提案：
   - 11 个任务（T3.1–T3.11）的顺序；明确三栏编辑器"块级优先、字符级增强"的两阶段交付边界；
   - 冲突解析的数据结构设计（不依赖工作区标记符，直接读 index stage）；
   - RebasePlan 纯逻辑的属性测试设计（proptest 断言什么）；
   - 快照 v2 的备份策略与磁盘占用控制参数（默认阈值、超出后的降级行为）；
   - 破坏性操作安全测试矩阵（15 场景）的自动化实现方式。
3. 明确指出你认为最容易失败的一个环节，以及你打算如何提前验证它（spike）。
4. 不要开始编码，等我批准。
```

### KO-4 — M4 启动

```text
【KO-4：启动 M4 里程碑】
1. 通读 docs/PLAN.md 的 M4 章节与 §5.7。
2. 输出 M4 执行提案：
   - 12 个任务（T4.1–T4.12）的顺序；
   - HostProvider trait 与 ProviderCapabilities 的完整定义（含 GitLab/Gitea 的预埋）；
   - GitHub Device Flow 的完整交互设计（含轮询、slow_down、过期、拒绝四类分支）；
   - 契约测试方案（wiremock 覆盖哪些端点、如何避免真实网络）；
   - 限流与离线降级策略。
3. 说明哪些部分需要我提供一个真实的 GitHub 账号做端到端验证（并给出替代方案）。
4. 不要开始编码，等我批准。
```

### KO-5 — M5 启动

```text
【KO-5：启动 M5 里程碑】
1. 通读 docs/PLAN.md 的 M5 章节与 §5.9。
2. 输出 M5 执行提案：
   - 10 个任务（T5.1–T5.10）的顺序；
   - 三平台 PTY spike 方案（先在 Windows ConPTY 与 Linux 上验证 portable-pty，再全面开发）；
   - 诊断规则文件格式与规则来源（列出你计划覆盖的 50 条错误的分类）；
   - 终端中破坏性命令的拦截提示设计（与安全层的衔接方式）。
3. 不要开始编码，等我批准。
```

### KO-6 — M6 启动

```text
【KO-6：启动 M6 里程碑】
1. 通读 docs/PLAN.md 的 M6 章节与 §5.8。
2. 输出 M6 执行提案：
   - 10 个任务（T6.1–T6.10）的顺序；
   - 插件清单 schema 与宿主 API 的完整定义（含权限到 API 的映射表）；
   - wasmtime 的体积与编译时间评估（是否需要 feature 开关）；
   - 三个示例插件的具体功能定义（必须能证明 API 可用）；
   - 主题 JSON 格式与 i18n key 命名规范。
3. 不要开始编码，等我批准。
```

### KO-7 — M7 启动

```text
【KO-7：启动 M7 里程碑（发布工程）】
1. 通读 docs/PLAN.md 的 M7 章节与 §8.4/§8.5/§11、§12.3（零成本方案）。
2. 输出 M7 执行提案：
   - 10 个任务（T7.1–T7.10）的顺序；
   - **macOS 自动更新 spike 方案**：在无公证（仅 ad-hoc 签名）的情况下验证 tauri-plugin-updater
     是否可用；若不可用，给出"检查更新 → 打开下载页"的降级设计（这是本阶段最大的不确定性）；
   - GPG 密钥生成、存储（CI Secret）、离线备份与轮换流程；
   - 打包矩阵与产物命名规范；更新清单的目录结构与托管方式（GitHub Pages 免费域名）；
   - 文档五件套的目录结构与各自条目清单。
3. 不要开始编码，等我批准。
```

### KO-8 — M8 启动

```text
【KO-8：启动 M8 里程碑（零成本发布加固，禁止任何付费）】
1. 通读 docs/PLAN.md 的 M8 章节、§8.5（零成本信任方案）、§8.6（免费渠道）、§12.3。
2. 输出 M8 执行提案：
   - 8 个任务（T8.1–T8.8）的顺序；
   - macOS ad-hoc 签名验证方法（如何在 CI 中断言签名存在）；
   - Windows 信任加固的完整用户路径（下载 → 校验 → 安装 → 首次启动），逐步说明用户会看到什么；
   - 6 个包管理器渠道的清单文件结构与各自的提交/审核要点；
   - 你需要我手工执行的不可自动化步骤清单（如向 winget-pkgs、flathub、AUR 提交 PR）。
3. 明确重申：本阶段不允许产生任何费用；如某项必须付费才能完成，直接标记为「不做」并给出免费替代。
4. 不要开始编码，等我批准。
```

---

## 4. 逐任务提示词

> 格式说明：每条提示词上方标注 `依赖` / `估时` / `审批点`。使用时按 §1.2 拼接 GLOBAL-PRE 与 GLOBAL-POST。
> `审批点：无` 表示代理可自行完成并继续，无需你介入。

### 4.1 M0 地基（T0.1–T0.12）

#### T0.1 初始化 Tauri 2 + React + Vite 工程

`依赖：—` ｜ `估时：1d` ｜ `审批点：需你提供 GitHub 公共仓库地址与组织名`

```text
【T0.1 初始化工程脚手架】
只做工程初始化，不实现任何业务功能。
1) 在仓库根初始化 Tauri 2 项目：前端 React + TypeScript + Vite + Tailwind CSS，
   组件库采用 shadcn/ui（配置 components.json，输出目录 src/ui/components）。
2) 建立 pnpm 脚本：dev / build / preview / lint / typecheck / test / tauri / i18n:lint / i18n:extract。
3) 配置 TypeScript 严格模式（strict、noUncheckedIndexedAccess、exactOptionalPropertyTypes）
   与 ESLint（含 react-hooks、import 排序、no-restricted-imports 禁止前端直接 import @tauri-apps/api 之外的底层模块）。
4) 配置 Prettier + .editorconfig + .gitignore（排除 node_modules、dist、target、**/gen、真实凭据文件）。
5) 在 tauri.conf.json 设置：productName "ForgeDesk"、identifier "org.forgedesk.app"（暂定，若与 §15.6 决策不符以决策为准）、
   window 尺寸 1280x800、最小 960x600、标题 "ForgeDesk"。
6) package.json 中 name 为 forgedesk，version 0.0.1。
硬约束：不得引入任何 AI/LLM 依赖，不得引入任何分析/遥测 SDK。
完成后运行 pnpm lint && pnpm typecheck && pnpm tauri build --debug（或 cargo check）确保通过。
回报中输出：创建的文件树、安装的依赖清单、与计划书的偏差。
```

#### T0.2 Rust workspace 与 crate 骨架

`依赖：T0.1` ｜ `估时：1d` ｜ `审批点：无`

```text
【T0.2 建立 Rust workspace 与模块骨架】
1) 在仓库根创建 Cargo workspace（src-tauri 作为成员之一），成员如下，
   每个 crate 只含 lib.rs + rustdoc 说明 + 一个空的 pub struct，保证 cargo build 通过：
   crates/domain, crates/services, crates/commands, crates/git-engine, crates/provider,
   crates/storage, crates/snapshot, crates/diagnostics, crates/credentials, crates/jobs,
   crates/platform, crates/plugin-host（最后两个可在 M6 前仅留空壳）。
2) 统一依赖版本于 [workspace.dependencies]：thiserror、anyhow、serde、serde_json、
   tracing、tracing-subscriber、tokio、tokio-util（CancellationToken）、uuid、time/chrono、
   rusqlite（bundled feature）、git2、secrecy、parking_lot。
3) 配置 rustfmt.toml（edition 2021、max_width 100）与 clippy lint 级别（在 workspace lints 中开启
   unwrap_used/expect_used 为 warn、correctness/suspicious 为 deny）。
4) 在 crates/commands 中实现最小 Tauri command：app_version() -> AppVersion { version, git_sha, target }，
   并在前端调用展示（用于验证 IPC 通路）。
5) 在 crates/domain 建立 AppError、ErrorCode 枚举（先放 GIT_CONFLICT / AUTH_EXPIRED / PATH_NOT_REPO /
   VALIDATION / NETWORK / RATE_LIMITED / PTY_UNSUPPORTED / UNKNOWN 等占位），并实现 Serialize。
硬约束：domain crate 不得依赖任何 IO crate（git2/rusqlite/reqwest 均不可）。
完成后 cargo build --workspace 与 pnpm tauri dev 均须成功；回报 dependency 图与 lint 配置说明。
```

#### T0.3 设计系统 token 与原创应用图标

`依赖：T0.1` ｜ `估时：1.5d` ｜ `审批点：**需要你确认图标方案**（红线 R2）`

```text
【T0.3 设计系统 token 与原创图标】
⚠️ 红线：应用图标必须 100% 原创，禁止使用 Git / GitHub / Tauri 官方 Logo、禁止 Octocat 及其任何变体、
   禁止与上述图形产生视觉联想（猫、触手、分支线构成的动物等一律禁止）。
1) 在 src/ui/tokens.css 用 CSS 变量定义设计 token：色彩（brand / spark / canvas / surface / border /
   text-primary / text-secondary / text-muted / success / warning / danger）、字号阶（12/13/14/16/20/24/32）、
   间距（4/8/12/16/24/32）、圆角（4/6/8/12/16）、阴影（3 级）、动效时长（120/200/320ms）、
   字体族（系统字体栈 + 可选 Inter，若使用 Inter 必须附带 OFL 许可证文件）。
2) 提供亮/暗两套主题，通过 <html data-theme="light|dark"> 切换；另提供 "system" 模式（跟随系统）。
   颜色必须同时满足 WCAG AA 对比度（正文 ≥ 4.5:1，大字 ≥ 3:1），请在回报中给出你的对比度检查结果。
3) 设计并生成原创应用图标：主题为**抽象锻炉/砧台几何造型**（可用折线、弧面、火花的抽象化处理），
   靛蓝→青紫渐变；禁止出现分支、节点连线、圆点构成的图、任何动物形象。
   生成 32/64/128/256/512/1024 的 PNG，并转换出 .ico 与 .icns 放入 src-tauri/icons/。
   同时保存矢量源文件到 docs/brand/icon-source.svg。
4) 写 docs/BRAND.md：设计概念说明、色彩体系、最小尺寸与留白规则，
   并显式声明"本图标完全原创，未使用 Git / GitHub / Tauri 官方 Logo 或其变体"。
5) 提供 src/ui/__dev__/DesignSystemPage.tsx：本地预览全部 token（色板、字号、间距、阴影）
   与主题切换开关。
验收：pnpm dev 可访问设计系统预览页；亮暗切换无闪烁；图标文件齐全且 tauri.conf.json 已引用。
⚠️ 完成后必须请求人类审批图标方案后再进入 T0.4。
```

#### T0.4 应用外壳布局与路由

`依赖：T0.3` ｜ `估时：2d` ｜ `审批点：**需要你确认主界面布局**（红线 R3）`

```text
【T0.4 应用外壳布局与路由】
⚠️ 红线：不得复刻 GitHub Desktop / GitKraken / Sourcetree / Fork / Git-cola 的界面布局。
   请自行设计一套布局，并在回报中用文字描述你的布局决策与视角来源（不必参考竞品）。
1) 实现应用外壳：顶部标题栏（仓库切换器 + 全局搜索入口 + 账号头像位 + 更新提示位）、
   左侧主导航（图标 + 文字的垂直导航，可折叠）、主内容区、底部状态栏（当前仓库 / 分支 / 操作状态 / 后台任务）。
2) 路由（react-router 或 TanStack Router，二选一并说明理由）：
   /（仪表盘 Dashboard）、/repo/:id/status（工作区）、/repo/:id/history（历史）、
   /repo/:id/branches、/repo/:id/conflict、/repo/:id/terminal、
   /github（仓库/PR/Issue/Actions 子路由）、/settings/*、/plugins。
   本任务只做空页面骨架与导航，不实现功能。
3) 实现 Zustand store：uiStore（侧栏折叠、主题、当前仓库 id、面板布局）与 jobStore（后台任务列表占位）。
4) 支持键盘导航：Tab 顺序合理、导航项可被 Enter 触发、Esc 关闭浮层；所有交互元素有可见焦点环。
5) 布局为响应式：窗口宽度 < 1100px 时侧栏自动折叠为图标模式。
验收：所有路由可跳转；pnpm lint/typecheck 通过；在回报中提供布局文字线框图（ASCII 或列表）。
⚠️ 完成后必须请求人类审批主界面布局后再进入 T0.5。
```

#### T0.5 基础组件库与主题切换

`依赖：T0.4` ｜ `估时：2d` ｜ `审批点：无`

```text
【T0.5 基础组件库】
在 src/ui/components 基于 Radix UI 原语 + Tailwind 实现以下组件，全部支持键盘操作与暗色主题：
Button（primary/secondary/ghost/danger/link，size: sm/md/lg，loading 态）、IconButton（必有 aria-label）、
Input、Textarea、Select、Checkbox、RadioGroup、Switch、Slider、Dialog、AlertDialog、Sheet（抽屉）、
Tabs、Tooltip、Popover、DropdownMenu、ContextMenu、Toast（含 toast queue）、
Table（排序/空态/加载骨架）、Badge、Tag、Progress、Skeleton、EmptyState、ErrorState、
SplitPane（可拖拽分栏）、Resizable、VirtualList（用于长列表）。
要求：
1) 所有组件用 forwardRef，样式通过 className + tailwind-merge 合并。
2) 危险操作按钮（danger）在 AlertDialog 中必须显示"影响说明"插槽。
3) 为每个组件写一个精简的 Vitest 测试：渲染 + 键盘交互 + 禁用态（覆盖 aria 属性）。
4) 禁止在组件内写死十六进制颜色值或固定 px 字号，一律引用 tokens.css 的 CSS 变量。
5) 组件展示页（可复用 T0.3 的 DesignSystemPage）覆盖全部组件与全部状态（默认/hover/disabled/loading/error）。
验收：pnpm test 全绿；在回报中列出组件清单与各自的测试覆盖点。
```

#### T0.6 统一错误模型、i18n 骨架与错误展示

`依赖：T0.2, T0.5` ｜ `估时：1d` ｜ `审批点：无`

```text
【T0.6 AppError + i18n + Toaster】
1) 后端：crates/domain 中完善 AppError（见 docs/PLAN.md §5.5 的结构：code/message/detail/hint/actions/retryable），
   为每种 ErrorCode 提供默认 message 与 i18n key 映射；detail 必须经过脱敏器
   （过滤 Authorization、token=、ghp_/gho_/github_pat_/glpat- 等模式）。
2) 后端：crates/commands 提供统一错误转换层，把 anyhow/thiserror 错误映射为 AppError；
   提供 sanitize_log(text) -> String 并在 tracing 的格式化层中应用。
3) 前端：src/lib/errors.ts 提供 useAppError / normalizeError(e: unknown): AppError，
   兼容 IPC 返回的非结构化错误；提供 ErrorState 与 ErrorToast 组件：
   Toast 展示 title（i18n）+ hint，可展开查看 detail，可渲染 actions 按钮（点击后调用对应 Tauri command）。
4) i18n：接入 i18next + react-i18next，命名空间按功能域拆分（common/repo/history/conflict/github/terminal/editor/settings/plugin），
   建立 locales/zh-CN 与 locales/en-US 目录；本任务只需 common 命名空间。
5) 实现 `pnpm i18n:lint` 脚本：扫描 src 下 .ts/.tsx 中出现的用户可见中文字符串，
   未被 t(...) 包裹且不在白名单注释 // i18n-ignore 的行即报错退出。
6) 提供一个演示用的失败 command（如 debug_throw_error(code)）用于端到端验证错误链路。
验收：在 UI 中触发演示错误，Toast 正确显示 code/hint，展开可见 detail，动作按钮可点击；
   pnpm i18n:lint 通过；sanitize 单测覆盖 8 种敏感模式。
```

#### T0.7 SQLite 接入、迁移框架与设置持久化

`依赖：T0.2` ｜ `估时：1.5d` ｜ `审批点：无`

```text
【T0.7 存储层与设置】
1) crates/storage：使用 rusqlite（bundled）建立连接池（r2d2 或自研单写多读），
   数据库位于 Tauri 的应用数据目录（app_data_dir()/forgedesk.db）；启用 WAL、foreign_keys、busy_timeout。
2) 迁移框架：migrations/ 下版本化 SQL（0001_init.sql 起），启动时按版本顺序执行并记录 schema_version；
   迁移前自动备份 db 文件（保留最近 3 份），迁移失败则回滚并给出明确 AppError。
3) 实现 docs/PLAN.md §5.10 中全部表结构（repositories / settings / snapshots / operation_records /
   accounts / api_cache / audit_log）与索引；本任务只建表与提供 Repository 层 CRUD，
   不做业务逻辑。
4) 实现设置服务：settings_get(scope, repo_id, key) / settings_set(...) / settings_all(scope)，
   支持 global 与 repo 两种 scope，值为 JSON 字符串。
5) 前端：settingsStore 读取设置，设置页展示一个演示项（如 theme）并持久化。
验收：重启应用后设置保持；迁移在已有旧库上可正确升级（写单测：构造 v0 库 → 迁移到当前 → 断言表存在）；
   并发写入不报 SQLITE_BUSY。
```

#### T0.8 结构化日志与日志文件轮转

`依赖：T0.2, T0.7` ｜ `估时：1d` ｜ `审批点：无`

```text
【T0.8 日志系统】
1) 使用 tracing + tracing-subscriber，输出到：① 控制台（dev）② 文件（release），
   文件位于 app_log_dir()/forgedesk.log，按天轮转 + 单文件上限 10MB，保留 7 天（tracing-appender）。
2) 统一日志格式：时间戳、级别、target、message、结构化字段；所有日志经 sanitize_log 脱敏。
3) 提供命令：logs_open()（在系统文件管理器中打开日志目录）、logs_tail(lines) -> Vec<LogLine>（读取末尾 N 行，
   供"反馈"功能预填使用，返回前必须脱敏）。
4) 前端：设置页提供"查看日志目录"按钮；错误 Toast 展开后可"查看相关日志"（调用 logs_tail 并高亮当前错误时间附近的行）。
5) 建立 panic hook：把 panic 信息写入 logs/panic-<timestamp>.log，并在下次启动时检测到未正常退出的标记
   （为 M7 的崩溃恢复预留）。
验收：单测覆盖 sanitize；手工触发一次 panic，确认生成 panic 日志且主流程不崩溃（仅该操作失败）。
```

#### T0.9 规范文档五件套与 AGENTS.md 落地

`依赖：T0.1–T0.8` ｜ `估时：1.5d` ｜ `审批点：**需要你确认 AGENTS.md 与合规声明文案**`

```text
【T0.9 工程规范文档】
基于 docs/PLAN.md 生成以下文档（内容要具体、可执行，禁止空话）：
1) docs/ARCHITECTURE.md：分层图（用 mermaid）、crate 职责表、依赖方向规则（哪层可以依赖哪层）、
   数据流示例 3 条、扩展点说明。
2) docs/API.md：Tauri Command 登记表（Command / 参数 / 返回 / 错误码 / 所需能力等级 ReadOnly|Mutating|Network|Dangerous）
   与事件登记表；提供"新增 Command 的检查清单"。
3) docs/CODING_STYLE.md：Rust（错误处理、命名、禁止 unwrap、日志规范、测试命名）与
   TypeScript/React（组件结构、hooks 规范、状态归属规则、i18n 规则、禁止硬编码颜色）约定。
4) CONTRIBUTING.md：开发环境搭建、常用命令、提交规范（Conventional Commits）、PR 流程、
   i18n 贡献方式、诊断规则贡献方式、文档贡献方式。
5) AGENTS.md 已存在于仓库根，请校对并与上述文档保持一致（如有冲突，以 AGENTS.md 为准并修正其他文档）。
6) README.md：项目简介、一句话价值、原创截图占位（本阶段用设计系统页截图）、
   安装说明（指向 M7/M8 的文档，本阶段写"开发中"）、许可证、**必须包含免责声明**（见 docs/PLAN.md §9.5 模板全文）。
7) 在 .github/ 下创建：pull_request_template.md（含"未复制竞品 UI/未使用受限商标/未引入 AI 依赖"勾选项）、
   ISSUE_TEMPLATE/bug_report.yml、feature_request.yml、rfc.yml、question.yml。
验收：所有文档中的链接有效（可用 markdown-link-check 或自写脚本校验）；README 含完整免责声明。
⚠️ 完成后请求人类审批 AGENTS.md 与 README 免责声明文案。
```

#### T0.10 CI 三平台矩阵流水线

`依赖：T0.1–T0.9` ｜ `估时：2d` ｜ `审批点：需你在 GitHub 开启分支保护`

```text
【T0.10 CI 流水线】
创建 .github/workflows/ci.yml：
1) 触发：pull_request、push to main；使用 concurrency 取消同分支旧运行。
2) job quality（ubuntu-latest）：
   - actions/checkout（固定 commit SHA）、setup-node（缓存 pnpm）、pnpm/action-setup、
     dtolnay/rust-toolchain（stable + rustfmt + clippy）、Swatinem/rust-cache。
   - 步骤：pnpm install --frozen-lockfile → pnpm lint → pnpm typecheck → pnpm test -- --coverage
     → pnpm i18n:lint → cargo fmt --check → cargo clippy -- -D warnings → cargo test --workspace。
   - 覆盖率低于阈值（Rust 60%）时失败（使用 cargo-llvm-cov 或 tarpaulin）。
   - cargo deny check（配置 deny.toml：licenses 允许 MIT/Apache-2.0/BSD/ISC/OFL/0BSD/Unicode-3.0 等，
     显式拒绝 GPL/AGPL，libgit2 的链接例外需写明 allow）。
3) job build（matrix: windows-latest / macos-14 / ubuntu-22.04）：
   - 安装 Linux 依赖（libwebkit2gtk-4.1-dev、libgtk-3-dev、libayatana-appindicator3-dev、
     librsvg2-dev、libssl-dev、libsecret-1-dev）。
   - pnpm tauri build --debug（PR 阶段用 debug 加速）→ 上传 artifact。
4) 所有第三方 action 固定到 commit SHA；secrets 在非 release job 中不注入。
5) 写 docs/CI.md 说明每个 job 的作用与本地复现方式。
验收：在 GitHub 上推动一个包含故意 lint 错误的提交，确认 CI 失败；修复后全绿。
⚠️ 请求人类在 GitHub 上设置 main 分支保护（要求 quality + build 通过）。
```

#### T0.11 打包配置与产物命名

`依赖：T0.10` ｜ `估时：1.5d` ｜ `审批点：无`

```text
【T0.11 多平台打包配置】
1) tauri.conf.json 的 bundle 配置：
   - targets: ["msi", "nsis", "dmg", "app", "appimage", "deb", "rpm"]；
   - category: "DeveloperTool"；shortDescription / longDescription 使用
     "ForgeDesk — A Git client" 与中文描述（均不得出现 "Git" 作为产品名一部分）；
   - icons 指向 T0.3 的产物；Windows 下配置 wix 语言与 nsis 安装模式（perMachine 与 perUser 二选一，给出理由）；
   - 文件关联：不注册 .git 相关关联；注册自定义协议 forgedesk:// （为 M8 预留，本任务只配置不实现处理）。
2) 产物命名规范：ForgeDesk_<version>_<target>_<arch>.<ext>，在 CI 中通过脚本重命名，
   保证 Release 附件名稳定可被 package manager 清单引用。
3) 在 CI build job 中生成 SHA256SUMS（三平台产物汇总）并上传为 artifact（本任务只生成，GPG 签名在 M8）。
4) 验证 AppImage 在 ubuntu-22.04 上可执行、deb 可安装（CI 中执行 --help 冒烟）。
验收：三平台 CI 均产出可下载的安装包；在本地（Windows）实测安装、启动、卸载流程正常。
```

#### T0.12 合规检查脚本与 CI 合规作业

`依赖：T0.11` ｜ `估时：1d` ｜ `审批点：无`

```text
【T0.12 合规自动化检查】
创建 .github/workflows/compliance.yml 与 scripts/compliance/ 下的检查脚本，覆盖 docs/PLAN.md §9.5 的红线：
1) 名称检查：断言 package.json name、tauri.conf.json productName/identifier、README 标题、
   窗口标题中不含 "git"/"github"（大小写不敏感，允许出现在 "A Git client" 这类描述语中，需按上下文白名单）。
2) 免责声明检查：断言 README.md 与 docs/PRIVACY.md（若存在）包含关键词
   "not affiliated"、"Software Freedom Conservancy"、"GitHub, Inc."、"Tauri"，缺失即失败。
3) 图标检查：计算 src-tauri/icons/ 下所有文件的 SHA256，与 scripts/compliance/known-logos.json
   （内含 Git/GitHub/Tauri/Octocat 已知 Logo 哈希，若无法取得哈希则用尺寸+文件名黑名单替代）比对，命中即失败；
   同时断言 docs/brand/icon-source.svg 存在。
4) 依赖许可检查：cargo deny check licenses 与 pnpm licenses list 生成 docs/LICENSE-AUDIT.md，
   若出现 GPL/AGPL（除 libgit2 例外）则失败并提示替换建议。
5) AI 依赖检查：扫描 Cargo.toml / package.json 的依赖名与源码中的
   openai|anthropic|ollama|llama|transformers|onnxruntime|tensorflow|torch 等关键词，命中即失败。
6) 竞品视觉检查（尽力而为）：在 .github/COMPETITOR-REVIEW.md 中提供人工评审清单与流程说明
   （此部分无法自动判定，输出提醒）。
验收：脚本在本地可运行（pnpm compliance）；故意在 README 删除免责声明后 CI 失败。
```

### 4.2 M1 Git 核心闭环（T1.1–T1.12）

#### T1.1 GitProcess 安全执行器与输出解析器

`依赖：T0.2` ｜ `估时：3d` ｜ `审批点：无`

```text
【T1.1 安全执行 git 进程与解析器】
在 crates/git-engine 实现：
1) GitProcess：基于 tokio::process::Command，接口：
   run(args: &[String], opts: GitRunOpts) -> Result<GitOutput, AppError>
   GitRunOpts { cwd: PathBuf, env: Vec<(String,String)>, timeout: Duration, cancel: Option<CancellationToken>,
                stdin: Option<Vec<u8>>, on_stderr_line: Option<Box<dyn Fn(&str) + Send + Sync>> }
   要求：
   - 参数一律以数组传递，禁止 shell 拼接（代码中不得出现 sh -c / cmd /C 包裹）；
   - 固定环境：LC_ALL=C、LANG=C、GIT_TERMINAL_PROMPT=0、GIT_ASKPASS=、GIT_PAGER=cat、GIT_OPTIONAL_LOCKS=0（读操作）；
   - 支持超时与取消（取消时 kill 子进程并回收）；
   - stdout/stderr 分别捕获并保留原始字节（含编码容错：非 UTF-8 用 lossy 转换并标记）；
   - 执行前后写 tracing 日志，命令参数与输出经 sanitize_log 脱敏；
   - 记录耗时，超过 500ms 记 info 级慢日志。
2) 解析器（每个都要有独立模块与单测）：
   - parse_status_porcelain_v2(&[u8]) -> StatusReport（支持 -z 的 NUL 分隔、重命名/复制双路径、
     子模块状态、conflicted 的 XY 组合、分支头信息 # branch.head/# branch.ab/# branch.oid）；
   - parse_diff_numstat(&[u8]) -> Vec<FileStat>（处理二进制 -  - 与重命名 {old => new}）；
   - parse_log_format(&[u8]) -> Vec<Commit>（使用 %x1f 字段分隔、%x1e 记录分隔、含 parents/refs/时间戳/签名状态占位）；
   - parse_ls_files_stage(&[u8]) -> Vec<UnmergedEntry>（为 M3 冲突做准备）。
3) 所有解析器输入一律为字节切片，禁止先 lossy 转字符串（路径可能非 UTF-8）。
4) 测试：在 crates/git-engine/tests/fixtures/ 下放真实 git 输出样本（≥ 15 个，覆盖各种边界），
   表驱动断言解析结果；另加"fuzz 式"测试：随机字节输入不 panic。
验收：cargo test -p git-engine 全绿；覆盖率 ≥ 85%；回报中列出 fixture 清单与覆盖的边界情况。
```

#### T1.2 GitEngine 抽象与双实现一致性测试

`依赖：T1.1` ｜ `估时：5d` ｜ `审批点：**需要你确认读引擎选型（D-03）**`

```text
【T1.2 GitEngine trait 与双实现】
1) 在 crates/domain 定义与 IO 无关的领域类型：RepositoryInfo、RepoId、StatusReport、FileChange、
   DiffReport、DiffHunk、DiffLine、Commit、Branch、Tag、Remote、StashEntry、LogQuery、Page<T>、
   CommitSpec、StageSpec、ResetSpec、MergeSpec、PullSpec、PushSpec、FetchSpec、ReorderSpec（rebase 预留）。
2) 在 crates/git-engine 定义 trait GitEngine（见 docs/PLAN.md §5.6 的方法清单，
   读操作与写操作都要有，未实现的写方法可先返回 AppError::UNKNOWN("not implemented") 但必须登记 TODO 与所属任务）。
3) 实现 CliGitEngine：
   - 读：discover/status/diff/log/show/branch_list/tag_list/remote_list/stash_list/reflog；
   - 写：init/clone/stage/unstage/commit/amend/reset/checkout/merge/cherry_pick/revert/stash/fetch/pull/push
     （其中 clone/fetch/pull/push 需支持进度回调，解析 --progress 的 stderr 行）。
   所有写操作不在此层做快照（快照由 services 层负责）。
4) 实现 Libgit2Engine：discover/status/diff/log/show/branch_list/tag_list/remote_list；
   写方法一律返回 AppError{code:"UNSUPPORTED_BY_ENGINE"}。
5) 一致性差分测试（关键）：
   在临时目录构造 6 类仓库：① 简单线性 ② 多分叉+merge ③ 重命名+删除 ④ 二进制文件 ⑤ 子模块 ⑥ 大量文件（1000+）。
   对同一仓库分别调用两个引擎的 status/diff/log，断言语义一致（定义"一致"的规范化规则并写成文档注释）：
   - status：文件路径集合、XY 状态、重命名对 必须一致；
   - diff：文件集合与增删行数必须一致；
   - log：oid 序列、parents、subject、时间戳必须一致（排序规则需明确）。
   若发现不一致，不要掩盖：在测试中标记 #[ignore] 并写 docs/GIT-ENGINE-DIFF.md 记录差异与处置决策。
硬约束：domain crate 不得依赖 git2 或 std::process。
验收：cargo test -p git-engine 全绿；差分测试报告写入 docs/GIT-ENGINE-DIFF.md。
⚠️ 若差分测试发现严重不一致，请停下来请求人类决策（继续用 libgit2 还是读路径也改 CLI）。
```

#### T1.3 仓库发现、打开、克隆、初始化

`依赖：T1.2` ｜ `估时：4d` ｜ `审批点：无`

```text
【T1.3 仓库生命周期】
crates/services/repository + crates/commands/repository：
1) repo_discover(path)：向上查找 .git；若 path 本身是裸仓库也要识别；若不在仓库内返回
   AppError{code:"PATH_NOT_REPO", actions:[{初始化仓库},{选择其他目录}]}。
2) repo_open(path)：
   - 调用 discover；写 repositories 表（upsert by canonical path）；
   - 并发审计仓库配置（见 §5.12 威胁规避）：读取 .git/config 与仓库级 config，
     检测危险项 core.fsmonitor、core.sshCommand、filter.*.clean/smudge、alias.* 中含 shell 执行、
     core.pager、core.editor 等，返回 RepoAuditReport 供前端展示警告（不阻塞打开）；
   - 检测 git 版本（git --version）并与最低版本（2.30）比较，低于则返回警告；
   - 返回 RepositoryInfo（含 default_branch、is_bare、is_shallow、is_lfs、worktree 列表）。
3) repo_clone(spec)：支持 URL（HTTPS/SSH）、目标目录、depth（浅克隆）、--recurse-submodules、
   --single-branch；进度通过 JobRunner 广播；失败时给出诊断（凭据/网络/目录已存在/目标非空）。
4) repo_init(spec)：支持指定默认分支名（init -b）、生成 .gitignore 模板（按语言，模板文件随包）与
   License 模板（仅生成文件，不代替用户选择许可证）。
5) repo_recent_list() / repo_forget(id) / repo_close(id)：最近列表基于 last_opened_at 排序，
   支持"从列表移除（不删除磁盘文件）"。
6) 命令全部登记到 docs/API.md；前端只做最小调用验证（真实 UI 在 T1.4）。
验收：对 5 个真实仓库（含 1 个裸仓库、1 个浅克隆、1 个子模块仓库）手工验证 open 与 clone；
   恶意 config 测试仓库必须产生审计警告且不自动执行任何 hook/命令。
```

#### T1.4 工作区状态模型与状态面板 UI

`依赖：T1.3` ｜ `估时：5d` ｜ `审批点：无`

```text
【T1.4 状态面板】
后端：git_status(repo_id) -> StatusReport，字段含：
  head（分支名或 detached + oid）、upstream 与 ahead/behind、operation_state（none|merge|rebase|cherry-pick|revert|bisect）、
  staged[]、unstaged[]、untracked[]、conflicted[]、ignored_count、以及每项的 FileChange
  （path、old_path、index_status、worktree_status、is_binary、is_submodule、is_lfs、size_bytes）。
  ignored 文件默认不返回，需显式参数 include_ignored。
前端 src/features/workspace：
1) 状态面板按分组展示：冲突 / 已暂存 / 未暂存 / 未跟踪；每组可折叠、显示计数；
   支持切换"目录树视图"与"扁平列表视图"（树视图按目录聚合，默认树视图）。
2) 每行：状态图标（新增/修改/删除/重命名/冲突/子模块/LFS，图标语义与颜色必须有文字可读替代）、
   文件名、路径（灰显）、行内操作（暂存/取消暂存/放弃/在编辑器打开/在文件管理器显示/复制路径）。
3) 顶部操作条：全选/取消全选、批量暂存/取消暂存、批量放弃（危险，需 AlertDialog 列出将丢失的修改）、
   刷新、切换是否忽略空白变更。
4) 文件变更行支持多选（Shift/Ctrl/Cmd）与键盘上下移动（j/k 可选）。
5) 性能：使用 VirtualList；10000 个变更文件时首屏渲染 < 1s，滚动流畅。
6) 空态与特殊态：干净工作区、无仓库、detached HEAD、正在 rebase/merge（顶部横幅提示并提供
   继续/中止入口，M3 实现具体逻辑，此处先占位并跳转冲突页）。
7) 数据通过 TanStack Query 获取，key 为 ['status', repoId]；收到 repo:changed 事件时 invalidate。
验收：在一个有 10000 个变更文件的仓库中实测性能并写基准报告；
   E2E 覆盖：打开仓库 → 看到分组 → 批量暂存 → 计数变化 → window.__errs 为空。
```

#### T1.5 Diff 解析与 Diff 查看器

`依赖：T1.4` ｜ `估时：6d` ｜ `审批点：无`

```text
【T1.5 Diff 查看器】
后端：
1) git_diff(repo_id, spec) -> DiffReport。spec 支持：
   { target: Worktree|Index|Commit(oid)|Range(from,to), path: Option<String>, ignore_whitespace: bool,
     context_lines: u8, detect_renames: bool }。
2) 解析统一 diff 为结构化 DiffHunk / DiffLine（含 old_no/new_no、类型、原始文本）。
3) 大文件保护：单文件 diff > 2MB 或行数 > 20000 时返回 truncated 标记 + 前 N 行，
   并提供 force_full 参数供用户显式加载。
4) 二进制文件：返回 is_binary + 大小 + 可选"十六进制前 1KB 对比"（不强制）。
5) 文本 diff 使用 git CLI（--no-color -U<n>），保证与用户终端一致；不得使用 libgit2 的 diff 格式化输出。
前端 src/features/diff：
1) 双模式：并排（side-by-side）与内联（unified），可切换并记忆偏好（settings）。
2) 行号列（旧/新）、变更类型色彩（新增/删除/修改）、行内字符级差异高亮（仅对"修改"行做，
   使用 diff 库计算词级差异；行数过大时自动关闭字符级高亮以保证性能）。
3) hunk 折叠/展开、上下 hunk 跳转、"显示更多上下文"（按需重新请求更大 context_lines）。
4) 语法高亮：使用 Monaco 的 diff 视图或 shiki；选定方案后说明理由与体积影响。
   若使用 Monaco，请确保 Worker 正确配置且按需加载语言（不打包全部语言）。
5) 大 diff 虚拟化：只渲染可见行。
6) 复制/导出：复制 hunk、复制文件 diff、导出为 .patch 文件（由后端提供生成 patch 的能力）。
验收：在真实仓库上验证并排/内联正确性；一个 3000 行 diff 的文件滚动流畅；
   E2E 覆盖切换模式与 hunk 折叠；提供性能基准数据。
```

#### T1.6 行级/块级暂存与取消暂存

`依赖：T1.5` ｜ `估时：5d` ｜ `审批点：无`

```text
【T1.6 部分暂存】
这是本里程碑正确性要求最高的任务，必须做"对拍测试"。
后端 crates/services/staging：
1) git_stage(repo_id, spec) / git_unstage(repo_id, spec)。spec 支持三种粒度：
   - Files(paths[])
   - Hunks(path, hunk_indices[])      // 按 hunk 暂存
   - Lines(path, selections[])        // 按行选择（含部分行的"选取该行的一部分"暂不支持，明确限制）
2) 实现方式：从 git diff 生成补丁 → 按选择裁剪补丁 → git apply --cached --recount --whitespace=nowarn
   （取消暂存用 --cached --reverse）。必须处理：
   - 新增文件（/dev/null 的 old 侧）与删除文件；
   - 重命名文件（含重命名时的内容变更）；
   - CRLF/LF 与 core.autocrlf；
   - 文件末尾无换行符（\ No newline at end of file）；
   - 二进制文件（不允许行级，只允许整体暂存并说明）。
3) 对拍测试（强制）：随机生成仓库状态，随机选择行/块执行暂存，然后执行
   git diff --cached --numstat 与 git diff --numstat，断言两者与"用户选择"完全一致。
   至少 200 组随机用例（可用 proptest 或固定种子循环）。
4) 失败处理：补丁无法应用时返回 AppError{code:"PATCH_APPLY_FAILED", detail: 原始 stderr, hint,
   actions:[刷新状态并重试]}，且**保证仓库状态未被部分修改**（先 dry-run：git apply --check）。
前端：
1) 在 Diff 视图中提供：行 hover 选择、点击行号选择单行、Shift 选择范围、hunk 头部"暂存此块"按钮、
   悬浮操作条"暂存选中行"。
2) 丢弃修改（discard）：仅在"未暂存"区域可用；弹 AlertDialog 展示将丢弃的 diff 摘要与影响，
   确认后执行 git checkout -- <path>（或对 hunk 使用反向 apply）；必须先做 dry-run 检查。
3) 提供快捷键：s 暂存选中、u 取消暂存选中、d 丢弃（需确认）。
验收：对拍测试 200 组全通过；E2E 覆盖行选择 → 暂存 → 状态面板计数变化 → git diff --cached 与预期一致。
```

#### T1.7 提交两段式流程与等价命令生成器

`依赖：T1.6` ｜ `估时：4d` ｜ `审批点：无`

```text
【T1.7 提交（prepare → execute）】
后端 crates/services/commit：
1) commit_prepare(repo_id, spec) -> CommitPlan：
   struct CommitPlan { plan_id: Uuid, repo_id, files: Vec<PathBuf>, message: String, description: Option<String>,
     author: Option<Identity>, sign: SignMode, hooks: Vec<String>, equivalent_command: String,
     head_oid: String, index_fingerprint: String, created_at }
   - spec 支持：message、description、amend、signOff、noVerify、authorOverride；
   - equivalent_command：生成真实可执行的 git 命令（含 -m 的引号转义规则说明），
     若文件数 > 20 则使用 -F <file> 形式的说明文本；
   - index_fingerprint：对 git ls-files --stage 输出取哈希，用于执行前校验仓库未被外部修改。
2) commit_execute(plan_id)：
   - 校验 plan 未过期（TTL 5 分钟）与 index_fingerprint 一致，不一致返回
     AppError{code:"PLAN_STALE", actions:[重新生成计划]}；
   - 调用 SnapshotManager.create(repo_id, "pre-commit")（T1.9 实现，本任务先定义接口并注入）；
   - 写 operation_records 审计；
   - 执行 git commit（用 -F 传消息文件避免转义问题，临时文件放系统临时目录并在用后删除）；
   - 解析 hooks 输出（pre-commit / commit-msg / prepare-commit-msg 的 stdout/stderr），
     失败时返回 AppError{code:"HOOK_REJECTED", detail: 原始输出, actions:[查看输出,禁用 hooks 重试]}；
   - 成功后返回 CommitId 并 invalidate 前端 status/log。
3) 提交信息校验（服务端）：非空、首行 ≤ 72 字符时给出建议（不阻断）、
   检测并拦截"空提交"（无暂存内容）并给出明确提示、检测 message 中是否只有空白。
4) 提供 commit_message_hint(repo_id) -> { recent_messages: Vec<String>, template: Option<String>,
   branch_style: Option<String> }：从最近 20 条提交与分支命名风格推断（纯本地规则，无 AI），
   供前端做"风格提示"（不是自动生成内容）。
前端 src/features/commit：
1) 提交面板：多行消息输入（首行/描述用分隔线视觉区分）、字符计数、最近消息历史下拉、
   签名与 signOff 开关、amend 模式（自动填充上一次消息）。
2) 提交前预览对话框：展示文件清单（分组）、等价 git 命令（可复制）、将要执行的 hooks 列表、
   作者信息；用户确认后才调用 commit_execute。
3) 提交进行中显示进度与 hooks 输出流；失败时展示 HOOK_REJECTED 的原始输出（可折叠）。
4) 快捷：Ctrl/Cmd+Enter 提交；提交按钮在无暂存内容时禁用并提示原因。
验收：为 prepare/execute 写单测（含 PLAN_STALE、空提交、hook 拒绝三类分支）；
   E2E：暂存 → 写消息 → 预览 → 提交 → 历史与状态更新；等价命令在真实终端执行得到同样结果。
```

#### T1.8 Amend、提交表单校验与 Hook 结果展示

`依赖：T1.7` ｜ `估时：2d` ｜ `审批点：无`

```text
【T1.8 Amend 与 hooks 展示】
1) 后端 git_amend_prepare(repo_id, spec)：支持两种 amend：
   - 仅修改提交信息（不改变文件）；
   - 修改信息 + 把当前暂存区并入上一次提交。
   返回 CommitPlan（含等价命令 git commit --amend ...）与"是否会影响已推送提交"的提示
   （通过检查 HEAD 是否包含于任一 remote-tracking ref 判断；若是则标记 warning: "该提交可能已推送到远端"）。
2) 前端在提交面板提供 "Amend 上一次提交" 开关，开启后：
   - 自动填充上一次提交信息；
   - 显示警示条（若可能已推送）：说明修改历史的风险 + 提示可能需要 force-with-lease；
   - 允许选择"仅改信息 / 含暂存内容"。
3) Hook 结果展示组件 HookOutputPanel：区分 pre-commit 失败、commit-msg 拒绝、lint-staged 输出；
   对常见输出做轻量结构化（例如识别 ESLint/prettier 的错误行并高亮），但不得引入 AI 解析。
4) 提供 .git/hooks 状态查看：列出仓库中存在的 hooks 与其是否可执行（仅展示，不编辑）。
验收：单测覆盖"可能已推送"判定；E2E 覆盖 amend 仅改信息与 amend 含内容两条路径，
   且断言 amend 后旧提交 oid 变化、提交数量不变。
```

#### T1.9 快照管理器 v1 与一键回滚

`依赖：T1.7` ｜ `估时：4d` ｜ `审批点：无`

```text
【T1.9 快照 v1】
crates/snapshot：
1) SnapshotManager trait：
   create(repo_id, label, kind: SnapshotKind) -> SnapshotId
   list(repo_id, limit) -> Vec<SnapshotMeta>
   restore(snapshot_id) -> RestoreReport
   diff(snapshot_id) -> SnapshotDiff（与当前状态的差异摘要）
   prune(repo_id, policy)
2) v1 记录内容（写入 snapshots 表）：
   head_oid、index_tree_oid（git write-tree 的结果，注意：不要污染用户 index，需用 GIT_INDEX_FILE 指向临时 index）、
   reflog_ref（创建一个自定义 ref refs/forgedesk/snapshots/<id> 指向 HEAD，保证对象不被 gc）、
   current_branch、detached 状态、operation_state、未跟踪文件清单（路径列表，v1 不备份内容，M3 再备份）。
3) restore 流程：
   - 校验快照可用（reflog_ref 存在）；
   - 先创建"回滚前快照"（防误回滚）；
   - git reset --hard <head_oid>；
   - 用 GIT_INDEX_FILE 恢复 index 到 index_tree_oid（git read-tree）；
   - 校验：HEAD oid、index tree oid 与快照一致，否则返回
     AppError{code:"RESTORE_VERIFY_FAILED"} 且不留下中间态（若首个步骤已改 HEAD 而后续失败，
     必须回到回滚前快照）；
   - 写审计。
   绝不使用 git reflog 或 HEAD@{n} 这类可能被外部操作干扰的方式作为唯一依据。
4) 保留策略：默认每仓库保留最近 50 条或 30 天（以先到者为准），可在设置中调整；
   清理时删除 refs/forgedesk/snapshots/<id> 与表记录。
5) 与写入路径集成：services 层在所有破坏性操作前后自动调用 create（本任务只需接通 commit 路径，
   其余操作在 T2.8/T3.8 接通）。
6) 前端：设置页或仓库页提供"操作历史与快照"列表（本任务最小版：列表 + 回滚按钮 + 确认对话框），
   显示 label、时间、head 短 oid、kind；回滚后展示 RestoreReport。
验收（强制自动化）：破坏性操作 → 回滚 → 断言 git status --porcelain=v2 -z 输出、
   rev-parse HEAD、ls-files --stage 哈希、未跟踪文件集合四项与操作前完全一致。
```

#### T1.10 文件系统监听与状态自动刷新

`依赖：T1.4` ｜ `估时：3d` ｜ `审批点：无`

```text
【T1.10 仓库监听】
1) crates/platform 定义 FileWatcher trait 与实现（notify crate）：
   watch(repo_path, options) -> WatcherHandle，回调去抖动后的变更事件。
   忽略 .git/objects、.git/logs、node_modules、target 等目录（可配置），避免无意义刷新。
2) 去抖动：默认 300ms 合并窗口；单次事件量 > 2000 时只发一次"大量变更"事件并建议手动刷新
   （避免大仓库 checkout 时 UI 抖动）。
3) 后端在仓库打开时启动监听，关闭时停止；发送 Tauri 事件 repo:changed { repoId, kind, paths }。
4) 前端 QueryInvalidation 策略：
   - repo:changed 且涉及工作区 → invalidate ['status', repoId] 与相关 diff query（若当前打开的文件受影响）；
   - 涉及 HEAD/refs（.git/HEAD、.git/refs、.git/packed-refs 变化）→ invalidate ['log', repoId]、['branches', repoId]；
   - 使用 staleTime + 节流保证 1 秒内不重复请求。
5) 平台注意事项：Linux inotify watch 上限不足时检测并提示（读取 /proc/sys/fs/inotify/max_user_watches）；
   macOS FSEvents 的目录级事件做路径过滤；Windows 排除只读属性事件噪音。
6) 提供设置项：自动刷新开/关、去抖动时长。
验收：在外部终端执行 git checkout / git commit / 编辑文件，UI 在 1 秒内更新；
   在 10 万文件仓库上开启监听，CPU 空闲占用接近 0（采样证明）。
```

#### T1.11 操作审计日志

`依赖：T1.7, T1.9` ｜ `估时：1d` ｜ `审批点：无`

```text
【T1.11 审计日志】
1) crates/services 中实现 AuditLog 服务：记录每条写操作（op_type、repo_id、args 摘要、
   开始与结束时间、退出码、stderr 摘要（脱敏）、关联 snapshot_id、是否可逆）。
   args 摘要需脱敏（不得包含 token/密码）并限制长度（≤ 2KB）。
2) 在能力等级为 Mutating 与 Dangerous 的所有 service 方法上统一调用（建议用宏或在与 commands 交界处统一拦截）。
3) 提供命令 audit_list(repo_id?, limit, offset) 与 audit_export(repo_id?, format: json|csv, from, to)
   -> 文件路径（写入用户选择的目录，本任务返回临时文件路径并在设置页可另存）。
4) 前端：设置 → 高级 → 操作历史（表格：时间/类型/结果/耗时/快照，支持筛选与导出按钮）。
5) 保留策略：默认保留 90 天或 10000 条，可配置；导出/清理操作本身也要记录。
验收：单测断言脱敏与长度限制；手工执行若干操作后导出 CSV 内容正确（含中文不乱码，UTF-8 BOM 可选）。
```

#### T1.12 M1 测试补齐与闭环 E2E

`依赖：T1.1–T1.11` ｜ `估时：5d` ｜ `审批点：无`

```text
【T1.12 M1 测试与验收】
1) Rust 覆盖率：cargo llvm-cov 报告，domain 与 git-engine 覆盖率 ≥ 85%，workspace 整体 ≥ 60%；
   未达标处补测试（不要用 #[cfg(test)] 空壳刷覆盖率，必须断言真实行为）。
2) Playwright + tauri-driver E2E（Linux CI 必需）覆盖：
   - 打开仓库 → 状态分组展示 → 行级暂存 → 提交 → 历史更新；
   - discard 流程（含确认对话框取消与确认两条路径）；
   - 提交被 pre-commit hook 拒绝 → 展示输出 → 无半成品提交；
   - reset --hard 后从快照回滚 → 文件恢复。
   每个用例结束断言 window.__errs 数组为空。
3) 建立 window.__errs：在 src/main.tsx 中挂载 window.addEventListener('error') 与
   'unhandledrejection' 收集到 window.__errs，仅在 dev/E2E 生效（生产环境上报到本地日志）。
4) 在 5 个真实仓库上执行手工验收清单（大小从 100 到 100000 提交），
   记录每个仓库的：打开耗时、状态刷新耗时、提交耗时、内存占用，写入 docs/PERF-BASELINE.md。
5) 修复本里程碑暴露的全部 P0/P1 缺陷；编写 M1 验收报告 docs/acceptance/M1.md，
   逐条对照 docs/PLAN.md 的 M1 验收标准给出「通过/不通过 + 证据」。
验收：CI 全绿；E2E 全绿；验收报告无未解释的"不通过"。
```

### 4.3 M2 历史 DAG / 分支 / 远端同步（T2.1–T2.10）

#### T2.1 历史分页查询与 DAG 泳道布局算法

`依赖：T1.2` ｜ `估时：6d` ｜ `审批点：无`

```text
【T2.1 历史查询与 DAG 布局】
后端 crates/services/history：
1) git_log_page(repo_id, query) -> Page<CommitNode>（游标分页，基于提交序号而非 skip/limit 以避免全量扫描）。
   LogQuery 支持：refs（默认 HEAD，多选如 --all）、path（文件历史）、author、since/until、
   message_contains、first_parent_only、max_count、follow_renames。
2) 使用 git log --format 自定义：%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%D%x1f%s%x1e
   加 --date-order（或 topo-order，需说明选择理由并在 UI 提供切换）。
3) 布局引擎（纯逻辑，放 crates/domain/history，严禁 IO）：
   layout(commits: &[CommitNode], mode: LayoutMode) -> GraphLayout
   输出每个节点的 (lane: u16, row: u32, color_index: u16) 与边列表
   (from_oid, to_oid, from_lane, to_lane, kind: Straight|Merge|Branch)。
   规则（必须实现并写入文档注释）：
   - lane 分配：优先复用"等待中的 lane"，按父提交首次出现顺序分配新 lane；
   - lane 颜色稳定：color_index = 首次分配的 lane 序号 % palette_size，不得随分页变化而变；
   - 分支线（Branch）与合并线（Merge）在渲染层用不同样式区分；
   - 支持"折叠已合并分支"模式（对每个 merge，若其第二父的所有祖先都在第一父的祖先集中，可折叠为汇总节点）。
4) 属性测试（proptest，强制）：
   ① 任意随机 DAG：所有父子关系都有对应边，且每条边两端 lane 与节点 lane 一致；
   ② 确定性：同输入两次布局结果完全相同；
   ③ 无重叠：同一 row 区间内同 lane 不得被两个不相关的边同时占用；
   ④ 分页一致性：先取 100 条再取 200 条，前 100 条的 lane 分配必须一致（这是最容易出错的地方，务必断言）。
5) 增量刷新：git_log_refresh(repo_id, since_oid) 只返回比 since_oid 新的提交与受影响的 lane 变化。
验收：属性测试全绿；在 5 万提交的测试仓库上，布局 5000 个节点的耗时 < 200ms（在回报中给出基准数据）。
```

#### T2.2 Canvas 提交图渲染层与命中检测

`依赖：T2.1` ｜ `估时：6d` ｜ `审批点：**需要你确认提交图视觉方案**（红线 R3）`

```text
【T2.2 提交图渲染】
⚠️ 红线：不得模仿任何现有 Git 客户端的图形语言。请自行设计一套原创节点样式
   （例如：圆角胶囊节点 + 内嵌作者首字母 + 分支彩色胶囊标签），并在回报中用文字描述设计决策。
1) 渲染架构（混合渲染）：
   - Canvas 2D 绘制：节点、连线、ref 标签、选中高亮、合并/分支线样式；
   - DOM overlay：交互元素（hover 卡片、右键菜单、拖拽手柄）与无障碍层（屏幕阅读器可读取节点列表）。
2) 性能要求：
   - 视口裁剪：只绘制可见 row 范围 ± 200 行；
   - 分层：离屏 canvas 缓存静态层（连线）与动态层（选中/hover）；
   - DPR 适配（devicePixelRatio 1/1.5/2 均需清晰）；
   - 目标：5 万节点时首屏 < 1.5s，滚动 ≥ 50fps。
3) 交互：
   - 缩放 0.5x–3x（Ctrl/Cmd + 滚轮）、平移（拖动空白区）、迷你地图（可开关）；
   - hover 高亮同一分支链路并显示提交摘要卡片；
   - 单击选中（同步右侧详情面板）、Ctrl/Cmd 点击多选、Shift 点击选择区间（为 M3 rebase 框选做准备）；
   - 右键菜单：复制 oid/复制消息、创建标签/分支、cherry-pick、revert、reset 到此提交（写入菜单但功能在 T2.8 接通）、
     "将此提交设为比较基准"。
4) 无障碍：提供"列表模式"（表格视图）作为等价访问路径，键盘可上下移动节点并按 Enter 打开详情。
5) 提供 dev 性能面板（可开关）：节点数、布局耗时、绘制耗时、fps、内存。
验收：在 5 万与 10 万提交仓库上给出帧率与渲染耗时基准；E2E 覆盖缩放/平移/选中/右键菜单打开。
⚠️ 完成后请求人类审批提交图视觉方案。
```

#### T2.3 历史筛选、搜索与迷你地图

`依赖：T2.2` ｜ `估时：4d` ｜ `审批点：无`

```text
【T2.3 历史筛选与搜索】
1) 顶部筛选栏：分支多选（含"全部分支"）、作者（从仓库作者列表选择）、
   时间范围（预设 + 自定义）、消息关键词、仅显示合并提交 / 仅显示我的提交开关。
2) 搜索：后端 git_log_search(repo_id, query) 使用 git log --grep（区分大小写开关）；
   提供"高亮匹配"与"上一处/下一处"跳转（自动滚动到对应节点并选中）。
3) 文件历史：从文件树或编辑器右键"查看该文件历史"，进入历史页并应用 path 过滤，
   启用 --follow 跟随重命名，并在图中标记重命名事件（例如节点上加 R 徽标）。
4) 过滤器状态同步到 URL query（可分享/可刷新保持），并持久化到 repo 级 settings。
5) 迷你地图：显示视图窗口位置与总节点范围的可视化缩略条，可拖动跳转；
   筛选状态下迷你地图同步更新。
6) 当筛选结果为空时给出明确空态与"清除筛选"按钮。
验收：单测覆盖 filter → query 参数映射；E2E 覆盖按作者筛选 + 关键词搜索 + 跳转高亮；
   路由刷新后筛选状态保持。
```

#### T2.4 提交详情面板

`依赖：T2.2` ｜ `估时：3d` ｜ `审批点：无`

```text
【T2.4 提交详情】
后端 git_commit_detail(repo_id, oid) -> CommitDetail：
  { meta: {oid, short_oid, parents[], author{name,email,time}, committer{...}, subject, body,
           signature_status: None|Good|Bad|Unknown|UnsignedWithKey},
    refs: Vec<RefLabel>, stats: { files_changed, insertions, deletions },
    files: Vec<FileChangeWithStat>, is_merge: bool, is_head: bool, is_pushed: bool }
1) 签名验证：使用 git verify-commit 或 git log --show-signature 的解析结果（不引入 GPG 库），
   状态在 UI 上用文字 + 图标区分（不得只靠颜色）。
2) 文件列表：可点击展开单个文件的 diff（复用 T1.5 组件）；合并提交可切换"相对第一父/第二父"的 diff 视图。
3) 对合并提交，"与第一父比较"与"与第二父比较"必须都能查看（这是常见需求）。
前端：
1) 详情面板支持"钉住"（固定显示）与"跟随选中"两种模式。
2) 元信息区可复制（oid 短/长、作者邮箱、消息）；提供"在浏览器打开"（仅当能推断出远端 URL 时）。
3) 底部操作条：创建分支/标签、cherry-pick、revert、reset 到此（危险，走计划预览 + 快照）、
   "复制为 patch"、"在历史中定位父提交/子提交"。
验收：单测覆盖签名状态解析与合并提交的父选择；E2E 覆盖合并提交的双父 diff 切换。
```

#### T2.5 分支与标签完整管理

`依赖：T1.2` ｜ `估时：4d` ｜ `审批点：无`

```text
【T2.5 分支与标签】
后端（分支）：
  git_branch_list(repo_id, include_remote: bool) -> Vec<Branch>
  git_branch_create(repo_id, name, start_point, checkout, track_upstream)
  git_branch_switch(repo_id, name, opts{force, stash_before_switch, create_as: Option<name>})
  git_branch_rename(repo_id, old, new, rename_remote)
  git_branch_delete(repo_id, names[], force, also_delete_remote)
  git_branch_set_upstream(repo_id, branch, upstream|None)
  git_branch_compare(repo_id, a, b) -> { ahead, behind, commits_only_in_a, commits_only_in_b }
后端（标签）：
  git_tag_list(repo_id) / git_tag_create(repo_id, name, target, message, sign, force)
  / git_tag_delete(repo_id, names[], also_delete_remote) / git_tag_push(repo_id, names[], remote)
关键实现要求：
1) 切换分支前若工作区不干净：提供三种策略（stash 后切换 / 强制切换（危险，需确认 + 快照）/ 取消），
   不得静默失败。
2) 删除分支时：若分支未合并，必须展示"该分支独有提交"清单（oid + 标题）并要求二次确认；
   force 删除必须有快照且标记不可逆风险。
3) 分支名校验：遵循 git check-ref-format 规则（可调用 git check-ref-format 实现），
   拒绝含空格、~^:?*[\、连续点、以 . 或 / 开头结尾等，并给出人话原因。
4) 分支列表按"当前分支置顶 → 本地 → 远端"分组，支持搜索、按最近提交时间排序、
   显示 ahead/behind 徽标与 upstream 名。
前端：分支页 + 侧栏分支切换器（下拉 + 搜索 + 快速创建）；标签页（列表 + 创建/删除/推送）。
验收：单测覆盖切分支三策略、未合并删除确认、分支名校验（≥ 12 个非法用例）；
   E2E 覆盖创建 → 切换 → 重命名 → 删除全流程。
```

#### T2.6 Fetch / Pull / Push 与 Remote 管理

`依赖：T2.5` ｜ `估时：5d` ｜ `审批点：**需要你确认是否接受真实 GitHub 仓库做联调**`

```text
【T2.6 远端同步】
后端 crates/services/sync：
1) Remote CRUD：git_remote_list / add / remove / rename / set_url。提供 URL 校验与建议
   （HTTPS vs SSH 的差异说明）。
2) git_fetch(repo_id, spec{remote: Option<String>, all: bool, prune: bool, tags: bool, depth: Option<u32>})
   进度：解析 git 的 stderr 进度行（"Receiving objects: 45% (123/273)"）为结构化进度事件。
3) git_pull(repo_id, spec{remote, branch, strategy: FfOnly|Merge|Rebase, autostash, allow_unrelated})：
   - 执行前 create 快照；
   - 冲突时返回 PullOutcome::Conflicted { files }，并确保 operation_state 正确（M3 接管解决流程）；
   - autostash 场景需在完成后恢复 stash 并处理恢复冲突。
4) git_push(repo_id, spec{remote, local_branch, remote_branch, set_upstream, force_with_lease: bool|String,
   push_tags, dry_run})：
   - **禁止**出现裸 --force：代码中不得存在 "--force" 字面量（除 --force-with-lease 与
     --force-if-includes 外），并在单测中断言参数构造器不会生成裸 force；
   - force_with_lease 需先 fetch 对应 remote 并展示"远端当前 commit 与本地记录"的对比，让用户确认；
   - 被拒绝（non-fast-forward）时返回 AppError{code:"PUSH_REJECTED", actions:[Fetch 后重试, 使用 force-with-lease, 取消]}。
5) 凭据：
   - HTTPS：从 crates/credentials 取（按 host + login），通过 GIT_ASKPASS 指向一个临时的
     辅助可执行文件（或使用 git 的 credential.helper 机制）来安全传入，**禁止**把 token 放进命令行或 URL；
   - SSH：交给系统 agent；检测 SSH_AUTH_SOCK 与已加载密钥，失败时给出 AUTH_REQUIRED 与配置指引；
   - 设置 GIT_TERMINAL_PROMPT=0，任何需要交互的输入都会失败并转为结构化错误。
6) 全部网络操作必须可取消（CancellationToken 传递并 kill 子进程）。
前端：
1) 顶部同步条：Fetch / Pull（策略下拉，默认取设置）/ Push（含"提交并推送"按钮）；
   显示 ahead/behind、上次 fetch 时间、远端名。
2) 进度：底部状态栏进度条 + 可展开的详细日志（实时追加）。
3) 冲突：弹出引导对话框"检测到冲突，是否使用冲突解决向导"，跳转 M3 页面（M3 前先展示冲突文件列表）。
4) 错误：网络不可达 / 认证失败 / 被拒绝 三类各自的引导（含"打开代理设置"入口）。
测试：使用本地 bare 仓库模拟远端，覆盖 正常 fetch/pull/push、non-ff 拒绝、冲突、取消、断网（无效代理）五类；
   若需要真实 GitHub 联调，请请求人类提供测试仓库。
```

#### T2.7 凭据解析与认证失败诊断

`依赖：T2.6` ｜ `估时：3d` ｜ `审批点：无`

```text
【T2.7 凭据与认证诊断】
1) crates/credentials：基于 keyring 实现 CredentialStore trait：
   store(host, login, kind: Pat|Oauth|Password, secret) / get(host, login) / delete(host, login) / list()
   服务名统一为 "org.forgedesk.app"，account 为 "<provider>:<host>:<login>"；
   使用 secrecy 包装内存中的明文，Drop 时清零；任何日志不得打印 secret。
2) 检测系统凭据库可用性：启动时探测（写入并读取一个哨兵值）；
   在 Linux 上失败时返回 AppError{code:"KEYRING_UNAVAILABLE", hint: 安装 libsecret 或启用加密文件回退}，
   并实现加密文件回退（Argon2 派生密钥 + AES-GCM 加密的本地文件，需用户设置口令，
   在设置页明确说明安全性差异）。
3) git 凭据集成：
   - 提供 authenticate(repo_id, host) -> CredentialHandle，供网络操作使用；
   - 通过 GIT_ASKPASS 指向应用自身的辅助入口（同一可执行文件带 --askpass 参数启动，
     从临时安全通道读取凭据；若实现复杂，可改为生成临时 shell 脚本并在使用后立即删除，
     但必须在文档中说明该权衡）；
   - 支持多次失败后不再重试（避免账号锁定）：连续 3 次失败返回 AUTH_FAILED 并要求用户重新登录。
4) 认证失败诊断：把 git 的认证类 stderr 映射为结构化错误（不泄露凭据）：
   "Authentication failed" / "Permission denied (publickey)" / "could not read Username" /
   "Host key verification failed" / "SSL certificate problem" / "Proxy CONNECT aborted"，
   每条给出原因与 actions（重新登录 / 检查 SSH 密钥 / 检查代理 / 信任主机指纹）。
5) SSH 密钥检测：列出 ~/.ssh 下的公钥与对应私钥是否存在（不读取私钥内容），
   检测 ssh-agent 中已加载的密钥（ssh-add -l），提供"测试连接"（git ls-remote，5s 超时）。
验收：单测覆盖 keyring 读写（用 mock 或临时内存实现）、认证错误映射（≥ 8 个真实 stderr 样本）、
   回退加密文件方案的加解密往返。
```

#### T2.8 Stash / Cherry-pick / Revert / Reset / Reflog

`依赖：T1.9, T2.6` ｜ `估时：5d` ｜ `审批点：无`

```text
【T2.8 Stash 与历史操作】
所有写操作必须：create 快照 → 写审计 → 执行 → 失败时诊断；reset/revert/cherry-pick 的冲突进入冲突状态。
1) Stash：
   git_stash_save(repo_id, {message, include_untracked, keep_index, paths?})
   git_stash_list / git_stash_show(diff) / git_stash_apply(index, {restore_index}) / git_stash_pop / git_stash_drop / git_stash_clear
   git_stash_branch(index, branch_name)（从 stash 创建分支）
   注意：stash 的 diff 需要单独解析（stash 提交有三父结构），
   展示"该 stash 相对其 base 的变更"而不是原始的 stash 提交 diff（这是常见易错点，务必写单测）。
2) Cherry-pick：支持单提交与多提交（区间），-x 选项（记录来源）、--no-commit 选项；
   冲突时进入冲突状态（M3 前先返回冲突文件列表）。
3) Revert：单提交与多提交；-m 用于合并提交（需用户选择主父，UI 必须明确展示要 revert 哪个父）；
   冲突时进入冲突状态。
4) Reset（soft/mixed/hard）：
   - 一律走 prepare → execute 两段式：reset_prepare 返回受影响内容摘要
     （将丢弃的提交数、将丢失的暂存更改、将丢失的工作区更改文件清单）；
   - hard 必须要求用户输入确认（可要求输入 "reset" 或勾选"我已备份"），并创建快照；
   - 明确提示"远端是否已有这些提交"。
5) Reflog：
   git_reflog(repo_id, ref?: Option<String>, limit) -> Vec<ReflogEntry>（oid、旧值、新值、动作、时间、消息）
   提供"从该点创建分支"（把 reflog 的某条恢复成新分支）——这是最安全的恢复方式，UI 中优先推荐；
   以及"重置当前分支到此处"（危险）。
前端：历史页与仓库页提供 Stash 面板、Reflog 面板（时间线风格）；
   所有危险操作使用统一的 DangerousActionDialog 组件（展示计划、影响、快照状态、确认输入）。
验收：单测覆盖 stash 三父 diff 解析、reset prepare 的影响摘要计算、reflog 恢复为新分支；
   E2E 覆盖 stash 保存 → 应用 → 恢复，reset --hard → 从 reflog 恢复。
```

#### T2.9 大仓库性能模式与基准回归

`依赖：T2.1–T2.8` ｜ `估时：4d` ｜ `审批点：无`

```text
【T2.9 性能优化与基准】
1) 建立基准脚本 scripts/bench/：生成 3 类测试仓库（5 万提交 / 10 万提交 + 200 分支 / 1 万个变更文件），
   测量并输出 JSON：打开仓库、状态计算、日志首页、DAG 布局、DAG 首屏渲染、内存峰值。
2) Rust 侧基准使用 criterion（crates/git-engine/benches/、crates/domain/benches/）；
   前端基准用一个 bench 模式页面（?bench=1）输出 performance.measure 结果。
3) 优化项（按需实施，必须有数据支撑，禁止无测量优化）：
   - 状态计算改走 libgit2 + 后台线程，缓存上次结果并按 mtime/refs 变化失效；
   - 日志分页批量 500 条/次，前端预取下一页；
   - DAG 布局在 Rust 侧线程池执行，结果按 (repo_id, tip_oids_hash, mode) 缓存（LRU，上限 50）；
   - 前端仅渲染可见区域，ref 标签超过 3 个合并为 "+N"；
   - 大仓库自动进入"性能模式"：关闭字符级 diff、缩小上下文行数、限制首屏节点数（可在设置强制关闭）。
4) 在 CI nightly 中运行基准并把结果写入 docs/PERF-BASELINE.md（提交到仓库或用 artifact 保存），
   若相对基线退化 > 10% 则 nightly 失败。
验收：在 10 万提交仓库上达到 docs/PLAN.md §10.9 的指标；回报中给出优化前后的对照数据表。
```

#### T2.10 M2 测试补齐与验收报告

`依赖：T2.1–T2.9` ｜ `估时：3d` ｜ `审批点：无`

```text
【T2.10 M2 验收】
1) DAG 正确性自动化：构造含 线性 / 分叉合并 / octopus merge / 游离 HEAD / 空提交 / 重复合并 的测试仓库，
   断言布局结果的父子边集合与 git log --graph 语义等价（实现一个基于祖先关系的独立断言器，
   不要依赖字符串比对）。
2) E2E 覆盖：历史页浏览与筛选、分支创建切换删除、fetch/pull/push（对本地 bare 远端）、
   冲突提示跳转、stash 全流程、reset 影响预览与取消。
3) 全部写操作必须有快照 + 审计记录的自动化断言（遍历 operation_records 与 snapshots 关联性）。
4) 编写 docs/acceptance/M2.md：逐条对照 PLAN 的 M2 验收标准给出通过/不通过与证据（含基准数据截图或数字）。
5) 修复本阶段暴露的全部 P0/P1 缺陷。
验收：CI 全绿；覆盖率达标（domain ≥ 85%、整体 ≥ 60%）；验收报告完整。
```

### 4.4 M3 冲突 / Rebase 可视化 / 快照回滚（T3.1–T3.11）

> **本里程碑是产品的差异化核心，也是最容易失败的部分。** 每个任务完成后请如实回报，不要为了让验收通过而掩盖问题。

#### T3.1 冲突解析与冲突状态机

`依赖：T1.2, T2.8` ｜ `估时：4d` ｜ `审批点：无`

```text
【T3.1 冲突模型与状态机】
crates/domain/conflict + crates/services/conflict：
1) 冲突数据来源：**必须**使用 git 的 index stage（git ls-files -u 与 git show :1:<path> / :2: / :3:），
   不得依赖工作区文件中的 <<<<<<< 标记（用户可能已手动编辑或标记被破坏）。
2) 数据结构：
   struct ConflictState { op_kind: Merge|Rebase|CherryPick|Revert, op_in_progress: bool,
     current_step: Option<u32>, total_steps: Option<u32>, head_name: Option<String>,
     into_branch: Option<String>, files: Vec<ConflictFile>, can_continue: bool, can_abort: bool, can_skip: bool }
   struct ConflictFile { path, kind: Text|Binary|DeletedByUs|DeletedByThem|AddedByBoth|AddedByUs|AddedByThem,
     base: Option<FileBlob>, ours: Option<FileBlob>, theirs: Option<FileBlob>, worktree_exists: bool }
   struct FileBlob { size, is_binary, encoding_hint, content: Option<String> }  // 内容 > 2MB 时置 None
3) 操作状态检测：通过 .git/MERGE_HEAD / REBASE_HEAD / CHERRY_PICK_HEAD / REVERT_HEAD / rebase-merge/ 目录
   / rebase-apply/ 目录 / sequencer/ 目录综合判断，不能用单一文件是否存在判断。
   rebase 的进度（current/total）从 .git/rebase-merge/msgnum 与 end 读取。
4) git_conflict_state(repo_id) / git_conflict_mark_resolved(repo_id, paths[])（执行 git add，并校验该文件
   已无未解决的 stage 条目）/ git_conflict_continue() / git_conflict_abort() / git_conflict_skip()。
   continue 前必须检查所有冲突文件是否已解决；未解决时返回 CONFLICT_UNRESOLVED 并列出未解决文件。
5) abort 必须：创建快照 → 执行 abort → 校验回到操作前状态（对比操作开始时的 HEAD 与分支名）→ 写审计。
6) 交互式 rebase 的 continue 需要驱动 sequencer（git rebase --continue 可能因提交信息编辑器而阻塞 →
   必须设置 GIT_EDITOR=true 或使用 --no-edit 语义，需实测确认并用单测锁定行为）。
验收：单测在 4 类冲突场景（merge/rebase/cherry-pick/revert）下断言状态检测、进度读取、continue 前置校验；
   用真实的 <<<<<<< 已被手动删除但 index 仍冲突的文件验证"不依赖标记符"这一要求。
```

#### T3.2 三栏冲突编辑器（核心）

`依赖：T3.1` ｜ `估时：8d` ｜ `审批点：**需要你确认三栏编辑器交互方案**`

```text
【T3.2 三栏冲突编辑器】
⚠️ 红线：不得复刻任何现有工具（尤其 GitKraken/Fork）的合并编辑器视觉与交互；
   请自行设计并说明设计决策。允许参考通用的"三方合并"概念，但视觉与交互必须原创。
1) 布局（原创设计，供参考但请自行决策）：
   - 左栏 = 本地（ours）、中栏 = 结果（result）、右栏 = 远端（theirs）；
   - 顶部工具条：切换"显示基线（base）"、切换"仅显示冲突块/显示全文"、上一处/下一处未解决、
     显示行号开关、字体大小；
   - 底部固定操作条：保存并标记已解决 / 标记已解决（不保存）/ 中止操作 / 跳过（rebase 时）。
2) 冲突块模型：
   - 由 T3.1 的三个 blob 计算冲突区块（使用 diff3 风格算法，需实现或使用纯 Rust 库；
     若使用库，必须说明许可与算法）；
   - 每个块提供：[采用本地] [采用远端] [两者保留（本地在前/远端在前）] [手动编辑]；
   - 未解决块以醒目样式标记（必须同时用图标 + 边框 + 文字，不得只靠颜色）；
   - 已解决块折叠为紧凑的绿色摘要行，可展开修改。
3) 结果文本编辑：
   - 结果栏可直接编辑（contenteditable 或绑定到轻量编辑器；**不要**在 M3 引入 Monaco 的 diff 模式做合并，
     避免与 M5 的编辑器规划冲突）；
   - 编辑后该块标记为"自定义"；
   - 提供撤销/重做（至少 20 步）。
4) 行内字符级差异：对 ours/theirs 的对应块做词级 diff 并高亮；块超过 500 行时自动关闭（性能）。
5) 保存：写回工作区文件（保持原文件换行符风格与 BOM 设置）→ git add → 刷新 status。
   保存前检查结果中是否残留 <<<<<<< / ======= / >>>>>>> 标记，若残留则警告（不阻断，允许用户确认）。
6) 二进制冲突（图片/PDF/其他）：展示"无法显示差异"，提供 [采用本地] [采用远端] [选择文件替换…] 三个操作，
   并在可能时提供图片的并排预览。
7) 删除类冲突（DeletedByUs / DeletedByThem / AddedByBoth）：给出专用文案与"保留/删除"按钮，
   绝不以报错形式呈现。
8) 文件列表侧栏：全部冲突文件 + 已解决/未解决计数 + 点击切换；已解决文件可点开复查。
验收（强制）：
   - E2E：文本冲突全流程（含两者保留、手动编辑、逐块操作）→ 结果文件无残留标记（自动 grep 断言）；
   - E2E：二进制冲突处理路径不崩溃；
   - E2E：一个 20 文件 / 200 冲突块的场景，操作延迟 < 100ms（在回报中给出测量方法）；
   - 单测：冲突块计算算法对 20 组人工构造的三方输入输出符合预期。
⚠️ 完成后请求人类审批三栏编辑器交互方案。
```

#### T3.3 冲突解决的键盘流与批量操作

`依赖：T3.2` ｜ `估时：3d` ｜ `审批点：无`

```text
【T3.3 冲突键盘流与批量】
1) 键盘操作（必须全部可用，且不破坏输入框内的正常输入）：
   j/k 或 ↑/↓ 移动选中块；o 采用本地（ours）；t 采用远端（theirs）；b 两者保留；
   Tab 在三个面板间切换焦点；n/p 上一处/下一处未解决；Ctrl/Cmd+S 保存；Ctrl/Cmd+Enter 标记并继续。
2) 批量操作（需二次确认并展示影响清单）：
   "所有冲突块采用本地"/"所有采用远端" → 必须弹出确认对话框，说明会覆盖哪些内容量（块数、行数）。
3) "整个文件采用本地/远端"（文件级快捷操作）。
4) 解决进度：顶部显示 "已解决 3 / 共 7 个文件"，并提供"全部标记为已解决并继续"（仅在全部文件都已解决时启用）。
5) 无障碍：所有块操作可仅用键盘完成；焦点可见；屏幕阅读器可读取"第 N 块冲突，本地 M 行，远端 K 行"。
验收：E2E 用纯键盘完成一次完整冲突解决（不点鼠标）；批量操作确认对话框在取消时不产生任何修改。
```

#### T3.4 Merge 流程可视化

`依赖：T3.1` ｜ `估时：3d` ｜ `审批点：无`

```text
【T3.4 Merge】
1) git_merge_prepare(repo_id, spec{source, strategy: Merge|Squash|FastForwardOnly|NoFf|Ours|Theirs, message}) -> MergePlan：
   包含：是否可以 fast-forward、将产生的提交信息（含默认合并信息）、
   "将要合并的提交"清单（source 独有的提交）、冲突预检（用 git merge-tree 或 merge --no-commit --no-ff 的 dry-run
   在临时 index 中预演，避免污染工作区；若实现复杂可退化为"执行后再报冲突"，但必须在文档中说明权衡）、
   等价命令字符串、是否需要快照。
2) git_merge_execute(plan_id)：快照 → 执行 → 成功或冲突。
3) 合并后：若产生了合并提交，跳转历史页并高亮该提交；
   若配置了"合并后删除源分支"，执行删除（二次确认）。
4) 中止：git_merge_abort（走 abort 统一流程）。
5) git_merge_continue（处理冲突后继续）：需提交合并信息（默认信息需允许编辑，且提供"使用默认信息"快捷）。
前端：合并对话框（源分支选择 + 策略 + 预览 + 冲突预检结果 + 等价命令）；
   合并进行中在状态栏常驻横幅（"正在合并，3 个文件冲突" + 继续/中止按钮）。
验收：单测覆盖 fast-forward 与非 ff 判定、squash 后不产生合并提交、
   冲突预检与真实执行结果一致（对同一场景分别预检与执行，断言一致性）。
```

#### T3.5 Rebase 计划模型（纯逻辑 + 属性测试）

`依赖：T1.2` ｜ `估时：4d` ｜ `审批点：无`

```text
【T3.5 RebasePlan 纯逻辑】
crates/domain/rebase（**严禁 IO**）：
1) 数据模型：
   enum RebaseStep { Pick, Reword { message: String }, Edit, Squash, Fixup, Drop }
   struct RebasePlan { base: String /* oid */, head: String, steps: Vec<(String /* oid */, RebaseStep)>,
                        allow_flatten_merges: bool, autosquash: bool }
2) validate(&self, repo_graph: &GraphView) -> Result<(), Vec<PlanError>>：
   - 禁止 drop 全部提交；
   - 禁止把第一个 step 设为 squash/fixup（必须前面有可合并的提交）；
   - 禁止对包含 merge 的提交做 squash（除非 allow_flatten_merges，且此时必须提示会丢失合并结构）；
   - 禁止出现重复 oid；
   - steps 中的 oid 必须都是 base..head 区间内（含）的提交且顺序合法（拓扑可达顺序）。
3) to_todo_file(&self) -> String：生成与 git rebase -i 兼容的 todo 内容
   （pick/reword/edit/squash/fixup/drop + 短 oid + 主题；squash 与 fixup 的正确区分；
   reword 需要在执行过程中能注入新消息 → 说明采用的方式：写 todo 后按序执行并用 GIT_SEQUENCE_EDITOR 注入）。
4) preview(&self, commits: &[CommitNode]) -> RebasePreview：
   计算"执行后的提交列表"（新顺序、合并后的提交消息草案、被丢弃的提交、受影响的提交总数、
   是否会影响已推送提交（若区间内任一提交存在于 remote-tracking ref 中则标记 warning））。
5) 属性测试（proptest）：
   ① 任意合法 plan 生成的 todo 文件可被真实 git 解析
      （在临时沙箱仓库执行 GIT_SEQUENCE_EDITOR='cat > /dev/null' git rebase -i 的干跑，断言无格式错误）；
   ② preview 预测的"结果提交数量"与真实执行后的数量一致（对 20 组随机合法 plan 验证）；
   ③ validate 对人为构造的非法 plan 必须拒绝（每个 PlanError 至少一个用例）。
验收：属性测试全绿；domain crate 无任何 IO 依赖（用 cargo tree 或依赖断言测试保证）。
```

#### T3.6 拖拽式 Rebase 面板与预览树

`依赖：T3.5, T2.2` ｜ `估时：6d` ｜ `审批点：**需要你确认 rebase 面板交互方案**`

```text
【T3.6 可视化 Rebase】
⚠️ 红线：不得复刻竞品的 rebase 面板视觉与交互，请自行设计并说明。
1) 入口：
   - 历史页框选一组提交 → 右键"整理这些提交"；
   - 历史页右键单个提交 → "修改此提交信息（reword）"/"编辑此提交（edit）"/"丢弃此提交（drop）"；
   - 提交详情面板的"整理历史"按钮。
2) 面板布局（自行设计）：
   - 左侧/主区：可拖拽排序的提交列表（每项显示短 oid、主题、作者、时间）；
     每项右侧有操作下拉（Pick 默认 / Reword / Edit / Squash / Fixup / Drop）；
     拖拽重排时必须有明确的"落点指示"，并支持键盘替代方案（Alt+↑/↓ 移动当前项）。
   - 右侧：预览区，展示"执行后的新历史"（复用 DAG 渲染的简化版本，或列表形式），
     标注：被压缩的提交（合并为一组）、被丢弃的提交（划掉）、消息被修改的提交（标记）、
     受影响的提交总数、是否可能影响已推送提交的警告。
   - 底部：等价 todo 内容（可展开查看，让用户学习 git rebase -i）+ 执行/取消按钮。
3) 合法性：非法操作（如把第一项设为 squash）在 UI 层即时禁用并给出人话原因（同时后端 validate 兜底）。
4) 执行前必须：展示确认对话框（含"将重写 N 个提交的历史，若已推送需要 force-with-lease"）+ 创建快照。
5) 执行采用 step-by-step 驱动：
   - 后端在 sequencer 中逐步推进，每步完成后推送事件 rebase:step { current, total, oid, action }；
   - 若某步产生冲突 → 暂停并跳转冲突向导（T3.2），解决后回到 rebase 流程；
   - 面板保持打开并显示进度，支持"中止并还原"。
验收：E2E 覆盖 ① squash 三个提交 ② drop 中间提交 ③ reword 消息 ④ 拖拽重排顺序
   ⑤ 执行中冲突 → 解决 → 继续 ⑥ 执行中冲突 → 中止 → 快照还原；
   并断言新历史与 preview 预测一致。
⚠️ 完成后请求人类审批 rebase 面板交互方案。
```

#### T3.7 Rebase 逐步执行与冲突穿插

`依赖：T3.6` ｜ `估时：5d` ｜ `审批点：无`

```text
【T3.7 Rebase 执行引擎】
1) 执行方式决策：不要依赖 git rebase -i 的交互式 sequencer 长期驻留（编辑器会阻塞）。
   推荐实现：把 RebasePlan 转成 todo 文件 → 通过 GIT_SEQUENCE_EDITOR 注入 → 执行 git rebase -i
   （或 git rebase --onto 序列），并在需要 reword/edit 时通过 GIT_EDITOR 注入消息。
   请先用一个 spike 脚本验证以下三个场景在 Windows/macOS/Linux 上均可非交互执行：
   ① reword 注入消息；② edit 暂停后由应用手动 git commit --amend 再 continue；
   ③ squash 时消息合并（--autosquash 或手动注入）。
   若某平台无法非交互化，请记录并给出降级方案（例如逐条 cherry-pick 到临时分支再重置，纯 CLI 实现）。
2) 实现 RebaseExecutor：
   - 状态机：Idle → Running → Paused(Conflict) → Paused(Edit) → Completed | Aborted | Failed；
   - 每步执行后读取 .git/rebase-merge/msgnum 与 end 上报进度；
   - Paused(Edit)：暂停并提示用户"修改完成后点击继续"，继续时执行 git commit --amend（允许改内容与消息）再 git rebase --continue；
   - 冲突：上报冲突状态并等待解决；
   - 失败：拉取原始 stderr → diagnostics → 返回结构化错误 + "中止并还原"建议。
3) 全部执行过程写审计，并保证幂等性：重复调用 continue 不会产生重复提交（用 sequencer 状态判断）。
4) 提供 git_rebase_preview_only(plan) 供 UI 预演（不修改仓库）。
验收：单测 + 集成测试覆盖上述三种暂停场景（在临时仓库上真实执行 git 命令）；
   三平台 CI 均须通过（若某平台无法自动化，请在 CI 中标记并说明，同时提供手工验证记录）。
```

#### T3.8 快照 v2（工作区备份与磁盘控制）

`依赖：T1.9` ｜ `估时：5d` ｜ `审批点：**需要你确认磁盘占用阈值策略**`

```text
【T3.8 快照 v2】
1) 在 v1 基础上增加：
   - 未跟踪文件的**内容备份**：复制到 app_cache_dir()/snapshots/<repo_id>/<snapshot_id>/untracked/，
     保持相对路径结构；备份清单写入 snapshots 表（新增字段 manifest_json 与 backup_bytes）；
   - 已跟踪但被修改的文件不备份（可通过 HEAD + index 恢复），以减少体积；
   - 被忽略文件（gitignore）默认不备份，可在设置中开启"包含忽略文件"。
2) 磁盘控制（**本节是你需要决策的重点**）：
   - 默认单快照备份上限：200MB（可配置 0 = 不限制）；
   - 预估体积：先执行 git status --porcelain 统计未跟踪文件数与总大小；
     若超过上限 → 不静默跳过，而是返回 created_with_warning 状态，
     在 UI 明确提示"本次快照未包含 N 个未跟踪文件（共 X MB），回滚时这些文件不会被恢复"，
     并要求用户确认（危险操作前的对话框内展示）；
   - 单仓库快照总占用上限：默认 2GB，超限时按 LRU 清理（清理前在 UI 提示）；
   - 提供"立即清理快照缓存"按钮与当前占用显示（设置页）。
3) 幂等与并发：同一仓库同时创建快照时串行化（per-repo 互斥锁）；
   备份过程中应用崩溃 → 下次启动检测孤儿目录并清理。
4) 恢复流程升级：
   - 恢复未跟踪文件（删除当前多余的未跟踪文件？——**不做自动删除**，只恢复快照中的文件，
     并在报告中列出"当前存在但快照中不存在的未跟踪文件"，由用户决定是否清理）；
   - 恢复后执行完整校验（HEAD / index / 已跟踪文件哈希 / 未跟踪文件集合）并输出 RestoreReport。
5) 提供 snapshot_diff(snapshot_id)：列出"快照与当前状态的差异"（哪些文件会变、哪些会丢失、哪些会恢复）。
验收：自动化测试断言"含未跟踪文件的工作区 → reset --hard + clean -fdx → 回滚后
   未跟踪文件内容逐字节恢复"；断言超阈值时返回 created_with_warning 且 UI 提示正确；
   断言 LRU 清理后不残留孤儿目录。
⚠️ 完成后请求人类确认磁盘阈值默认值（200MB / 2GB 是否可接受）。
```

#### T3.9 回滚校验器与失败安全

`依赖：T3.8` ｜ `估时：3d` ｜ `审批点：无`

```text
【T3.9 回滚校验与失败安全】
1) 实现 RepoFingerprint：{ head_oid, head_ref, index_tree_oid, tracked_files_hash(可选，大仓库跳过),
   untracked_paths_sorted_hash, operation_state }，提供 capture(repo_id) 与 compare(a, b) -> Diff。
2) 回滚事务化：restore 分为阶段（下快照 → 恢复 HEAD → 恢复 index → 恢复未跟踪文件 → 校验），
   每阶段记录进度；任一步失败：
   - 停止后续步骤；
   - 尝试回退到"回滚前快照"；
   - 若回退也失败，进入"紧急模式"：输出详细的恢复指引（含可复制的 git 命令）到 UI 与日志，
     并把快照 ID 与备份路径显式展示给用户（绝不让用户无从下手）。
3) 幂等：对同一快照重复 restore 必须安全（第二次为 no-op 或结果一致），并有测试断言。
4) 崩溃恢复：restore 过程中写入 snapshots 表的 restore_in_progress 标记与步骤；
   启动时若发现未完成的 restore，提示用户"检测到上次回滚未完成，是否继续/查看详情/放弃"。
5) 校验报告：RestoreReport { snapshot_id, stages: Vec<StageResult>, verified: bool, report_lines: Vec<String> }，
   在 UI 中以可折叠清单展示（用户能看到"HEAD 已恢复 / index 已恢复 / 未跟踪文件已恢复 12 个 / 校验通过"）。
验收：故障注入测试（在恢复的中间阶段人为抛错）断言 ① 不留下中间态 ② 输出可执行的恢复指引
   （测试中真的执行指引命令并断言仓库可用）；幂等测试通过。
```

#### T3.10 操作历史面板与一键回滚入口

`依赖：T1.11, T3.9` ｜ `估时：3d` ｜ `审批点：无`

```text
【T3.10 操作历史与回滚入口】
1) 后端：operation_history(repo_id, limit, offset, filters{op_type, only_dangerous, only_reversible})
   返回操作记录 + 关联快照 + 当前是否仍可回滚（校验快照 ref 是否存在）。
2) 前端 src/features/operations：
   - 时间线/表格视图，按天分组；每行显示：时间、操作类型（人话名称 + 原始 git 命令摘要）、
     结果（成功/失败）、影响（文件数/提交数）、是否有快照、[回滚到此点] 按钮；
   - 支持筛选与搜索；失败的操作用醒目但非仅颜色的样式（图标 + 文案）；
   - 点击行展开详情：原始参数（脱敏后）、stderr 摘要、关联快照的 snapshot_diff 摘要。
3) 回滚交互：
   - 点击"回滚"→ 展示 snapshot_diff（将发生什么）→ 二次确认（要求输入确认词或勾选）→ 执行 → 展示 RestoreReport；
   - 回滚本身作为一条新操作记录追加（可追溯）。
4) 在状态栏提供"最近一次可回滚点"指示器：显示最近一次破坏性操作后是否可回滚，
   点击可直接回滚（这是本产品的核心安全感知入口，务必显眼但不过度打扰）。
5) 空态：无操作记录时引导用户（说明"每一次危险操作都会自动打点"）。
验收：E2E 覆盖 执行危险操作 → 时间线出现记录 → 回滚 → 校验报告展示 → 再次回滚为幂等；
   断言状态栏指示器在危险操作后立即出现。
```

#### T3.11 破坏性操作安全测试矩阵

`依赖：T3.1–T3.10` ｜ `估时：5d` ｜ `审批点：无`

```text
【T3.11 破坏性操作安全测试矩阵】
实现自动化测试套件（Rust 集成测试 + 少量 E2E），逐条覆盖 docs/PLAN.md §10.6 的 15 个场景，
每条都必须断言"可回滚且回滚后逐字节一致"：
 1) reset --hard HEAD~1（有已提交 + 已暂存 + 未跟踪）
 2) reset --mixed（有已暂存）
 3) rebase 中途 abort
 4) rebase 中途失败（hook 拒绝）→ 无残留 .git/rebase-merge
 5) checkout -f 丢弃修改
 6) clean -fdx
 7) stash drop
 8) push --force-with-lease（lease 不匹配时必须拒绝）
 9) branch -D 未合并分支 → 通过 reflog 恢复
10) cherry-pick 冲突后 abort
11) 删除 worktree（明确提示不可恢复的边界）
12) 回滚过程中崩溃 → 重启后仍可回滚（幂等）
13) 磁盘空间不足时创建快照（用 mock 或限制配额模拟）→ 提前拒绝并提示，不产生半成品
14) 仓库只读 → 明确错误且不损坏
15) 外部进程并发写仓库 → 指纹校验发现并提示刷新
要求：
- 每条用例构造独立的临时仓库（fixture 生成器），互不干扰；
- 断言工具统一为 assert_repo_equals(fingerprint_before, fingerprint_after)；
- 对第 12 条，用子进程模拟崩溃（在恢复中途 kill），再启动新进程继续；
- 测试耗时控制在 CI 可接受范围（< 10 分钟），并行执行。
额外任务：把此套件接入 CI，作为发布前的强制门禁（任何一条失败即阻断发布）。
验收：15 条全部通过并在回报中逐条列出证据（测试名 + 关键断言）。
```

### 4.5 M4 GitHub 集成（T4.1–T4.12）

#### T4.1 HostProvider 抽象与能力声明

`依赖：T1.2` ｜ `估时：3d` ｜ `审批点：**需要你确认 D-01 是否改变（是否需要 GitLab/Gitea 预埋）**`

```text
【T4.1 Provider 抽象层】
crates/provider：
1) 定义与具体平台无关的领域模型（放 crates/domain/provider）：
   ProviderId（GitHub|GitLab|Gitea）、HostConfig { id, kind, base_api_url, base_web_url, label },
   RemoteRef { provider, host, owner, repo }、User、Repository、Branch、PullRequest、PullRequestReview、
   Issue、Label、Milestone、WorkflowRun、WorkflowJob、Release、Notification、RateLimitState、Page<T>、Cursor。
2) 定义 trait HostProvider 与能力声明：
   struct ProviderCapabilities { pulls: bool, pull_reviews: bool, inline_comments: bool, issues: bool,
     labels: bool, milestones: bool, actions: bool, actions_logs: bool, checks: bool, releases: bool,
     release_assets: bool, gists: bool, notifications: bool, code_search: bool, graphql: bool }
   要求：UI **必须**依据 capabilities 隐藏/禁用功能，禁止在代码中以 provider 名字做 if 判断
   （用 grep 断言：src/ 下不得出现 'github' 作为分支条件，仅有展示用的显示名除外）。
3) 定义错误类型 HostError（可转换为 AppError）：NotAuthenticated、TokenExpired、NotFound、
   Forbidden{required_scope}、Validation{fields}、RateLimited{reset_at, remaining}、Network、Unsupported。
4) 在 crates/provider 提供 ProviderRegistry：注册多个 HostConfig → HostProvider 实例，
   支持按 RemoteRef 解析 URL（解析 HTTPS 与 SSH 两种形式：git@github.com:owner/repo.git
   与 https://github.com/owner/repo.git，以及自建 host 的路径前缀）→ RemoteRef。
   URL 解析必须写表驱动单测（≥ 15 个 URL 变体，含企业版带端口与子路径的情况）。
5) 本任务只定义抽象与 GitHub 的空实现骨架（每个方法返回 Unsupported），
   真正的实现从 T4.3 起逐步替换。
验收：单测覆盖 URL 解析与能力声明；grep 断言无 provider 名硬编码分支；cargo test 全绿。
```

#### T4.2 HTTP 客户端与限流/缓存中间件

`依赖：T4.1` ｜ `估时：3d` ｜ `审批点：无`

```text
【T4.2 HTTP 层】
1) 基于 reqwest 构建统一 HttpClient：
   - 用户代理：`ForgeDesk/<version> (+<repo_url>)`（GitHub 要求可识别的 UA）；
   - 超时：连接 10s、总 30s（长请求可覆盖）；
   - 连接池复用；HTTP/2；
   - 代理：从设置读取（HTTP/HTTPS/SOCKS5、no_proxy 列表），支持按 host 覆盖；
    - 自定义 CA 证书导入（用于企业自建实例）；
   - 重试：仅对 5xx、超时、连接错误重试，最多 3 次，指数退避（250ms/1s/4s）+ 抖动；
     4xx 一律不重试（429 除外，按 Retry-After 等待一次）。
2) 限流中间件：
   - 解析响应头 x-ratelimit-limit / remaining / reset / used 与 retry-after，写入共享的 RateLimitState；
   - remaining == 0 时立即返回 HostError::RateLimited 而不发请求（fail fast）；
   - 通过 Tauri 事件 rate-limit:changed 通知前端显示额度与重置时间。
3) ETag 缓存中间件：
   - 对 GET 请求保存 ETag 与响应体到 api_cache 表（key = method + url + 账号 + provider）；
   - 下次请求带 If-None-Match，304 时从缓存返回；
   - 缓存条目带过期时间（默认 5 分钟，可被调用方覆盖），提供 invalidate_cache(key_prefix) 命令；
   - **不得缓存**包含敏感信息的响应（如 token 交换响应）——实现时用显式 allow_cache 标志，默认不缓存。
4) 日志：请求方法与 URL（脱敏 query 中的 token）、状态码、耗时、是否命中缓存；
   响应体不打日志（避免泄露），仅在 trace 级别且开启显式调试开关时输出前 2KB（脱敏 + 用户可见警告）。
5) 在 CI 中使用 wiremock 起本地 mock server 验证重试、限流、304 三条路径。
验收：单测覆盖重试策略（5xx 重试、4xx 不重试、429 按 Retry-After）、限流 fail fast、ETag 304 复用；
   断言日志中不会出现 Authorization 头。
```

#### T4.3 GitHub OAuth Device Flow 登录

`依赖：T4.2` ｜ `估时：4d` ｜ `审批点：**需要你创建 GitHub OAuth App 并告知 Client ID**`

```text
【T4.3 Device Flow 登录】
⚠️ 本任务需要人类协助：请在 GitHub 上创建一个 OAuth App（无需付费），
   勾选 "Enable Device Flow"，并把 **Client ID** 提供给我（Client Secret **不要**提供，
   Device Flow 的公开客户端不需要 secret）。请在开工前请求审批。
1) 实现 GitHubAuthFlow：
   - POST /login/device/code（scope: repo read:user user:email notifications gist，
     说明每项 scope 的用途并允许用户减少授权）；
   - 返回 { user_code, verification_uri, expires_in, interval, device_code }；
   - 轮询 POST /login/oauth/access_token，处理四类分支：
     authorization_pending（按 interval 继续）、slow_down（interval += 5s）、
     expired_token（失败并提示重试）、access_denied（用户拒绝，明确提示）；
   - 轮询使用可取消的任务，UI 关闭或超时即停止（避免后台一直打）。
2) 命令：auth_login_device_start(provider, scopes) -> DeviceLoginSession、
   auth_login_device_poll(session_id) -> PollResult（由前端按 interval 轮询或改为后端推送事件，
   二选一并说明理由）、auth_login_device_cancel(session_id)。
3) 成功后：GET /user 获取 login/avatar/name/email → 存 accounts 表 + token 存 keyring（T2.7 的 CredentialStore）
   → 返回 Account。
4) PAT 登录：auth_login_pat(provider, host, token, label) → 调用 GET /user 验证 token 有效性
   （无效则报错且不落盘）。
5) 企业版支持：允许自定义 base_api_url，登录流程适配（GitHub Enterprise 的 Device Flow 端点路径需确认）。
6) 前端：登录页提供两种方式，Device Flow 展示大号 user_code + "复制并打开浏览器"按钮 +
   实时状态（等待授权/成功/失败），并提供"我已完成授权"手动触发一次立即检查的按钮。
7) 安全：token 永不写日志/永不出现在前端（前端只拿 Account 元数据）；
   提供 auth_logout(account_id)（删除 keyring 条目 + accounts 记录，并可选清除该账号的 api_cache）。
验收：wiremock 覆盖上述四类轮询分支与 PAT 无效场景；手工用一个真实 GitHub 账号完成一次完整登录（需人类配合）。
⚠️ 开工前请求人类提供 OAuth App Client ID。
```

#### T4.4 多账号管理与 keyring 存储

`依赖：T4.3` ｜ `估时：4d` ｜ `审批点：无`

```text
【T4.4 多账号】
1) accounts 表与领域模型支持：多账号（同一 provider 多账号、不同 host）、账号标签（如"工作"/"个人"）、
   头像、scopes、创建时间、最后使用时间。
2) 按仓库绑定账号：repositories 表增加 account_id 字段（可为空）；
   规则解析顺序：仓库显式绑定 → host 匹配的默认账号 → 全局默认账号 → 无（提示登录）。
   git 网络操作（fetch/push）也必须使用该解析结果选择凭据。
3) 命令：accounts_list()、accounts_set_default(account_id)、
   repo_bind_account(repo_id, account_id|None)、accounts_switch(account_id)（影响全局默认）。
4) Token 失效检测与恢复：
   - 任何 API 调用返回 401 → 标记该账号 auth_state = Expired → 推送事件 auth:expired →
     前端顶部横幅提示"账号 X 的登录已过期"，提供"重新登录"按钮（保留仓库绑定）；
   - **禁止无限重试**：标记过期后，该账号的请求直接失败并给出引导，直到重新登录。
5) 前端：设置 → 账号页（列表、设为默认、重新登录、退出登录、查看 scope、查看关联仓库数）；
   顶部账号切换器（切换后刷新所有 GitHub 数据 query）。
6) 隐私：账号列表与头像 URL 存本地；不发送任何账号信息到第三方（除 GitHub API 本身）。
验收：单测覆盖账号解析优先级（4 种组合）；模拟 401 后断言只提示一次且不重试；
   手工验证两个账号并存且按仓库绑定生效（需人类配合提供第二个账号或 PAT）。
```

#### T4.5 仓库浏览、搜索、Fork、Star 与克隆联动

`依赖：T4.4` ｜ `估时：5d` ｜ `审批点：无`

```text
【T4.5 仓库服务】
1) 命令：
   gh_repo_list(account_id, kind: Mine|Starred|Org(org)|Contributed, sort, page_size, cursor)
   gh_repo_search(query, in: Name|Description|Readme|Topic, sort, order, language?, stars_min?, cursor)
   gh_repo_detail(ref) —— 含 default_branch、语言构成、贡献者、分支/标签列表（分页）、
     README（raw + 渲染所需的 markdown）、license、topics、stars/forks/watchers、最近提交时间
   gh_repo_fork(ref, {organization, name, default_branch_only})
   gh_repo_star(ref, starred: bool) / gh_repo_watch(ref, level) / gh_repo_archive(ref, archived: bool)
   gh_repo_clone_plan(ref) -> { recommended_url_https, recommended_url_ssh, default_dir_name, size_hint }
2) Fork 后的联动（关键体验）：Fork 成功后自动：
   - 提示用户是否立即克隆；
   - 若克隆，自动识别 upstream（原始仓库）并写入 remote `upstream`（这是开源贡献标准流程，必须做对）；
   - 在 UI 中展示"当前分支 → upstream/main"的贡献关系提示。
3) 权限处理：Star/Watch/Archive/Fork 需要对应权限；403 时返回 Forbidden{required_scope} 并提示
   "需要重新授权以获取 xxx 权限"（引导重新登录并增加 scope），而非笼统报错。
4) 分页：统一使用 Link 头的下一页 URL 或 cursor；前端无限滚动 + 骨架屏 + 错误重试（保留已加载项）。
5) 缓存：仓库详情与列表使用 ETag 缓存（T4.2）；提供"强制刷新"按钮（带 force 参数跳过缓存）。
验收：wiremock 覆盖分页（多页 Link 头）、403 权限、Fork 后 upstream 配置；
   手工用真实账号验证"Fork → Clone → remote -v 显示 origin 与 upstream"（需人类配合）。
```

#### T4.6 README 与 Markdown 安全渲染

`依赖：T4.5` ｜ `估时：3d` ｜ `审批点：无`

```text
【T4.6 Markdown 渲染】
1) 前端使用 markdown-it（或 remark）+ DOMPurify：
   - 支持 GFM（表格、任务列表、删除线、自动链接、代码块语言高亮通过 shiki 或 lowlight，按需加载语言）；
   - 渲染前清理：禁用 HTML 中的 script/style/iframe/form/object/embed，禁用 on* 事件属性，
     禁用 javascript: / data: 协议（data:image 白名单允许）；
   - 图片与链接：默认**不自动加载远程图片**（隐私与安全），而是显示占位并提示"点击加载图片"，
     用户可设置"始终加载来自 <host> 的图片"白名单。
2) URL 打开：所有外部链接必须通过 openExternal 命令（后端校验 scheme 仅 http/https/mailto）打开系统浏览器，
   禁止在应用内导航。
3) CSP：在 tauri.conf.json 配置严格 CSP（default-src 'self'; img-src 'self' data: asset:;
   style-src 'self' 'unsafe-inline'; script-src 'self'）并根据实际需要最小化放宽；
   在回报中说明每条放宽的理由。
4) 相对链接与图片：README 中的相对路径需解析为仓库文件（通过 API 获取 raw 内容或转换为 blob 链接）。
5) 提供 Markdown 渲染的通用组件，供 PR 描述、Issue 描述、评论复用。
验收（强制安全测试）：构造包含 <img onerror>, <script>, javascript: 链接, <iframe>,
   CSS expression, SVG onload 的恶意 Markdown，断言全部被清理且不执行（E2E 中断言 window.__errs 为空，
   且断言页面未出现预期外的 DOM 元素）。
```

#### T4.7 PR 列表、详情与内联评论

`依赖：T4.5, T4.6` ｜ `估时：8d` ｜ `审批点：**需要你确认是否用真实仓库做 PR 联调**`

```text
【T4.7 PR 核心】
后端 crates/services/pulls（GitHub 用 GraphQL 优先，失败回退 REST）：
1) gh_pr_list(account_id, ref, filter{state, author, assignee, reviewer, labels, search, base, head})
   → 游标分页；返回列表项含 checks 汇总状态、review 决策、是否 draft、可合并性、更新时间。
2) gh_pr_detail(ref, number) → PullRequestDetail：
   { meta（标题/正文/状态/draft/base/head/作者/标签/指派/审查者/里程碑/是否可合并/冲突原因）、
     files: Vec<FileChangeWithStat>、checks: Vec<CheckRun>、reviews: Vec<Review>、
     review_comments: Vec<ReviewComment>（含 path、line、side、in_reply_to）、
     timeline: Vec<TimelineEvent>（评论、提交、标签变更、指派、review、check 变化，按时间合并排序） }
3) 文件 diff：使用 GitHub 的 diff 端点（Accept: application/vnd.github.v3.diff）或逐文件的 patch 字段，
   复用 T1.5 的 Diff 查看器渲染；大 PR（> 300 文件）必须支持按目录折叠与懒加载 diff。
4) 评论：
   - gh_pr_comment_issue（普通评论）；
   - gh_pr_review_comment_create(ref, number, {body, path, line, side, start_line?, start_side?, commit_id, in_reply_to?})
     必须正确处理"多行评论"（start_line）与"过时评论"（commit_id 不在最新 head 时提示）；
   - gh_pr_review_comment_list（按 path 分组，支持回复线程折叠）。
5) Review 草稿与提交（为 T4.8 打基础）：本任务先实现"创建单条评论"，草稿队列在 T4.8。
6) 错误处理：评论行号越界（422）、PR 已关闭、无权限，均映射为结构化错误与可读提示。
验收：wiremock 契约测试覆盖列表筛选、详情、行级评论（含多行）、行号越界；
   手工在真实仓库上完成"创建 PR → 查看 diff → 添加行级评论"（需人类提供测试仓库）。
```

#### T4.8 Review 提交与 PR 合并

`依赖：T4.7` ｜ `估时：5d` ｜ `审批点：**合并 PR 属于不可逆操作，首次需人类在旁确认**`

```text
【T4.8 Review 与合并】
1) Review 草稿队列（本地）：
   - 用户可在 diff 上累积多条评论（pending），在本地状态中管理（Zustand + 持久化到 settings 以防崩溃丢失）；
   - 提交时一次性提交（GraphQL addPullRequestReview 支持 comments 数组，可带 event）；
   - 提交后清空草稿并刷新详情；失败时保留草稿并明确报错。
2) 提交 Review：event ∈ {APPROVE, REQUEST_CHANGES, COMMENT}，可带 summary body。
   - REQUEST_CHANGES 前提示"该操作会阻止 PR 合并"；
   - 自己给自���的 PR 提交 APPROVE 会被 GitHub 拒绝 → 提前禁用并说明原因。
3) 合并：
   - gh_pr_merge(ref, number, spec{method: Merge|Squash|Rebase, commit_title, commit_message,
     sha (乐观锁), delete_branch})；
   - 合并前必须展示"合并条件检查"：是否冲突（mergeable/mergeable_state）、
     必需 review 是否满足、必需 check 是否通过、是否 draft；任一不满足时禁用合并按钮并说明原因（不隐藏按钮，便于学习）；
   - 传递 head sha 做乐观锁，避免基于过期信息合并；409/405 时提示刷新；
   - 合并成功后可选删除源分支（需确认），并提示用户本地分支的处理方式（同步/删除/保留）。
4) 关闭/重开 PR、转为 Draft/Ready、修改标题与正文、增删标签与指派（本任务一并实现，量小）。
5) 全部写操作写审计（记录 PR 编号与动作）。
验收：wiremock 覆盖合并条件不满足的四类原因、乐观锁失败（409）、草稿队列提交与失败保留；
   E2E 覆盖"累积 3 条评论 → 提交 REQUEST_CHANGES → 草稿清空 → 详情刷新"。
⚠️ 首次真实合并 PR 时请在回报中请求人类确认（避免误操作）。
```

#### T4.9 Issue、标签与里程碑

`依赖：T4.5, T4.6` ｜ `估时：4d` ｜ `审批点：无`

```text
【T4.9 Issue 管理】
1) gh_issue_list(account_id, ref, filter{state, author, assignee, labels, milestone, search, sort, direction})
   → 游标分页；支持按 "assigned to me" / "created by me" / "mentioned me" 快捷筛选。
2) gh_issue_detail(ref, number) → { meta, body(markdown), timeline, comments, assignees, labels, milestone,
   linked_prs, is_locked }。
3) 写操作：
   gh_issue_create(ref, {title, body, labels, assignees, milestone}) —— **前置**：
     调用 gh_issue_template(ref) 获取仓库的 Issue 模板（.github/ISSUE_TEMPLATE/*.yml）并让用户选择模板、
     按模板字段渲染成表单填写（这是差异化体验点，务必实现）；
   gh_issue_update(ref, number, {title, body, state, state_reason, labels, assignees, milestone})
   gh_issue_comment_create / update / delete
   gh_label_list / create / update / delete（含颜色与描述）
   gh_milestone_list / create / update / delete / close
4) 体验要求：
   - Issue 列表支持"看板/列表"两种视图切换（看板按 label 或 milestone 分列，纯本地分组，无需项目功能）；
   - 创建 Issue 时支持粘贴图片？——**不做上传**（需要额外 scope 与复杂度），改为提示用户使用 GitHub 网页；
     在文档中明确此限制。
   - Markdown 编辑器提供基础工具栏（加粗、斜体、代码、链接、列表、引用、任务列表）与实时预览（复用 T4.6）。
验收：wiremock 覆盖 Issue 模板获取与表单渲染、label/milestone CRUD；
   E2E 覆盖"选择模板 → 填写 → 创建 Issue（mock）"。
```

#### T4.10 Actions 状态、运行记录与日志

`依赖：T4.5` ｜ `估时：5d` ｜ `审批点：无`

```text
【T4.10 Actions】
1) gh_workflow_list(ref) → 工作流列表（名称、路径、状态）。
2) gh_run_list(ref, filter{branch, status, conclusion, workflow, actor, event}) → 游标分页，
   列表项含：运行编号、标题、分支、事件、状态与结论、耗时、触发者、开始时间。
3) gh_run_detail(ref, run_id) → { meta, jobs: Vec<{name, status, conclusion, steps: Vec<{name, status, conclusion, number, started_at, completed_at}>}> }。
4) 日志：
   - gh_run_log_download(ref, run_id) 返回重定向后的 zip 下载地址（GitHub 的日志是 zip）；
   - 后端下载 zip → 解压到临时目录 → 提供 gh_job_log(job_id) 按需读取单个 job 的日志文本；
   - 大日志（> 5MB）必须流式读取并分页返回，禁止一次性读入内存；
   - 前端日志视图：等宽字体、自动滚屏（可关）、按 ANSI 颜色解析着色（GitHub 日志含 ANSI 转义）、
     关键词搜索与高亮、"跳到失败步骤"快捷按钮、复制全文/复制选区。
   - 日志中可能包含敏感信息 → 展示前不做处理（这是用户自己的日志），但**不得写入应用日志文件**。
5) 操作：gh_run_rerun(run_id, {failed_only})、gh_run_cancel(run_id)、gh_run_rerun_job(job_id)、
   gh_workflow_dispatch(workflow_id, {ref, inputs})（需读取 workflow 的 inputs 定义并渲染表单）。
6) 实时性：运行中的 run 每 5 秒轮询一次（仅在 Actions 页可见时轮询；页面隐藏即停止——这是硬性要求，避免浪费额度）。
7) 在仓库页与 PR 页展示 checks 汇总徽标，点击跳转 Actions 详情。
验收：wiremock 覆盖 zip 日志下载与解压、大日志分页读取、rerun/cancel；
   断言"页面不可见时停止轮询"（写单测断言 timer 被清理）。
```

#### T4.11 限流降级、离线行为与 Dashboard 聚合

`依赖：T4.5–T4.10` ｜ `估时：3d` ｜ `审批点：无`

```text
【T4.11 降级与 Dashboard】
1) 全局限流与离线状态：
   - 顶部状态栏显示当前账号的 API 额度（remaining/limit）与重置倒计时（仅在消耗较多或接近耗尽时强调提醒）；
   - 额度耗尽时所有 GitHub 数据请求走缓存，并在页面顶部显示"已达 API 限额，显示缓存数据（将于 HH:MM 恢复）"横幅；
   - 网络不可达时进入离线模式：GitHub 页面展示缓存的最后一份数据 + 明确的离线提示；
     本地 Git 功能必须 100% 不受影响（写自动化测试断言：断网时打开仓库/提交/push 到本地 bare 远端仍可用）。
2) 缓存策略细化：
   - 列表类：TTL 2 分钟；详情类：TTL 2 分钟；README：TTL 1 小时；分支/标签：TTL 5 分钟；
   - 用户操作导致的变更立即 invalidate 相关 key（例如评论后 invalidate 详情与时间线）。
3) Dashboard（仪表盘）：
   - 汇总卡片：已打开仓库（分支/状态/是否有冲突）、我的 PR（待我审查 / 我创建的）、
     我参与的 Issue、最近的 Actions 失败（仅限当前绑定账号可见的仓库）；
   - 每个卡片可点击下钻；支持"仅显示我关注/星标的仓库"过滤（避免数据量过大导致 API 消耗）；
   - **必须限制请求规模**：Dashboard 只对"用户显式 pin 的仓库（默认最多 5 个）"拉取数据，
     其余不自动拉取（在 UI 中说明"以节省 API 额度"，避免一打开应用就打爆限额）。
4) 提供"统计与额度"设置页：显示本会话的 API 调用次数、缓存命中率、剩余额度。
验收：断言离线模式下本地 Git 功能全部可用（自动化）；
   断言 Dashboard 只对 pin 的仓库发请求（wiremock 断言请求集合）。
```

#### T4.12 GitHub 契约测试与 M4 验收

`依赖：T4.1–T4.11` ｜ `估时：4d` ｜ `审批点：无`

```text
【T4.12 M4 契约测试与验收】
1) 使用 wiremock 建立完整的 GitHub Mock 服务，覆盖：
   /user、/repos、/search/repositories、/repos/{o}/{r}、/branches、/tags、/pulls（列表与详情）、
   /pulls/{n}/files、/pulls/{n}/reviews、/pulls/{n}/comments、/issues、/issues/{n}/comments、
   /actions/runs、/actions/runs/{id}/jobs、日志 zip、/releases、/notifications、GraphQL 端点。
   契约测试要求：断言请求方法/路径/关键查询参数/请求体字段；断言响应解析结果；
   断言错误码映射（401/403/404/409/422/429/5xx 各至少一例）。
   覆盖路径比例 ≥ 90%（在回报中给出分子/分母）。
2) 提供一个"可选的真实仓库冒烟测试"：
   由环境变量 GITHUB_TOKEN 与 TEST_REPO 控制，默认在 CI 中跳过；
   覆盖：读仓库、读 PR 列表、读 Issue 列表、读 Actions 列表（**只读，不做任何写操作**）。
3) 前端单测：PR 筛选映射、评论草稿队列、日志 ANSI 解析、缓存失效逻辑。
4) E2E（mock 模式）：登录（mock）→ 仓库列表 → 打开 PR → 查看 diff → 添加评论 → 提交 review。
5) 编写 docs/acceptance/M4.md，逐条对照 PLAN 的 M4 验收标准给出证据；
   修复全部 P0/P1 缺陷。
验收：契约测试覆盖 ≥ 90%；CI 全绿；验收报告完整；确认无任何写操作在真实仓库上被执行（除人类明确授权的联调）。
```

### 4.6 M5 终端 / 诊断 / 编辑器（T5.1–T5.10）

#### T5.1 portable-pty 三平台 Spike

`依赖：T0.2` ｜ `估时：2d` ｜ `审批点：**若 Spike 失败需要你决定降级方案**`

```text
【T5.1 PTY Spike（先验证再开发）】
目标：在写正式代码前确认 portable-pty 在三平台的可用性与限制。**不要开始做完整终端 UI。**
1) 写一个最小 Rust 二进制（examples/pty_spike.rs）：
   - 在 Windows 上创建 ConPTY 会话（powershell.exe 或 pwsh 或 cmd，按可用性选择）；
   - 在 macOS/Linux 上创建 PTY 会话（$SHELL 或 /bin/bash）；
   - 执行命令、读取输出、写入输入、resize；
   - 测试：中文输出、emoji、ANSI 颜色、宽字符（CJK）对齐、Ctrl+C 中断、
     交互式提示（如 git config --global --edit 会等待输入）、进程退出。
2) 在 app 中接入一个最小 Tauri 命令 + 事件通道，验证：
   - 二进制数据经 IPC 的传输方式（建议 Vec<u8> 序列化或 base64，需说明选择与性能影响）；
   - 每秒 5000 行输出的吞吐量（压测：执行一个快速打印 10 万行的命令，测量阻塞与丢失情况）。
3) 记录结论到 docs/PTY-SPIKE.md：
   - 三平台是否可用；已知问题；Windows 版本最低要求（ConPTY 需 Win10 1809+）；
   - 吞吐量数据；IPC 传输方案结论；resize 行为。
4) 若某平台不可用或问题严重，**立即停下来**并给出降级方案（非交互日志式终端）供人类决策。
验收：docs/PTY-SPIKE.md 有明确结论与数据；spike 代码可运行（作为 example 保留或删除，需说明）。
⚠️ 若 Spike 失败，请求人类决策降级方案后再继续。
```

#### T5.2 内嵌终端组件

`依赖：T5.1` ｜ `估时：4d` ｜ `审批点：无`

```text
【T5.2 终端实现】
后端 crates/services/terminal：
  term_create(repo_id, {shell?, cwd?, cols, rows, env?}) -> { termId }
  term_write(termId, data: Vec<u8>) / term_resize(termId, cols, rows) / term_close(termId) / term_list()
  事件：term:output { termId, data: Vec<u8> }、term:exit { termId, code }
  要求：每个会话独立线程；写操作加缓冲（16ms 合并）以减少 IPC 次数；
  会话退出后保留最后 1000 行输出供查看；关闭仓库时若仍有活跃终端要提示。
前端 src/features/terminal：
1) xterm.js + fit-addon + search-addon + web-links-addon；
   WebGL renderer（失败自动降级 canvas，需检测并记录）。
2) 多标签：新建/关闭/重命名标签、拖拽排序、标签标题显示当前运行命令（可关闭）；
   "+" 按钮提供 shell 选择（默认 / PowerShell / Git Bash / cmd / zsh / bash）。
3) 主题跟随应用主题（提供两套 xterm theme 映射到 CSS 变量）；字体大小与行高可在设置调整（默认 13px/1.35）。
4) 复制粘贴：Ctrl/Cmd+C（有选区时复制，无选区时发送 SIGINT）、Ctrl/Cmd+V、右键菜单（复制/粘贴/全选/清屏/搜索）；
   鼠标选择即复制（可选设置）。
5) 搜索：Ctrl/Cmd+F 打开 xterm search-addon，支持正则与大小写选项。
6) 链接识别：点击 http(s) 链接经 openExternal 打开；识别本地文件路径（含 file:line 形式）→ 在编辑器中打开。
7) 与仓库联动：终端默认 cwd 为仓库根；切换仓库时若终端仍在其他仓库，给出提示（不允许静默切错目录）；
   终端中执行 git 命令后触发 repo:changed 相当于外部修改，状态自动刷新。
8) 会话持久化：应用重启后不恢复会话（明确说明），但提供"在新标签中恢复上次命令"的便捷（历史记录）。
验收：三平台手工验证交互式命令（如 top / vim 打开退出）；
   E2E 覆盖 echo 输出、resize、复制粘贴、搜索；断言 window.__errs 为空。
```

#### T5.3 终端危险命令拦截提示与安全衔接

`依赖：T5.2, T1.9` ｜ `估时：3d` ｜ `审批点：**需要你确认拦截强度（提示 or 阻断）**`

```text
【T5.3 终端安全衔接】
背景：终端可以绕过 UI 的安全层直接执行危险 git 命令，这会让"快照与回滚"失去保障。需要一个平衡方案。
1) 命令识别：解析用户输入的完整命令行（在 Enter 时读取当前行缓冲），做轻量词法分析
   （不需要完整 shell 语义，识别 `git` 子命令与关键参数即可）。识别目标包括：
   reset --hard、clean -fd/-fdx、checkout -f/--force、push --force（裸 force）、branch -D、
   stash drop/clear、rebase、filter-branch、gc --prune=now、update-ref -d、reflog expire。
2) 行为（分级，可在设置中调整）：
   - **提示级（默认）**：在终端上方显示一条非阻塞提示条："检测到危险操作 git reset --hard，
     建议使用图形界面的安全操作（自动快照），或继续在终端执行。点击这里查看图形入口"；
     2 秒后可自动消失，不阻断输入。
   - **确认级（可选开启）**：需要用户确认后才把命令写入 pty（默认关闭，避免烦人）。
   - **始终记录级（强制）**：无论用户选择哪种，命令执行后自动在 operation_records 中写一条记录
     （来源标记为 terminal），并尝试在命令执行后立即创建补偿快照（因为无法预知结果）。
3) 实现细节：
   - 拦截不修改用户输入内容（发送给 pty 的字节必须与用户输入完全一致，除确认级外）；
   - 识别失败时不得阻断（宁可漏报不可误伤）；
   - 提供设置项：终端安全提示开关、级别、是否自动补偿快照。
4) 需要在文档中明确说明**能力边界**：终端中的任何操作都无法保证可回滚（例如在终端里执行
   外部工具直接改文件），UI 中不得宣称"终端操作也可回滚"。
验收：单测覆盖命令识别的 15 个正例与 15 个负例（不得误报 git commit、git push 普通推送、
   git checkout <branch> 等安全命令）；E2E 覆盖提示条出现且不阻断输入。
⚠️ 完成后请求人类确认默认拦截级别（提示级 / 确认级）与是否默认开启自动补偿快照。
```

#### T5.4 命令解释器（本地知识库）

`依赖：T5.2` ｜ `估时：3d` ｜ `审批点：无`

```text
【T5.4 命令解释】
目标：用户输入 git 命令时，用人话解释它做什么、有什么风险、等价图形入口在哪。**纯本地规则，禁止任何 AI 调用。**
1) 数据文件：assets/git-explains.yaml，结构：
   command: reset
   subcommands:
     - name: "--hard"
       summary: "把分支、暂存区和工作区全部重置到指定提交，未提交的修改会被永久丢弃。"
       risk: dangerous
       equivalent_ui: "工作区 → 危险操作 → 重置（自动快照）"
       docs_url: "https://git-scm.com/docs/git-reset"
   global_flags:
     - name: "--no-verify"
       summary: "跳过 Git 钩子检查（如 pre-commit 校验）。"
       risk: caution
2) 覆盖范围：≥ 60 个常用命令/子命令/关键参数（init、clone、status、add、restore、commit、branch、
   checkout、switch、merge、rebase、cherry-pick、revert、reset、reflog、stash、tag、fetch、pull、push、
   remote、submodule、lfs、worktree、bisect、blame、log、diff、show、config、gc、fsck 等）。
   内容需准确，禁止编造不存在的参数。
3) 解析：轻量解析用户输入的命令行 → 匹配解释条目 → 生成解释卡片。
4) UI：
   - 在终端中输入命令并按 Enter 前（或按 Ctrl+/ 触发"解释这条命令"）显示解释卡片（非模态浮层）；
   - 卡片内容：人话解释、风险等级（安全/注意/危险，用图标 + 文字）、等价图形入口按钮（若存在）、
     官方文档链接（openExternal）、"插入到当前行"辅助按钮；
   - 提供独立的"命令字典"页面（可搜索全部条目，按风险等级与分类筛选）。
5) 版本提示：文档中说明解释内容基于本地知识库快照，可能滞后于最新 git 版本；提供"反馈/纠正"入口（跳 GitHub Issue）。
验收：单测覆盖解析与匹配（≥ 30 条）；抽查 15 条解释内容的准确性（回报中列出抽查清单与依据）；
   断言代码中不存在任何网络调用（grep 断言 + 依赖检查）。
```

#### T5.5 诊断规则引擎

`依赖：T1.1, T1.9` ｜ `估时：4d` ｜ `审批点：无`

```text
【T5.5 诊断引擎】
crates/diagnostics：
1) 规则文件格式（assets/diagnostics/*.yaml）：
   id: push-non-fast-forward
   stage: [write, network]
   match:
     all_of:
       - contains: "! [rejected]"
       - regex: "non-fast-forward|fetch first"
     none_of: []
   confidence: 0.95
   title_key: diag.push_rejected.title
   explanation_key: diag.push_rejected.explanation
   causes: [diag.push_rejected.cause1, diag.push_rejected.cause2]
   fixes:
     - id: fetch-then-retry
       label_key: diag.push_rejected.fix_fetch
       action: { kind: command, command: git_fetch, args: {} }
     - id: use-force-with-lease
       label_key: diag.push_rejected.fix_lease
       action: { kind: dangerous, command: git_push, args: { force_with_lease: true } }
2) 引擎：diagnose(stderr: &str, ctx: &DiagContext) -> DiagnosticReport
   { primary: Option<Diagnostic>, alternatives: Vec<Diagnostic>, raw_summary: String }
   规则按 confidence 排序；context 提供消歧信息（op_type、是否有 upstream、是否 detached、是否浅克隆）。
3) 规则加载：编译期内嵌（include_str!）+ 支持运行时覆盖目录（app_config_dir()/diagnostics/），
   便于不发版修规则；加载时做 schema 校验与冲突检测（同 id 重复 → 警告）。
4) 覆盖 ≥ 50 条规则，必须包含 docs/PLAN.md M5 验收标准列出的 10 类错误，以及其他常见错误
   （权限、磁盘、路径过长、文件名非法、锁文件存在 index.lock、SSL 证书、代理、LFS 未安装、
   子模块、换行符、大文件警告、钩子失败、分支不存在、远端不存在等）。
5) 动作安全：kind = dangerous 的动作在前端必须走统一的 DangerousActionDialog（含计划预览与快照），
   绝不允许诊断卡片一键直接执行危险操作。
6) i18n：全部规则文案走 i18n key（中英文）；规则文件本身不含文案，只含 key（保证可翻译）。
验收：表驱动单测——58 条规则各至少一个真实 stderr 样本（fixture）断言命中；
   误报测试——20 个无关 stderr 样本断言 primary 为 None；
   消歧测试——同一 stderr 在不同 context 下给出不同 primary。
```

#### T5.6 错误诊断展示与修复动作执行

`依赖：T5.5` ｜ `估时：3d` ｜ `审批点：无`

```text
【T5.6 诊断 UI 与动作执行】
1) DiagnosticsCard 组件：
   - 标题（人话）+ 原因列表（可展开）+ 置信度可视化（高/中/低，用文字 + 图标）；
   - 修复动作按钮（按 kind 分组：安全动作直接执行 / 危险动作打开 DangerousActionDialog）；
   - "查看原始错误"可折叠区（等宽字体，支持复制）；
   - "复制这条错误到搜索"按钮（打开浏览器搜索，需用户点击）；
   - "这个诊断不对"反馈按钮（记录到本地并生成预填的 GitHub Issue 链接）。
2) 集成点（所有可能产生 git 错误的地方都必须展示诊断）：
   - 提交失败、同步失败、分支操作失败、reset/rebase 失败、终端中的失败命令（在提示条中提供"诊断此错误"）；
   - 错误 Toast 中的"详情"应展示诊断卡片，而不仅是原始字符串。
3) 修复动作执行：
   - action.kind = command：直接调用对应 Tauri command 并展示执行结果（成功 → 刷新状态；失败 → 递归诊断）；
   - action.kind = dangerous：打开 DangerousActionDialog（展示计划 + 快照说明 + 确认）；
   - action.kind = guide：打开文档（内置的 troubleshooting 页面或 openExternal）。
4) 提供"诊断历史"（最近 20 条错误与诊断结果）在设置页，便于排查重复问题。
验收：E2E 覆盖 ① 制造 non-fast-forward 错误 → 诊断卡片出现 → 点击"先 fetch 再重试" → 成功；
   ② 制造认证失败 → 诊断卡片出现 → 点击"检查 SSH 配置" → 跳转设置页；
   ③ 点击"这个诊断不对"生成反馈记录。
```

#### T5.7 文件树与 Monaco 编辑器

`依赖：T2.2, T4.6` ｜ `估时：6d` ｜ `审批点：无`

```text
【T5.7 文件树与编辑器】
后端 crates/services/workspace：
  fs_tree(repo_id, path, {show_ignored, show_hidden}) -> Vec<TreeNode>（懒加载一层）
  fs_read(repo_id, path) -> { content, encoding, eol: Lf|Crlf|Cr, has_bom, size, is_binary, truncated }
  fs_write(repo_id, path, { content, preserve_eol: bool, preserve_bom: bool }) -> { written_bytes }
  fs_create / fs_rename / fs_delete（需确认；删除走"移入回收站"而非直接删除，明确说明）
  fs_watch(repo_id, path)（复用 T1.10 监听，前端订阅）
  安全：所有路径必须 canonicalize 后校验在仓库根内；拒绝符号链接逃逸（解析后二次校验）；
  限制单文件读取 ≤ 5MB（超出提示用外部编辑器打开）。
前端 src/features/editor：
1) 文件树：
   - 懒加载、目录折叠、图标按扩展名（自绘或开源图标集，需登记许可）；
   - 显示 Git 状态标记（已修改/新增/冲突/忽略），与工作区状态联动；
   - 操作：新建文件/文件夹、重命名（F2）、删除（Delete + 确认）、在文件管理器显示、复制路径、
     在终端打开此目录；
   - 过滤器：仅显示已修改文件（适合 diff 审查场景）、隐藏忽略文件。
2) Monaco 编辑器：
   - 多标签、脏标记、关闭时提示保存；
   - 按扩展名自动识别语言 + 手动切换 + 语言列表按需加载（不得打包全部语言）；
   - 基础编辑能力：查找替换、多光标、代码折叠、缩进、括号匹配、大小写转换；
   - **不做** LSP/智能补全（属"不做清单"），但可提供"基础单词补全"（编辑器内置）；
   - 外部变更检测：文件在磁盘上被修改时（非本应用写入），提示三选一：
     [重新加载（丢弃我的编辑）] [保留我的编辑] [并排对比]；绝不静默覆盖；
   - 保存时保持原有换行符与 BOM 设置（读取时的元信息）；保存后刷新 Git 状态。
3) 与 Git 集成：
   - 编辑器 gutter 可显示变更行标记（新增/修改/删除），点击可查看该行的 git blame（T5.8 实现）；
   - 提供"与 HEAD 对比"按钮，打开 diff 视图（左侧 HEAD 版本，右侧当前文件）。
验收：E2E 覆盖 打开文件 → 编辑 → 保存 → 状态栏变更计数；外部修改 → 提示三选一；
   路径逃逸测试（构造 ../ 与符号链接，断言被拒绝）；1000 个文件的目录树展开流畅。
```

#### T5.8 文件历史与 Blame

`依赖：T5.7, T2.3` ｜ `估时：4d` ｜ `审批点：无`

```text
【T5.8 文件历史与 Blame】
后端：
1) git_blame(repo_id, path, {range?, ignore_whitespace, detect_moves}) -> Vec<BlameLine>
   { line_no, oid, short_oid, author, author_mail, time, summary, is_uncommitted }
   实现方式：git blame --line-porcelain（注意性能：大文件先限制范围，或使用 --incremental）。
2) git_file_history(repo_id, path, {follow, limit, cursor}) -> Page<Commit>（--follow 跟随重命名）。
3) git_file_at(repo_id, path, rev) -> content（用于与历史版本对比）。
前端：
1) Blame 视图（侧栏形式，在编辑器左侧）：
   - 按提交分组显示作者与时间（连续同一提交的行合并为一块，视觉上用左侧色条区分）；
   - hover 显示提交摘要卡片；点击跳转该提交详情；
   - "未提交"的行（作者是本机且未 commit）单独标记；
   - 提供"复制该行 git blame 命令"便于学习。
2) 文件历史面板：时间线列表（含变更类型 A/M/D/R）、点击查看该次提交对该文件的 diff、
   支持与任意历史版本对比（在 diff 视图中打开）。
3) 性能：对 5000 行文件的 blame 需 < 2s（超时则降级为"仅显示当前可见区域范围的 blame"）。
验收：单测覆盖 line-porcelain 解析（含未提交行、重命名检测）；E2E 覆盖 blame 显示与跳转提交。
```

#### T5.9 快捷键系统与命令面板

`依赖：T5.7` ｜ `估时：3d` ｜ `审批点：无`

```text
【T5.9 快捷键与命令面板】
1) 快捷键框架：
   - 定义 CommandRegistry：每个命令有 id、标题（i18n）、分类、默认快捷键、可用条件（when 表达式）；
   - 用户可自定义快捷键并检测冲突（同一 when 上下文中重复 → 报错并阻止保存）；
   - 预设方案：Default（ForgeDesk 自有，不得照抄 VS Code 组合键布局作为"预设"名称；
     可提供 "IDE 风格" 作为可选预设，但需注明灵感来源并确保不是逐条复制）；
   - 显示当前平台修饰键（Cmd on macOS / Ctrl on Windows/Linux）；
   - 在设置页提供搜索、按分类分组、冲突高亮、"恢复默认"。
2) 命令面板（Ctrl/Cmd+Shift+P）：
   - 模糊搜索全部已注册命令（含插件命令，M6 接入）；
   - 显示快捷键提示、可用条件不满足时灰显并说明原因；
   - 支持"最近使用"与"常用"分组；
   - 键盘完全可用（↑↓ 选择、Enter 执行、Esc 关闭）。
3) 必须注册的核心命令（≥ 40 个）：仓库切换、刷新、暂存全部、提交、amend、push/pull/fetch、
   分支切换/新建、打开历史、打开终端、打开设置、切换主题、切换面板、打开命令面板、
   在编辑器打开文件、放弃修改、查看快照、回滚、打开冲突向导、搜索提交等。
4) 帮助页：展示全部快捷键（可导出为 Markdown）。
验收：单测覆盖冲突检测与 when 条件求值；E2E 覆盖自定义快捷键 → 生效 → 冲突时被拒绝；
   命令面板键盘操作全流程。
```

#### T5.10 布局系统与 M5 验收

`依赖：T5.1–T5.9` ｜ `估时：4d` ｜ `审批点：无`

```text
【T5.10 布局与 M5 验收】
1) 布局系统：
   - 可拖拽的 SplitPane（已有基础组件）+ 面板显隐控制（视图菜单）；
   - 布局预设：Default / 审查模式（大 diff 区）/ 终端模式（大终端区）/ 编辑器模式；
   - 布局持久化为 JSON 存 settings，支持导入导出；
   - 窗口大小变化时布局自适应；多窗口（M9 预留）不实现。
2) 恢复默认布局与"布局损坏"容错（非法 JSON → 回退默认并提示）。
3) M5 验收：
   - E2E：终端可用（echo/中文/emoji）、危险命令提示、诊断卡片（10 类错误中的至少 4 类走 E2E，
     其余走单测）、编辑器外部变更三选一、快捷键自定义、布局保存与恢复；
   - 断言 window.__errs 为空；
   - 编写 docs/acceptance/M5.md 逐条对照 PLAN M5 验收标准；
   - 修复全部 P0/P1 缺陷。
验收：CI 全绿；三平台手工验收记录（终端部分必须三平台各验一次）。
```

### 4.7 M6 插件 / 主题 / 多平台适配（T6.1–T6.10）

#### T6.1 插件运行时（WASI 沙箱）

`依赖：T0.2, T4.1` ｜ `估时：5d` ｜ `审批点：**需要你确认 wasmtime 引入的体积影响是否可接受**`

```text
【T6.1 插件运行时】
crates/plugin-host：
1) 清单规范 schema（plugin.json，用 serde + schemars 生成 JSON Schema 供文档使用）：
   { id, name, version, apiVersion: "0.1", author, license, description, homepage?,
     main: "plugin.wasm", permissions: [...], contributes: { commands: [...], panels: [...] } }
   校验规则：id 为反向域名风格且唯一；apiVersion 主版本不匹配则拒绝加载；
   permissions 只能取自白名单（fs:read / fs:write / git:read / git:write / net:github / ui:panel /
   ui:command / ui:toast / settings:read / settings:write）；manifest 中声明的 permissions
   与实际调用不符时以用户授权为准（未授权调用直接失败）。
2) 运行时：wasmtime，WASI 上下文按权限最小构造：
   - 默认**不**预打开任何目录；fs:read / fs:write 授权时也只预打开"用户显式选择的仓库目录"（通过 . 映射）；
   - 不继承环境变量（除 TZ 等无害项，需说明）；
   - 网络：WASI 无网络能力；如需 net:*，通过宿主提供的 host function（走 T4.2 的 HttpClient，
     并强制使用用户配置的代理与限流）实现，绝不放开任意 socket。
3) 生命周期：load(manifest) → activate(ctx) →（事件驱动）→ deactivate()；
   超时保护：单次 host function 调用 5s、单次插件调用（命令执行）30s；
   内存上限默认 64MB（wasmtime StoreLimits）；超限或 panic → 捕获并停用插件 + 记录日志 +
   通知前端（插件崩溃绝不影响宿主）。
4) 插件存储：app_data_dir()/plugins/<plugin_id>/（每个插件独立目录），
   记录已安装插件元数据到 settings 或新建 plugin_registry 表（需迁移脚本）；
   支持"开发者模式"从任意本地目录加载（需在设置中显式开启，并在 UI 中标注为不安全）。
5) 单元与集成测试：
   - 构造 3 个测试插件（正常、越权访问、死循环/内存爆炸），断言：正常插件可用、
     越权调用返回结构化错误且宿主不崩溃、异常插件被停用且宿主可继续工作。
验收：测试全绿；在回报中给出 wasmtime feature 裁剪后的二进制体积增量与编译时间增量。
⚠️ 若体积增量超过 5MB 或编译时间显著增加，请请求人类决策是否将插件系统改为可选 feature。
```

#### T6.2 宿主 API 与权限校验

`依赖：T6.1` ｜ `估时：4d` ｜ `审批点：无`

```text
【T6.2 宿主 API】
1) 定义插件可用的宿主函数（全部通过 wasmtime host function 注册，每个函数入口先校验权限）：
   ctx.log(level, message)
   ctx.get_repo_info() -> { path, name, current_branch, is_dirty }
   ctx.get_status(filter?) -> 简化状态（仅路径与状态码，不含内容）
   ctx.read_file(rel_path) -> String（需 fs:read；必须 canonicalize 后校验在仓库根内且非符号链接逃逸）
   ctx.write_file(rel_path, content)（需 fs:write；写前记录到审计，标注来源=插件）
   ctx.list_dir(rel_path)（需 fs:read）
   ctx.http_get_json(url, headers?)（需 net:*；限白名单域名，默认仅 api.github.com 与用户显式添加的域名）
   ctx.register_command({ id, title, run }) / ctx.register_panel({ id, title, location })
   ctx.show_toast({ level, message })
   ctx.get_setting(key) / ctx.set_setting(key, value)（需 settings:read / settings:write，键自动加插件前缀命名空间）
   ctx.get_git_log(limit, path?) -> 简化提交列表（需 git:read）
   ctx.git_stage(paths[]) / ctx.git_commit(message)（需 git:write；**必须**经 SnapshotManager 与审计，
     并在 UI 中标注"由插件 X 执行"）
2) 每个 host function：
   - 入口校验权限 → 未授权返回结构化错误（不是 panic）；
   - 参数做类型与范围校验（防越界/超长）；
   - 调用耗时与结果记入插件日志（不含敏感内容）。
3) 提供给插件的 ABI 说明文档 docs/PLUGIN-API.md：清单格式、权限表、每个 API 的签名与示例、
   限制与常见错误、三个示例插件的源码解读。
4) 版本兼容策略：apiVersion 以 MAJOR.MINOR 表示，MAJOR 不匹配拒绝加载；
   MINOR 升级只允许新增（不得移除或改签名）；在文档中明确"1.0 前不保证兼容"。
验收：为每个 host function 写单测（正常路径 + 未授权路径 + 参数非法路径）；
   断言所有 host function 都有权限检查（可用宏或代码扫描测试覆盖）。
```

#### T6.3 插件 UI 扩展点

`依赖：T6.2` ｜ `估时：4d` ｜ `审批点：**需要你确认插件面板的 UI 呈现方式**`

```text
【T6.3 插件 UI】
1) 命令扩展点：插件注册的命令进入命令面板（T5.9）；可绑定快捷键（在设置中显示为"插件：X 的命令"）。
2) 面板扩展点：
   - location: sidebar | bottom | repo-tab（在仓库详情中新增一个 Tab）；
   - 插件面板的 UI 渲染方式（需你决策，请给出建议并说明权衡）：
     方案 A：插件只提供数据，宿主用**受限的声明式 UI DSL**渲染（JSON 描述表格/列表/文本/进度/按钮），
             安全性最高、样式统一，但表达能力有限；
     方案 B：插件提供 HTML/CSS/JS，宿主在**独立 iframe（sandbox + 独立 origin）**中渲染，
             表达能力强，但需要处理 CSP、通信与样式隔离；
     方案 C：仅支持方案 A，方案 B 延后到 1.0 之后。
     建议：先实现方案 A（M6），把 B 作为后续 RFC。
3) 提交钩子扩展点：插件可注册 pre-commit / post-commit 处理器（只读或可修改提交信息，
   不允许阻断提交，除非插件在清单中声明并获用户授权 can_block）。
4) 自定义 Provider 扩展点：插件可注册 HostProvider（用于小众托管平台），
   但必须走同一套 HTTP 层与限流（防止插件绕过代理设置）。
5) 事件订阅：插件可订阅有限事件（repo_opened / repo_changed / commit_created / sync_completed），
   回调必须限时且不得阻塞主流程（异步分发 + 超时丢弃）。
验收：三个扩展点各有示例与测试；断言插件面板渲染失败时宿主显示兜底错误卡片而非崩溃；
   断言插件事件回调超时不影响宿主流程（写测试）。
```

#### T6.4 插件管理页

`依赖：T6.3` ｜ `估时：3d` ｜ `审批点：无`

```text
【T6.4 插件管理】
1) 插件列表页：已安装插件（名称、版本、作者、许可证、状态：启用/禁用/已停用（异常）、
   描述、主页链接）；支持启用/禁用/卸载/重新加载。
2) 权限面板（重点）：
   - 首次启用或权限变更时弹出授权对话框，**逐项**列出权限及其人类可读说明
     （例如 fs:write → "可以修改你打开的仓库中的文件"）；
   - 展示"该权限被使用的次数"（基于运行时统计）以增强透明度；
   - 已授权限可在详情页单独撤销（撤销后插件下次调用即失败）；
   - 危险权限（fs:write / git:write / net:*）用醒目样式并需要额外确认。
3) 插件日志页：按插件查看日志（时间、级别、消息、错误堆栈），支持复制与清空。
4) 开发者模式：从本地目录加载插件（需确认警告）、查看清单校验错误详情、热重载。
5) 安全提示：明确告知"插件由第三方提供，请仅安装你信任的来源"；
   显示插件文件的 SHA256（便于用户核对）；**不做**在线插件市场（避免供应链与审核成本，属不做清单）。
6) 空态：引导用户查看文档了解如何开发插件（链接 docs/PLUGIN-API.md 与示例插件）。
验收：E2E 覆盖 启用 → 权限授权 → 撤销权限 → 插件功能失效的完整链路；
   断言卸载插件后其数据目录被清理（或明确提示保留，二选一并在 UI 说明）。
```

#### T6.5 三个示例插件与插件开发文档

`依赖：T6.4` ｜ `估时：4d` ｜ `审批点：无`

```text
【T6.5 示例插件】
实现三个功能完整、有实际价值的示例插件（源码即开发教程）：
1) plugins/examples/commit-template：提交信息模板插件。
   - 提供 3 套模板（feat/fix/chore）与一个命令"用模板填写提交信息"；
   - 读取当前仓库的分支名与变更文件列表，按模板生成建议结构（**纯字符串规则，不得调用任何 AI**）；
   - 使用权限：git:read、ui:command、ui:toast。
2) plugins/examples/repo-stats：仓库统计面板插件。
   - 在 sidebar 注册面板，展示：最近 30 天提交数、作者分布（文本条形图）、
     提交时间热力图（用声明式 UI 的表格/条形表示）、当前分支与 ahead/behind；
   - 使用权限：git:read、ui:panel。
3) plugins/examples/repo-audit：仓库巡检插件。
   - 检查（只读）：大文件（> 1MB 的已跟踪文件）、疑似密钥文件（.env 等）、
     未忽略的构建产物目录、文件名大小写冲突风险、超过 100MB 的对象（LFS 建议）、
     提交信息规范符合率（按可配置的规则）；
   - 输出报告到面板，并提供"复制报告"与"导出 Markdown"；
   - 使用权限：git:read、fs:read（可选）、ui:panel。
要求：
- 每个插件的清单、源码、README 齐全；构建脚本明确（wasm32-wasip1 target）；
- 三个插件必须在 CI 中编译并运行基础冒烟测试；
- 编写 docs/PLUGIN-API.md 时以这三个插件为主线组织（先跑起来，再讲 API）。
验收：三个插件在应用中可加载、可用、可用截图（自绘）记录；冒烟测试通过。
```

#### T6.6 主题系统

`依赖：T0.3` ｜ `估时：3d` ｜ `审批点：无`

```text
【T6.6 主题】
1) 主题 JSON 格式：{ id, name, appearance: "light"|"dark", version, colors: { ... 与 tokens.css 对应 ... },
   fonts?: { ui?, mono?, sizeScale? }, xterm?: { ... } }
2) 内置主题：ForgeDesk Light / ForgeDesk Dark（默认）+ 至少 2 套额外原创配色
   （名称与配色必须原创，不得使用竞品或知名 IDE 的主题名与配色，例如不得命名或复刻 "Dracula"/"Monokai" 等）。
3) 功能：主题列表（预览卡片）、切换、导入 JSON、导出当前主题、删除自定义主题、
   "跟随系统"模式（监听 prefers-color-scheme）、非法主题时回退默认并提示具体字段错误。
4) 实时预览：主题编辑器（可选，量小则只做导入导出 + 变量对照表）。
5) 一致性：所有颜色必须经 CSS 变量；写一个脚本/ESLint 规则扫描 src 下的十六进制颜色字面量
   （白名单：tokens.css 与主题定义文件本身），CI 中执行。
6) 终端与编辑器主题同步（xterm theme 与 Monaco theme 由同一主题派生）。
7) 无障碍：在主题详情中显示对比度检查结果（至少正文与背景、按钮文字与按钮背景两组）。
验收：单测覆盖主题 JSON 校验（≥ 8 个非法用例）；E2E 覆盖切换主题 → 重启保持；
   断言颜色字面量扫描脚本通过。
```

#### T6.7 i18n 完整覆盖与贡献流程

`依赖：T0.6` ｜ `估时：3d` ｜ `审批点：无`

```text
【T6.7 i18n 完整化】
1) 全量抽取：把 src 与规则文件中所有用户可见文案迁移到 locales/{zh-CN,en-US}/*.json，
   命名空间：common / repo / workspace / diff / history / branch / sync / conflict / rebase /
   snapshot / github / terminal / editor / settings / plugin / diagnostics / updater / errors。
2) 英文文案必须**地道**（不得是中文直译），并注意术语一致性（维护术语表 docs/I18N-GLOSSARY.md：
   stage=暂存/stage、commit=提交、rebase 不翻译、snapshot=快照 等）。
3) 工具链：
   - pnpm i18n:lint：禁止未包裹的用户可见中文/英文字符串（白名单 // i18n-ignore）；
   - pnpm i18n:check：检测 key 缺失、多余 key、占位符不一致（如 {{count}}）、复数形式缺失；
   - pnpm i18n:extract：生成缺失 key 报告。
   三个脚本都在 CI 中运行，任一失败即构建失败。
4) 语言切换：设置页选择语言（含"跟随系统"）；切换后即时生效（无需重启），并刷新窗口标题与菜单。
5) 日期时间与数字：统一使用 Intl API（不得手写格式化），日期格式跟随语言，
   相对时间（"3 分钟前"）使用 Intl.RelativeTimeFormat。
6) 贡献流程：在 CONTRIBUTING.md 中写清如何新增语言（复制 en-US 目录 → 翻译 → 在 locales/index.ts 注册），
   并提供 Lua/JSON 校验脚本保证格式正确。
验收：i18n:lint 与 i18n:check 通过；抽查 30 条英文文案的地道性（回报中列出）；
   断言切换语言后界面无残留中文（E2E 扫描 DOM 中的中文字符，白名单为语言名称本身）。
```

#### T6.8 代理、SSH 与 GPG 设置与诊断

`依赖：T2.7, T4.2` ｜ `估时：4d` ｜ `审批点：**需要你确认是否需要支持企业自建 CA**`

```text
【T6.8 网络与密钥设置】
1) 代理设置：
   - 模式：不使用 / 系统代理（读取环境变量与系统设置）/ 手动（HTTP、HTTPS、SOCKS5 + 认证）；
   - no_proxy 列表（逗号分隔）；按 host 覆盖（如仅对 github.com 走代理）；
   - 连通性测试：分别测试 GitHub API（GET /meta 或 /zen）、raw.githubusercontent.com、
     以及 git 操作（git ls-remote 到一个公共仓库）；
   - 生效范围必须覆盖：HTTP 客户端（T4.2）、git CLI（通过 http.proxy 配置传递给子进程，
     使用 -c 参数而非修改用户配置，避免污染用户环境）、更新下载（T7.1）。
2) SSH 设置：
   - 检测 ~/.ssh 下的密钥（列出公钥文件名与类型，不读取私钥内容）；
   - 检测 ssh-agent（SSH_AUTH_SOCK / Windows 的 named pipe）与已加载密钥（ssh-add -l）；
   - known_hosts 查看与"信任新主机"流程（首次连接时展示指纹并要求用户确认，
     支持 SHA256 与 MD5 格式；**不得**自动设置 StrictHostKeyChecking=no）；
   - 测试连接：git ls-remote（5s 超时）并给出结构化结果；
   - 提供 ssh_config 只读查看（不编辑用户的 ssh_config，属不做清单）。
3) GPG 设置：
   - 列出本地密钥（gpg --list-secret-keys 解析，仅展示 key id、uid、过期时间）；
   - 设置默认签名密钥（写入**应用级**配置，通过 git -c user.signingkey 传递，不修改用户全局配置，
     或明确提示将要修改全局配置并需用户确认——请选择方案并说明）；
   - 测试签名：生成一段测试数据签名并验证；
   - commit.gpgsign 与 gpg.program 的检测与说明（不自动修改）。
4) 安全：所有私钥与口令永不读取明文；代理认证密码存 keyring。
验收：三平台手工验证（代理用本地 mock 代理、SSH 用真实公钥测试、GPG 用测试密钥）；
   单测覆盖各检测命令的输出解析；断言 git 子进程的代理/签名参数通过 -c 传递而非改用户配置。
```

#### T6.9 平台适配层补齐

`依赖：T2.7, T5.2` ｜ `估时：4d` ｜ `审批点：无`

```text
【T6.9 平台适配】
crates/platform 完整实现以下 trait 与各平台实现（#[cfg(target_os)]）：
1) CredentialStore：Windows Credential Manager / macOS Keychain / Linux Secret Service
   （不可用时回退加密文件，见 T2.7）。
2) ShellResolver：返回可用 shell 列表与默认 shell，含 pwsh/powershell/cmd/Git Bash/bash/zsh；
   探测路径并缓存。
3) PathNormalizer：路径分隔符、大小写敏感性、UNC 与长路径（Windows 加 \\?\ 前缀）、
   macOS Unicode NFD/NFC 规范化（路径比较必须规范化后比较）。
4) WatcherFactory：三平台文件监听实现与限制检测（见 T1.10）；
   Linux 额外检测 inotify 上限并提示。
5) Notifier：系统通知（Windows Toast / macOS UNUserNotification / Linux libnotify），
   用于后台任务完成、更新可用、冲突需要处理；提供开关与免打扰时段。
6) SystemIntegration：
   - 在系统文件管理器中显示文件 / 用默认应用打开文件；
   - 开机自启（可选，明确列出用户可见影响）；
   - 应用内隐藏/显示在托盘（Windows/Linux）、Dock 菜单（macOS）；
   - 关闭窗口行为设置（最小化到托盘 / 退出）。
7) 崩溃/无图形环境检测：Linux 无 DISPLAY/WAYLAND_DISPLAY 时给出明确错误而非崩溃。
要求：每个 trait 至少一个"可测的默认实现"（用于测试环境）与真实实现；
在 crates/platform/tests 中做跨平台条件测试。
验收：三平台手工验证 6 项能力；在回报中列出各平台已知限制与 workaround（写入 docs/PLATFORM-NOTES.md）。
```

#### T6.10 提交签名与验证展示

`依赖：T6.8` ｜ `估时：3d` ｜ `审批点：无`

```text
【T6.10 签名与验证】
1) 提交签名：
   - 支持 GPG 与 SSH 签名（git 2.34+ 支持 SSH 签名）；
   - 在提交面板提供签名开关（默认取仓库配置 commit.gpgsign），并在提交计划中展示将要使用的 key；
   - 签名失败（无 key、key 过期、gpg 不可用）必须给出精确原因与修复指引，且不产生未签名提交（除非用户显式选择"跳过签名"）。
2) 验证展示：
   - 历史页与提交详情中展示签名状态：Verified / Unverified / 未知 key / 签名损坏 / 未签名；
   - 使用 git log --show-signature 与 git verify-commit 的输出解析（不引入 GPG 库）；
   - 状态必须用**图标 + 文字**双重表达，并提供 tooltip 说明（例如"签名有效，签名者：X，密钥指纹：Y"）；
   - 在提交图节点上对已签名且验证通过的提交显示一个小徽标（原创图形）。
3) SSH 签名的 known_signers 配置读取与说明（gpg.ssh.allowedSignersFile）；
   提供引导文档 docs/SIGNING.md（GPG 与 SSH 两种方式的完整设置步骤）。
4) 批量验证：提供一个"验证最近 N 个提交的签名"的按需操作（不自动全量执行，避免性能问题），
   结果以列表展示。
验收：用测试 GPG 密钥与测试 SSH 密钥各完成一次签名提交与验证（回报中贴出关键输出）；
   单测覆盖 5 种签名状态的解析；断言签名验证不修改仓库。
```

### 4.8 M7 自动更新 / CI / 打包 / 文档 / 发布（T7.1–T7.10）

> **M7 起进入零成本发布工程。所有签名相关工作均为免费方案，禁止申请任何付费证书或账号。**

#### T7.1 自动更新（含 macOS 无公证可行性 Spike）

`依赖：T0.11` ｜ `估时：4d` ｜ `审批点：**Spike 结论需要你确认是否采用降级方案**`

```text
【T7.1 自动更新】
⚠️ 本任务的第一部分是 Spike，必须先验证再实现：
1) Spike（关键不确定性）：在 macOS 上，应用仅使用 **ad-hoc 签名（无 Apple 公证）** 时，
   tauri-plugin-updater 能否成功下载并替换应用包。请：
   - 生成一对更新签名密钥（minisign）：`pnpm tauri signer generate -w ~/.tauri/forgedesk.key`；
     公钥写入 tauri.conf.json，私钥**不得**进入仓库（本地开发用临时密钥，生产用 CI Secret）；
   - 构造本地更新场景：起一个本地 HTTP 服务提供 latest.json 与 .app.tar.gz，
     在 ad-hoc 签名的测试构建上执行更新，验证是否成功；
   - 分别在 macOS（arm64）与 Windows 上验证；Linux 用 AppImage 验证。
   - 结论写入 docs/UPDATE-SPIKE.md，明确：哪些平台可用、哪些不可用、失败时的具体错误。
2) 若 macOS 不可用 → 实现降级方案（**这不是失败，是备选**）：
   - 命令 update_check() 仍正常工作，发现新版本时弹出"发现新版本"对话框，
     提供"打开下载页"（openExternal 到 GitHub Releases）与"查看更新说明"；
   - UI 中明确说明"macOS 版本需手动下载安装（本项目未使用付费签名服务）"——
     这是诚实的产品呈现，不得含糊其辞。
3) 正式实现（在验证通过的平台上）：
   - 命令 update_check(force)、update_download(version)（进度事件）、update_install()（安装并重启）；
   - 设置项：自动检查（默认开，频率：启动后 60s 一次 + 每 24 小时一次）、更新渠道（stable/beta）、
     跳过该版本、仅下载不安装；
   - UI：状态栏与设置页的更新提示条（有新版本时），含版本号、发布日期、更新说明（Markdown 渲染）、
     "立即更新/稍后/跳过此版本"；
   - 失败处理：签名校验失败、下载失败、磁盘不足、权限不足 → 各自给出可读原因与重试入口，绝不静默失败；
   - 启动时如果检测到"上次更新未完成"的标记，提示用户并给出回退指引。
4) 安全：公钥硬编码；校验失败必须中止且不替换现有版本；严禁关闭校验的开关（不得提供该配置项）。
5) 文档：docs/RELEASE.md 中写明密钥生成、存储（CI Secret 名）、离线备份、
   轮换与丢失恢复流程（若私钥丢失，用户将无法通过自动更新升级 → 必须明确说明与规避方法）。
验收：docs/UPDATE-SPIKE.md 结论明确；在可用平台上完成一次真实的"旧版 → 新版"自动更新；
   写测试用本地 mock 服务器验证签名失败路径会被拒绝。
⚠️ 若 macOS 不可用或密钥管理方案需要你确认，请请求审批。
```

#### T7.2 CI 完整流水线（lint / typecheck / test / build / e2e / nightly）

`依赖：T0.10` ｜ `估时：3d` ｜ `审批点：无`

```text
【T7.2 CI 完整化】
1) ci.yml 增补：
   - 覆盖率门禁：使用 cargo-llvm-cov，断言 workspace ≥ 60%、domain ≥ 85%；
     前端用 vitest coverage，断言关键模块 ≥ 70%；覆盖率报告上传 artifact。
   - 破坏性操作安全测试套件（T3.11）作为独立 job，必须在 PR 中通过。
   - i18n 检查（T6.7）与合规检查（T0.12）纳入必过门禁。
2) e2e.yml：Playwright + tauri-driver
   - PR 时在 ubuntu-22.04 上运行（安装 webkit2gtk 依赖 + xvfb）；
   - 夜间在三平台运行；
   - 失败时上传 trace、截图与 window.__errs 内容作为 artifact。
3) nightly.yml（每日 02:00 UTC）：
   - 全量测试 + 基准测试（T2.9）→ 与基线对比，退化 > 10% 则失败并创建 Issue（用 actions/github-script）；
   - 构建并发布 beta 更新清单（指向 nightly 构建产物），供早期测试者使用。
4) audit.yml：每日运行 cargo audit、cargo deny、pnpm audit；
   发现高危漏洞时自动创建 Issue（含修复建议）。
5) 所有 workflow：
   - 第三方 action 固定 commit SHA；
   - 使用 concurrency 取消旧运行；
   - release 相关 secrets 仅在 release/nightly job 注入；
   - 设置合理的 timeout-minutes（避免卡死消耗额度）。
6) 写 docs/CI.md：每个 job 的作用、本地复现命令、常见失败原因与处理。
验收：故意制造覆盖率不足、E2E 失败、基准退化三种情况，确认对应 job 失败并给出可读原因。
```

#### T7.3 发布流水线与 GitHub Release

`依赖：T7.1, T7.2` ｜ `估时：4d` ｜ `审批点：**发布动作需要你审批（首次）**`

```text
【T7.3 发布流水线】
创建 .github/workflows/release.yml：
1) 触发：push tag `v*.*.*` 或 workflow_dispatch（可指定 channel=stable|beta）。
2) 阶段一 质量门禁：复用 ci.yml 的全部检查（lint/typecheck/test/clippy/cargo test/覆盖率/安全测试/合规检查）。
   任一失败 → 立即终止，不产生任何发布产物。
3) 阶段二 构建（matrix）：
   - macos-14（aarch64）与 macos-13（x86_64）分别构建，或使用 universal 目标（说明选择）；
   - ubuntu-22.04（AppImage/deb/rpm）；
   - windows-latest（msi/nsis）+ 便携版 zip（自行打包 dist 目录）。
   - 产物重命名为 ForgeDesk_<version>_<target>_<arch>.<ext>。
4) 阶段三 校验和与签名（免费）：
   - 生成 SHA256SUMS（覆盖全部产物）；
   - 用项目 GPG 私钥（来自 CI Secret GPG_PRIVATE_KEY，口令 GPG_PASSPHRASE）生成 SHA256SUMS.asc；
   - 断言 GPG 公钥文件存在于仓库 docs/keys/forgedesk-release.pub，且与私钥匹配（gpg --list-packets 校验）。
5) 阶段四 更新清单：
   - 为每个 target/arch 生成 latest.json（含 version、pub_date、notes、platforms[].signature 与 url）；
   - 上传到 GitHub Pages 分支（gh-pages）的 updates/<target>/<arch>/<channel>.json。
6) 阶段五 创建 Release：
   - 自动生成 Release Notes：从 Conventional Commits 分类（feat/fix/perf/refactor/docs/chore/
     BREAKING CHANGE），并在末尾附"安装说明（含免费方案下的 macOS/Windows 信任指引）"与校验和；
   - 全部产物作为附件上传；**必须等所有平台构建成功后再创建 Release**（不得发布残缺版本）。
7) 阶段六 通知：创建 Discussion 公告（用 actions/github-script 或 gh cli，需 GITHUB_TOKEN 权限）。
8) 灰度：channel=beta 时先只更新 beta 清单；稳定后再手动把 stable 清单指向该版本（手动步骤写入 RELEASE.md）。
验收：用一个 v0.9.0-test.x 预发布 tag 完整跑通流水线（验证后删除该 Release 与 tag，需人工确认）；
   回报中贴出产物清单、校验和、Release Notes 样例。
⚠️ 首次创建真实 Release 前请求人类审批。
```

#### T7.4 打包细节与平台集成

`依赖：T0.11, T7.3` ｜ `估时：3d` ｜ `审批点：无`

```text
【T7.4 打包细节】
1) Windows：
   - 同时产出 MSI（WiX，适合企业部署）与 NSIS（适合个人用户），
     并在下载页说明二者区别；NSIS 选择 perUser（避免管理员权限，降低摩擦）并说明理由；
   - 便携版 zip：包含 exe 与全部资源，双击即可运行（不写注册表、配置写在 exe 同目录的 portable-data/，
     需实现"便携模式"检测：存在 portable-data 目录时使用该目录作为数据目录）；
   - 长路径支持：manifest 中声明 longPathAware；
   - 应用图标与安装器图标使用原创图标。
2) macOS：
   - DMG 内含 Applications 快捷方式与安装说明文本（纯文本，不放竞品截图）；
   - 记录并断言 ad-hoc 签名存在（见 T8.1）；
   - 说明 Apple Silicon 与 Intel 的产物区分（或提供 universal）。
3) Linux：
   - AppImage：确保包含必要依赖、fuse 提示（若用户无 fuse，给出 --appimage-extract 的使用说明）；
   - deb：撰写正确的 control 文件（Depends 含 libwebkit2gtk-4.1-0、libgtk-3-0、libsecret-1-0 等），
     描述字段使用 "ForgeDesk — A Git client"；
   - rpm：spec 等价配置；
   - .desktop 文件：正确的 Categories（Development;RevisionControl;）、StartupWMClass、
     本地化 Name/GenericName/Comment（中英），并安装图标到 hicolor 的多个尺寸目录。
4) 文件关联与协议：注册 forgedesk:// 协议（仅声明，不注册 .git 相关关联）；
   在应用中实现最小协议处理（forgedesk://open?path=... → 打开仓库），并做路径安全校验。
5) 打包后冒烟测试（CI 中执行）：安装 → 启动 → 退出码 0 → 卸载（Windows/Linux）。
验收：三平台打包产物在干净环境中安装启动成功；desktop 文件通过 desktop-file-validate；
   deb 通过 lintian 基本检查（允许有说明的警告）。
```

#### T7.5 崩溃恢复与安全模式

`依赖：T0.8` ｜ `估时：3d` ｜ `审批点：无`

```text
【T7.5 崩溃恢复】
1) 会话标记：启动时写 session.lock（含 pid、启动时间、版本）；
   正常退出时删除；下次启动若发现残留且 pid 不存在 → 判定为异常退出。
2) 异常退出处理：
   - 展示"上次异常退出"提示（含崩溃时间与日志文件路径）；
   - 提供"查看日志"（logs_tail 高亮 panic 附近内容）、"安全模式启动"、"正常启动"；
   - 若连续 3 次异常退出 → 自动进入安全模式并提示（避免无限崩溃循环）。
3) 安全模式（Safe Mode）：
   - 禁用：插件、终端、文件监听、自动更新检查；
   - 只加载核心 Git 功能；顶部常驻横幅"安全模式已启用"并提供"退出安全模式"；
   - 在安全模式下记录一条审计（便于判断问题来源）。
4) 崩溃报告（**本地生成，不上传**）：
   - 生成 crash-report-<timestamp>.json：版本、平台、错误类型、堆栈、最近 200 行日志（脱敏）、
     最近 20 条操作记录（脱敏）；
   - 提供"复制到剪贴板"与"保存到文件"（用户在 GitHub 提 Issue 时自行粘贴）；
   - **禁止自动上传**（符合隐私承诺）。
5) 未保存数据保护：编辑器有未保存修改时崩溃 → 下次启动时提示并可恢复（实现草稿自动保存到
   app_data_dir()/drafts/，每 5 秒或内容变化后防抖保存）。
6) 与快照机制衔接：若崩溃时正处于破坏性操作中，下次启动检测到未完成的操作并提示"可回滚"。
验收：人为触发 panic（dev 命令）→ 重启 → 异常退出提示出现 → 安全模式可启动 →
   崩溃报告已生成且已脱敏（断言报告中不含 token 样例）；
   编辑器草稿恢复测试通过。
```

#### T7.6 遥测策略、隐私说明页与审计导出

`依赖：T1.11` ｜ `估时：2d` ｜ `审批点：**需要你确认遥测是否做（建议：完全不做）**`

```text
【T7.6 隐私与审计导出】
1) 需求澄清：本项目的隐私承诺是"遥测默认关闭"。请给出**建议**并请求人类决策：
   - 方案 A（推荐）：**完全不做遥测**。设置页只提供"隐私说明"与本地统计（本机可见）。
   - 方案 B：提供默认关闭的匿名统计（自建端点或复用 GitHub Issues 报告），
     开启前展示"将发送的字段预览"，且不含任何唯一标识。
   请只实现 A，除非人类选择 B（若选 B，需另开任务）。
2) 隐私说明页（应用内，非仅文档）：清晰列出：
   - 收集什么（方案 A 下为"什么都不收集"）；
   - 存储在哪里（本地数据目录），包含哪些文件与数据库表；
   - 何时联网、向哪些域名联网（GitHub API、更新清单地址）、发送什么；
   - 用户权利：一键导出全部本地数据、一键删除全部本地数据、清除缓存；
   - 与 GitHub 的关系声明（不使用 GitHub Logo、非官方）。
3) 数据管理功能（设置 → 隐私与数据）：
   - 显示各项本地数据的占用（DB 大小、快照缓存大小、日志大小、草稿大小）；
   - "导出全部数据"（zip，含设置、审计、快照元数据；**不含** token）；
   - "删除全部数据"（二次确认 + 需输入确认词，删除后应用回到初始状态）；
   - "清理缓存与日志"（保留设置的轻量清理）。
4) 审计日志导出完善（T1.11 的增强）：支持按时间范围与仓库筛选导出 JSON/CSV，
   导出文件统一 UTF-8（可选 BOM），并在文件中包含"导出时的应用版本与时间"。
5) privacy 相关的文档同步：docs/PRIVACY.md 与应用内页面内容一致（写测试/脚本断言关键段落一致）。
验收：E2E 覆盖隐私页展示、导出数据、清理缓存；断言导出包中不含任何凭据（自动扫描导出内容）；
   断言默认配置下应用不发起除 GitHub API 与更新清单之外的外部请求（用本地代理抓包或 mock DNS 断言）。
⚠️ 请就遥测方案请求人类决策。
```

#### T7.7 用户文档五件套

`依赖：全功能就绪` ｜ `估时：4d` ｜ `审批点：**需要你审阅文档的准确性**`

```text
【T7.7 用户文档】
编写以下文档（要求：面向真实用户、步骤可复制、包含原创截图/示意图，禁止使用竞品截图）：
1) README.md（最终版）：一句话价值 + 核心能力（在线性 DAG、可视化 rebase、三栏冲突、快照回滚四点、
   每点配原创动图/截图）+ 安装（三平台，指向 docs/install/*）+ 快速上手（5 步：
   打开仓库 → 查看变更 → 暂存提交 → 查看历史 → 安全回滚）+ 常见问题入口 +
   许可证 + **完整免责声明**（PLAN §9.5 模板）+ "本产品不含 AI 功能"声明。
2) docs/install/{windows,macos,linux}.md：分步安装（含免费方案下的信任处理）、
   校验和验证步骤、常见拦截处理、卸载方法、便携版说明、系统要求。
3) docs/manual/*.md：按功能域的图文手册（工作区、历史、分支与同步、冲突解决、
   rebase 整理、快照与回滚、GitHub 集成、终端、编辑器、插件与主题、设置）。
   每个功能包含：它能解决什么问题 → 怎么操作（编号步骤）→ 等价 git 命令（教学价值）→ 常见误区。
4) docs/FAQ.md（≥ 20 条）：涵盖"会不会弄丢我的代码"、"是否需要登录"、"是否收费"、
   "是否会上传我的代码"、"支持私有仓库吗"、"支持企业版 GitHub 吗"、"支持 GitLab 吗"（说明 V2 计划）、
   "为什么安装时提示不安全"（免费方案解释，措辞坦诚）、"支持哪些 Git 版本"、
   "如何反馈问题"、"插件安全吗"、"能离线用吗"、"支持中文吗"、"和 GitKraken/Fork 有什么区别"
   （用文字对比表，不引用竞品截图与 Logo）。
5) docs/TROUBLESHOOTING.md（≥ 15 条）：Linux WebKit 渲染问题、inotify 上限、
   凭据库不可用、代理配置、SSH 主机指纹、GPG 签名失败、LFS 未安装、
   长路径（Windows）、文件名大小写冲突（macOS）、浅克隆限制、子模块问题、
   大仓库性能、自动更新失败、托盘图标异常、暗色主题下的终端颜色。
6) 所有文档中的命令必须实际验证过（在回报中说明验证方式）；链接必须有效（用脚本检查）。
验收：文档覆盖率（对照 4.2 功能表，标注已记录/未记录）；markdown 链接检查通过；
   抽查 10 条命令可复制即用。
⚠️ 完成后请求人类审阅文档准确性与措辞（尤其是免费方案下的信任说明）。
```

#### T7.8 社区文件与模板

`依赖：T0.9` ｜ `估时：2d` ｜ `审批点：无`

```text
【T7.8 社区文件】
1) CODE_OF_CONDUCT.md（Contributor Covenant v2.1 原文 + 联系方式）。
2) SECURITY.md：支持的版本范围、漏洞报告方式（GitHub Security Advisory 私密报告优先，
   提供备用邮箱）、响应时限说明、90 天披露政策、明确"不在公开 Issue 中披露漏洞细节"。
3) GOVERNANCE.md：当前为"单一维护者 + AI 代理执行"模式，说明决策流程、
   贡献者晋升为维护者的条件（≥ 5 个被合并的 PR + 持续参与 3 个月）。
4) .github/ISSUE_TEMPLATE/：bug_report.yml（含版本、平台、复现步骤、日志、
   "是否已尝试安全模式"）、feature_request.yml、rfc.yml（范围变更必须走 RFC）、
   question.yml（引导到 Discussions）、plugin_issue.yml（要求提供插件源与清单）。
5) .github/pull_request_template.md：包含变更说明、影响面、测试证据、
   文档更新、以及**合规勾选项**（未复制竞品 UI / 未使用受限商标 / 未引入 AI 依赖 / 未引入付费依赖）。
6) .github/DISCUSSION_TEMPLATE/：公告、想法、问答、展示（Show and tell）。
7) CONTRIBUTORS.md（初始为空模板 + 自动生成脚本：从 git log 提取贡献者）。
8) 提供 .github/workflows/stale.yml：自动标记长期无响应的 Issue（30 天提醒、60 天关闭，
   但 exempt 标签与 RFC 不参与）。
验收：所有模板在 GitHub 上可正常渲染（用 YAML 校验工具验证）；PR 模板勾选项齐全。
```

#### T7.9 官网静态站（GitHub Pages 免费域名）

`依赖：T7.7` ｜ `估时：3d` ｜ `审批点：无`

```text
【T7.9 官网】
使用 VitePress 或 Astro（二选一并说明理由）构建静态站，部署到 GitHub Pages（**免费域名，不购买域名**）：
1) 首页：一句话价值、四点核心能力（配原创图）、下载按钮（自动识别平台并高亮对应产物）、
   星标仓库入口、简短的技术说明（Tauri + Rust，体积小）。
2) /download：版本矩阵表（平台 × 格式 × 大小 × SHA256 链接）、
   校验和验证步骤（三平台命令）、GPG 公钥下载、
   **免费方案信任说明**（坦诚说明未使用付费签名，给出 macOS/Windows 的处理步骤）。
3) /docs：直接渲染 docs/ 下的用户手册（同源内容，避免两处维护；
   在构建时读取 docs 或使用 submodule/软链，请选择可行方案并说明）。
4) /changelog：从 GitHub Releases 拉取并渲染（构建时或运行时 fetch）。
5) /privacy、/license、/about（含免责声明与"不含 AI 功能"声明）。
6) SEO 与可访问性：title/description/og 图（原创）、语义化标签、键盘可达、对比度达标、深色模式。
7) CI：.github/workflows/pages.yml，在 tag 发布或 main 更新时构建并部署到 gh-pages。
8) 在下载页实现"平台自动识别"（navigator.userAgent / userAgentData + 手动切换兜底）。
验收：站点可访问；Lighthouse 性能/可访问性 ≥ 90；下载链接与实际 Release 产物一致（写脚本校验）；
   全站无竞品截图与受限商标素材。
```

#### T7.10 发布演练与 v1.0.0

`依赖：T7.1–T7.9` ｜ `估时：2d` ｜ `审批点：**必须由你批准发布**`

```text
【T7.10 发布演练与 v1.0.0】
1) 预发布演练（不发正式版）：
   - 打 v0.9.0-rc.1 tag → 走完整发布流水线 → 校验三平台产物、校验和、GPG 签名、
     更新清单、Release Notes；
   - 在三个干净环境（Windows 全新用户、macOS 全新用户、Ubuntu 容器）按官方文档执行：
     下载 → 校验 → 安装 → 首次启动 → 打开仓库 → 提交 → 查看历史 → 回滚 → 卸载；
     记录每一步的耗时与卡点，写入 docs/acceptance/RELEASE-DRILL.md；
   - 更新路径演练：安装 v0.8.0（或更早的构建）→ 触发自动更新 → 确认升级成功（可用平台）；
   - 回滚演练：把更新清单指回上一版本，确认客户端能正常降级（或至少能提示）。
2) 根据演练结果修复文档与流程问题。
3) 正式发布 v1.0.0：
   - 确认 CHANGELOG 完整、版本号三处一致（Cargo.toml / package.json / tauri.conf.json / tauri.conf 的 version）；
   - 打 tag v1.0.0 → 流水线执行 → 检查 Release 内容 → 手动把 stable 更新清单指向 v1.0.0；
   - 发布 Discussion 公告 + 官网更新。
4) 发布后 72 小时：
   - 监控新 Issue（数据相关 Bug 优先）；
   - 若出现严重问题（损伤仓库数据）→ 立即按 RELEASE.md 的回滚预案处理（下架 Release 为 draft + 公告 + 快速修复版本）。
5) 编写 docs/acceptance/M7.md 与 docs/acceptance/RELEASE-DRILL.md。
验收：演练记录完整且所有步骤通过；v1.0.0 Release 内容正确；官网与文档同步更新。
⚠️ 打正式 tag 前必须请求人类批准（展示 Release Notes 草稿与产物清单）。
```

### 4.9 M8 零成本发布加固 / 分发 / 社区（T8.1–T8.8）

> **本阶段零成本约束最强：不得申请任何付费证书、付费账号、付费商店。**
> 若某项工作必须付费才能完成，直接标记为「不做」并实现免费替代方案，不要请求预算。

#### T8.1 macOS ad-hoc 签名验证与首次启动信任引导

`依赖：T7.3` ｜ `估时：3d` ｜ `审批点：无`

```text
【T8.1 macOS 零成本信任】
1) 构建断言：在 release.yml 的 macos job 中增加验证步骤：
   - 对 .app 执行 `codesign -dv --verbose=2 ForgeDesk.app`，断言输出包含签名信息
     （Tauri 在无 identity 时使用 ad-hoc 签名；若输出为 "code object is not signed at all" 则 job 失败）；
   - 对 arm64 产物额外断言 `codesign --verify --deep --strict` 不报错（ad-hoc 也应通过结构校验）；
   - 明确记录：本步骤是**免费方案**的签名验证，不是 Apple 公证。
2) 编写 docs/install/macos.md，内容必须可被普通用户执行：
   - 系统要求（macOS 版本、芯片）；
   - 安装：打开 DMG → 拖入 Applications；
   - 首次打开的三个方法（按推荐顺序）：
     ① 右键（或 Control+点击）应用 → 选择"打开" → 在弹窗中再次点击"打开"；
     ② 系统设置 → 隐私与安全性 → 向下滚动找到 ForgeDesk → 点击"仍要打开"；
     ③ 终端命令：`xattr -dr com.apple.quarantine /Applications/ForgeDesk.app`（附说明该命令的作用）；
   - 校验下载：`shasum -a 256 ~/Downloads/ForgeDesk_x.y.z_aarch64.dmg` 与下载页 SHA256 对比；
   - 常见错误 FAQ：
     * "ForgeDesk 已损坏，无法打开" → 说明这是 macOS 对未公证应用的提示，给出方法 ③；
     * "无法验证开发者" → 给出方法 ①②；
     * "无法打开，因为它来自身份不明的开发者" → 说明原因并提供步骤；
   - 为什么会有这个提示：**坦诚解释**"本项目未使用 Apple 付费签名服务，因此 macOS 无法自动验证来源"，
     并引导用户自行校验 SHA256/GPG。
3) 应用内实现 FirstRunTrustHint 组件：
   - 仅在首次启动时展示（写入 settings，可"不再显示"）；
   - 内容：本应用未使用付费代码签名 → 你可以如何验证下载可信（校验和 / GPG / 源码自行构建）；
     提供"查看校验指引"（打开官网 /docs 校验段落或内联步骤）与"我知道了"；
   - 措辞必须坦诚、专业、不制造恐慌，不提供"关闭安全检查"之类的危险建议。
4) 冒烟记录：在一台从未安装过本应用的 macOS 上，按文档从下载到启动成功，
   记录每一步耗时与卡点，写入 docs/acceptance/M8-macos.md。
验收：CI 签名断言生效（故意去掉签名配置时 job 失败）；文档步骤实测可完成；
   首次启动引导页正确显示且可关闭。
```

#### T8.2 Windows 信任加固与免费签名计划申请材料

`依赖：T7.3` ｜ `估时：2d` ｜ `审批点：**需要你决定是否申请 SignPath 免费计划**`

```text
【T8.2 Windows 零成本信任】
1) 便携版 zip 完善：确保便携模式可用（存在 portable-data 时使用该目录作为数据目录），
   在 zip 内放置 README.txt（纯文本）说明：解压即用、校验方法、SmartScreen 处理方式。
2) 校验和与文档：
   - docs/install/windows.md 提供可复制的 PowerShell 命令：
     `Get-FileHash .\ForgeDesk_1.0.0_x64-setup.exe -Algorithm SHA256`
     以及与下载页 SHA256 的对比说明；
   - 提供 GPG 校验步骤（Windows 用户可用 gpg4win，或提供纯 PowerShell 的哈希校验替代）。
3) SmartScreen 处理指引：
   - 说明"Windows Defender SmartScreen 已阻止"弹窗的成因（文件无代码签名信誉）；
   - 步骤：点击"更多信息" → "仍要运行"；
   - **优先推荐**用户改用 Scoop / Winget 安装（来源更可信、可自动校验）；
   - 明确说明本应用未使用付费代码签名，并引导用户校验 SHA256。
4) 生成 docs/install/signpath-application.md（不代为提交申请）：
   - 整理面向开源项目的免费代码签名计划（如 SignPath Foundation 等）申请所需信息清单：
     项目名称与主页、公共仓库地址、开源许可证类型与文件、构建与发布流程说明、
     维护者信息占位、产物如何分发、是否接受源码审计；
   - 提供一份可直接复制的英文申请文案草稿（说明项目用途、用户规模预期、为何需要签名）；
   - 明确标注"本文件供人类提交申请使用，代理不得自行提交或声称已提交"。
5) 在 README 顶部"安装"章节附上 Windows 用户的三步提示（下载 → 校验 → 安装），降低首次摩擦。
验收：文档命令实测通过；便携版 zip 在干净 Windows 上解压即用；
   SignPath 申请材料完整（由人类复核后提交）。
⚠️ 请就"是否申请 SignPath 免费计划"请求人类决策（申请需人类亲自操作）。
```

#### T8.3 包管理器清单（Homebrew / Scoop / Winget / Flathub / Snap / AUR）

`依赖：T7.3` ｜ `估时：6d` ｜ `审批点：**所有第三方仓库提交必须由你执行**`

```text
【T8.3 包管理器清单】
在 packaging/ 下为 6 个免费渠道生成并维护清单；CI 在 tag 发布时自动更新版本与 hash 并开 PR
（**绝不自动推送到第三方仓库**，只开本地 PR 或在自有仓库开 PR）。
1) Homebrew Cask（自有 tap 仓库 homebrew-tap）：
   packaging/homebrew/forgedesk.rb
   包含：version、sha256（arm64 与 intel 两个 dmg）、url（指向 GitHub Release 资产）、
   name "ForgeDesk"、desc "A Git client for everyone"、homepage、app "ForgeDesk.app"、
   zap trash（应用数据与缓存目录）、livecheck（GitHub latest release）。
2) Scoop（自有 bucket 仓库）：
   packaging/scoop/forgedesk.json
   包含：version、description、homepage、license、architecture.64bit/arm64 的 url + hash、
   extract_dir、bin、shortcuts、checkver（github）、autoupdate（含 hash 提取的 url 模板）。
3) Winget：
   packaging/winget/ForgeDesk.ForgeDesk.yaml（version manifest）、
   ForgeDesk.ForgeDesk.installer.yaml（InstallerType: wix/nullsoft、InstallerSha256、
   InstallerLocale、ProductCode/AppsAndFeaturesEntries、ReleaseDate）、
   ForgeDesk.ForgeDesk.locale.en-US.yaml 与 .locale.zh-CN.yaml。
   附 packaging/winget/PUBLISH.md：fork microsoft/winget-pkgs → 放到 manifests/f/ForgeDesk/ForgeDesk/<version>/ →
   winget validate → 提 PR 的分步说明（供人类执行）。
4) Flathub：
   packaging/flatpak/org.forgedesk.ForgeDesk.yml（runtime org.gnome.Platform，sdk 对应版本，
   finish-args 仅：--share=network、--socket=x11/wayland、--device=dri、--filesystem=home 的必要子集、
   --talk-name=org.freedesktop.secrets（凭据库）、--socket=ssh-auth（SSH agent），
   **不得**请求 --filesystem=host 等过宽权限；被拒时准备申诉说明）；
   metainfo.xml（含 screenshots 使用原创截图、releases 段落、content_rating、developer 信息）；
   附 packaging/flatpak/PUBLISH.md 提交流程。
5) Snapcraft：
   packaging/snap/snapcraft.yaml（confinement: strict、base、apps.forgedesk 的 plugs：
   home、network、network-bind、desktop、wayland、x11、ssh-keys、password-manager-service；
   grade: stable；附 PUBLISH.md，说明需人类登录 Snapcraft 账号并执行 snapcraft upload）。
6) AUR：
   packaging/aur/PKGBUILD（source 指向 AppImage + sha256sums、pkgname=forgedesk-bin、
   pkgver/pkgrel、arch、license=('Apache-2.0')、depends 与 optdepends、install 脚本、
   maintainer 占位）、.SRCINFO（可由 makepkg --printsrcinfo 生成）。
   附 packaging/aur/PUBLISH.md（生成 .SRCINFO → 提交到 AUR 的步骤；需人类 AUR 账号）。
7) CI 作业 packaging-update.yml：在 Release 创建后运行，抓取产物 SHA256，
   更新上述清单中的 version/hash，并在 ForgeDesk 自有仓库开 PR（
   对第三方仓库只生成"待提交的补丁文件"作为 artifact）。
8) 每个渠道目录下附 README.md：提交方式、更新流程、常见审核拒绝原因与规避、
   "如果被拒应该怎么改"。
验收：本地可验证的部分全部验证（brew audit --cask 本地可跑则跑、winget validate、
   flatpak-builder 在容器中构建通过、snapcraft 本地构建、makepkg 生成 .SRCINFO）；
   第三方提交步骤写成清单交人类执行。
⚠️ 所有对第三方仓库的实际提交（PR/上传）必须请求人类执行。
```

#### T8.4 官网下载页与校验和/安装指引最终化

`依赖：T7.9, T8.5` ｜ `估时：3d` ｜ `审批点：无`

```text
【T8.4 下载页最终化】
1) 版本矩阵自动同步：
   - 构建时通过 GitHub API（或构建期脚本读取 Release 资产列表）生成
     data/releases.json，包含：版本、发布日期、每个平台的产物名/大小/SHA256/下载链接、
     各包管理器安装命令（brew/scoop/winget/flatpak/snap/aur）；
   - 下载页渲染该矩阵，并**自动生成校验命令**（点击"复制校验命令"按钮按用户平台给出对应命令）。
2) 安装指引区块（三平台 Tab）：
   - 每个平台展示：推荐方式（包管理器优先）→ 手动下载 → 校验 → 信任处理 → 首次启动；
   - 内容与 docs/install/*.md 保持一致（构建时读取同一数据源，避免两处维护）。
3) 校验与签名区块：
   - SHA256SUMS 与 SHA256SUMS.asc 下载链接；
   - GPG 公钥下载 + 指纹展示（便于用户核对）；
   - 三平台的校验命令（shasum / Get-FileHash / sha256sum）+ GPG 验证命令；
   - 提供"如何从源码自行构建"的链接（最高信任级别路径，README 中已有构建说明）。
4) 免费方案说明区块（坦诚、专业）：
   - 标题如"关于安装安全提示"；
   - 说明本项目未使用付费代码签名服务，因此 macOS/Windows 会显示安全提示；
   - 给出逐步处理方法，并强调"请务必通过校验和验证你下载的文件"；
   - 不得出现抱怨或推卸责任的措辞。
5) 校验脚本：scripts/verify-release.mjs，检查下载页上的每个链接可达、每个 SHA256 与
   GitHub Release 资产一致；在 CI（发布后）与本地可运行。
验收：verify-release.mjs 全绿；下载页在移动端可用；无竞品截图与受限商标素材；
   文案经"非技术用户可理解"自检（用词平实、步骤编号）。
```

#### T8.5 Release GPG 签名与校验和自动化

`依赖：T7.3` ｜ `估时：2d` ｜ `审批点：**GPG 密钥生成需要你确认（私钥备份责任在你）**`

```text
【T8.5 GPG 签名流水线】
⚠️ 免费方案下的信任根：项目 GPG 密钥。私钥丢失会导致后续版本无法被验证为同一维护者签名，
   因此**密钥生成与离线备份必须由人类确认**。
1) 生成项目 GPG 密钥（Ed25519，无过期或 3 年过期 + 明确轮换流程）：
   - 代理只在有可用 GPG 的环境下生成，并输出**公钥**到 docs/keys/forgedesk-release.pub；
   - 私钥**不得**写入仓库，须由人类导入为 GitHub Actions Secret：GPG_PRIVATE_KEY（ASCII armored）、
     GPG_PASSPHRASE、GPG_KEY_ID；
   - 在回报中给出供人类执行的完整备份命令与存放建议（加密离线介质 + 至少两处异地），
     以及"私钥丢失后的恢复流程"（生成新密钥 → 更新公钥 → 在 Release 说明中公告密钥轮换 →
     引导用户信任新公钥）。
2) release.yml 增补：
   - 汇总全部产物生成 SHA256SUMS（按平台排序，格式稳定）；
   - `gpg --batch --yes --armor --detach-sign --local-user <KEY_ID> -o SHA256SUMS.asc SHA256SUMS`；
   - 校验：`gpg --verify SHA256SUMS.asc SHA256SUMS` 必须成功，否则 job 失败；
   - 把 SHA256SUMS 与 SHA256SUMS.asc 一并上传 Release。
3) 提供用户侧校验文档与脚本：
   - docs/VERIFY.md：三平台完整校验步骤（导入公钥 → 验证签名 → 校验文件哈希）；
   - scripts/verify-download.sh 与 scripts/verify-download.ps1：一键校验脚本（下载后可运行）；
   - 在 README 与下载页链接到此文档。
4) 公钥发布：仓库内 docs/keys/ + 上传到公开 keyserver（若人类同意），
   并在 Release Notes 与官网展示指纹。
5) 在 CI 中增加断言：docs/keys/forgedesk-release.pub 存在且与用于签名的私钥匹配
   （通过 gpg --list-packets 比对 key id）。
验收：在一台干净机器上按 docs/VERIFY.md 完成校验（记录输出）；
   故意篡改一个产物字节后校验失败（证明校验有效）。
⚠️ 密钥生成与备份方案需要人类确认后再执行。
```

#### T8.6 推广内容制作（全免费渠道）

`依赖：T8.1–T8.5` ｜ `估时：5d` ｜ `审批点：**发布前需要你审阅文案与合规性**`

```text
【T8.6 推广内容】
⚠️ 合规红线（每份材料都要检查）：
- 统一使用 "ForgeDesk — A Git client"，不得使用 "Git"/"GitHub" 作为产品名的一部分；
- 不得使用 Git / GitHub / Tauri 官方 Logo、Octocat 及其变体；
- 不得使用竞品截图做对比（只能文字表格）；
- 不得出现"官方""兼容 GitHub 官方""被 GitHub 推荐"等暗示关联的表述。
产出（全部原创，图片自行生成或绘制）：
1) 演示视频 2 个（每个 60–120 秒，无语音或配字幕脚本）：
   - 视频 A《3 分钟看懂：再也不用担心 git reset --hard 丢代码》
     —— 聚焦"快照 + 一键回滚"，含真实操作录屏（用 dev 构建 + 测试仓库）；
   - 视频 B《可视化解决 Git 冲突与整理提交历史》
     —— 聚焦三栏冲突向导 + 拖拽 rebase。
   交付：录制脚本（分镜 + 旁白/字幕文案）+ 实际录屏文件 + 封面图（原创）。
2) 技术文章 3 篇（Markdown，可直接发布到掘金/知乎/Dev.to/Hashnode）：
   - 《如何为 10 万提交的仓库渲染可交互 DAG：我们的布局算法与踩坑》
   - 《双 Git 引擎设计：什么时候用 libgit2，什么时候必须调用 git CLI》
   - 《零成本发布一个跨平台桌面应用：Tauri 打包、签名替代方案与免费分发渠道》
3) 发布文案包 docs/launch/：
   - Hacker News Show HN 文案（英文，200 词内，突出痛点与技术）；
   - Product Hunt 文案（英文，含 tagline、描述、首条评论）；
   - Reddit（r/git、r/github、r/opensource、r/rust、r/linux）各一版（注意各版规则，禁止纯广告）；
   - V2EX / 知乎 / 掘金中文文案各一版；
   - 一条 200 字内的"电梯稿"（任何渠道通用）。
4) 素材库 docs/launch/assets/：原创截图（10 张，覆盖四张核心能力卡片）、
   一张对比表图片（文字表格渲染成图，不含竞品 Logo）、Logo 与图标多尺寸。
5) 发布节奏与渠道排期表（见 PLAN §14.2/§14.3 的落地版）。
验收：全部素材通过合规自检（逐条对照上面红线，在回报中给出检查表）；
   视频时长与分辨率达标；文案无拼写错误（用脚本或人工校对并说明）。
⚠️ 所有对外发布动作需人类审批后执行（代理只产出素材）。
```

#### T8.7 社区平台搭建与运营规范

`依赖：T7.8` ｜ `估时：2d` ｜ `审批点：**需要你决定是否建 Discord/Matrix 等即时社区**`

```text
【T8.7 社区运营】
1) GitHub Discussions 配置：
   - 分类：📣 Announcements（仅维护者可发）、💡 Ideas、🙋 Q&A、🛠️ Show and tell（展示你的用法/插件）、
     🐛 Bug triage（从 Issue 转来的集中讨论）、📚 Docs；
   - 欢迎贴（置顶）：项目定位、如何提问、如何贡献、响应预期（明确 SLA）、行为准则链接；
   - 每类提供模板/引导文案。
2) 运营规范 docs/COMMUNITY.md：
   - 响应 SLA：普通 Issue 首次响应 ≤ 72 小时；P0（数据风险）≤ 12 小时；
   - Issue 分类与标签体系（type/*、area/*、priority/*、status/*、good first issue、help wanted）；
   - 不处理的情况：辱骂、重复提交、要求实现"不做清单"内的功能（提供统一回复模板）；
   - 反馈处理闭环（对应 PLAN §14.4）：每周整理 → 进入 RFC 或排期 → 回复提交者 → 月度变更摘要。
3) 即时社区（二选一，需人类决策）：
   - 方案 A（推荐，零维护）：不建即时社区，全部走 Discussions；
   - 方案 B：Matrix（免费、开放协议、可自建）房间；
   - 方案 C：Discord（体验好，但需人工审核与管理）。
   请实现 A 的文档化方案，若人类选择 B/C 则另开任务。
4) 月度节奏落地：
   - .github/workflows/monthly-digest.yml：每月 1 日自动统计上月数据
     （新 Issue、已关闭、合并 PR、新贡献者、Release 数）并生成 Discussion 草稿（不自动发布，需人工确认）；
   - 维护 CHANGELOG 的月度摘要段落。
5) 提供回复模板库 docs/templates/replies/（常见问题、范围外请求、无效报告、安全报告、感谢贡献）。
验收：Discussions 分类与欢迎贴已创建（需人类执行 GitHub 操作，代理提供内容）；
   COMMUNTIY 相关文档（docs/COMMUNITY.md）与模板库完整；月度摘要工作流可手动触发验证。
```

#### T8.8 贡献者引导与 M8 收口

`依赖：T7.8, T8.1–T8.7` ｜ `估时：2d` ｜ `审批点：**Issue 创建需人类确认（或授权代理批量创建）**`

```text
【T8.8 贡献者引导与 M8 验收】
1) 设计并列出 15 个 good first issue（低门槛、有明确边界、有引导），建议分布：
   - 文档类 4 个：新增 3 条故障排查条目、补充中文手册某一节、修正一处英文文案、补充术语表；
   - i18n 类 2 个：新增一种语言包骨架、补充现有语言的未翻译 key；
   - 测试类 3 个：为某个解析器补 fixture 用例、为某个组件补无障碍测试、为诊断规则补误报用例；
   - 功能小项 3 个：新增一个诊断规则、新增一个快捷键预设、新增一个主题（原创配色）；
   - 插件类 2 个：一个只读统计插件、一个提交信息校验规则插件；
   - 工程类 1 个：改进某个性能基准脚本。
   每个 Issue 必须包含：背景、目标、具体步骤指引、涉及文件、验收标准、
   "如何运行测试"、"不确定时该问谁"。
2) 为每个 Issue 打上 good first issue + area/* 标签，并指派到对应的里程碑/项目看板。
3) 贡献者体验打磨：
   - 验证"新人按 CONTRIBUTING.md 从零搭起环境并跑通测试"的路径（在干净环境实测并记录耗时）；
   - 提供 docs/DEV-ENV.md（各平台依赖安装、常见构建错误）；
   - 提供 .devcontainer/devcontainer.json（可选，方便在线贡献）。
4) M8 收口验收：
   - 逐条对照 PLAN 的 M8 验收标准给出证据（含 macOS/Windows 实测记录、包管理器安装成功的输出、
     官网与校验和链接、GPG 校验通过记录、全流程零支出确认）；
   - 编写 docs/acceptance/M8.md；
   - 输出"下一阶段路线图建议"（V1.1 与 V2 的功能优先级，
     参考 PLAN 4.2 功能表中标为 V1.1/V2 的条目，按用户价值与成本排序）。
验收：15 个 Issue 内容完整并已创建（或已准备好待人类批量创建）；
   M8 验收报告完整；路线图建议提交人类评审。
```

---

## 5. 通用运维提示词（OPS）

> 这些提示词在任何阶段都可能用到。建议把它们存为片段，按需调用。

### OPS-1 任务续跑（上下文丢失后恢复）

```text
【OPS-1 续跑】
我在继续执行任务 <任务ID>。请先重建上下文：
1. 阅读 AGENTS.md 与 docs/PLAN.md 中该任务所属章节。
2. 用 git log --oneline -20 与 git status 查看当前进度。
3. 检查是否已有该任务的部分实现（grep 关键类型/文件名）。
4. 输出：已完成部分、未完成部分、你计划继续的步骤（≤ 8 步），然后继续执行。
不要重复已完成的工作；若发现之前实现有误，先说明再修正。
```

### OPS-2 纠偏（代理跑偏了）

```text
【OPS-2 纠偏】
你当前的实现偏离了 docs/PLAN.md 的约定。具体问题：
- 偏离点：<描述，例如"在 domain crate 中引入了 IO 依赖" / "界面布局与竞品雷同" /
  "引入了 AI 依赖" / "自行扩大了功能范围">
- 期望：<描述期望行为，并引用 PLAN 的章节号>
请执行：
1. 说明为什么会发生这个偏离（1–3 句）。
2. 列出需要回退/修改的文件与具体改法。
3. 执行修正，并运行质量门禁。
4. 在回报中说明如何避免再次发生（是否需要补充 lint 规则/文档约束/测试断言来固化防止复发）。
```

### OPS-3 卡住求裁决

```text
【OPS-3 卡住请求裁决】
你在任务 <任务ID> 上遇到了阻碍。请按以下格式输出，不要继续尝试：
1. 现象：具体错误/失败命令与完整输出（脱敏后）。
2. 已尝试的方案：列出 ≥ 3 种尝试及其结果。
3. 根因判断：你认为的根本原因，以及为什么无法自行解决。
4. 可选方案：2–3 个方案，每个方案的成本、风险、对既有架构的影响。
5. 你的建议：推荐哪个方案及理由。
6. 若不做决策的后果：阻塞范围与影响。
等待人类在 §2.3 格式下回复后再继续。
```

### OPS-4 代码审查

```text
【OPS-4 自我代码审查】
对最近的实现做一次严格审查，输出问题清单（不要立即修改，先列问题）：
1. 正确性：边界条件（空仓库、detached HEAD、浅克隆、非 UTF-8 路径、超长路径、
   符号链接、并发写、磁盘满）；是否有未处理的 Result / unwrap / expect。
2. 安全：是否存在 shell 拼接、路径逃逸、token 泄漏到日志、XSS 风险、越权插件调用。
3. 架构一致性：是否违反分层（domain 无 IO、前端不直连 FS、写操作必经快照与审计）。
4. 性能：是否在热路径做了全量扫描/O(n²)/不必要的克隆；是否缺少虚拟化或分页。
5. 测试：新增逻辑是否有对应测试；测试是否断言了真实行为（而非仅"不 panic"）。
6. i18n 与无障碍：文案是否走 i18n；交互是否可键盘完成；是否仅靠颜色传达信息。
7. 合规：是否触碰 AGENTS.md 的红线（R1–R8）。
按严重程度排序输出，并为每条给出修复建议与工作量估计。
```

### OPS-5 系统化修 Bug

```text
【OPS-5 修 Bug】
用户报告的问题：<描述>
请按系统化调试流程处理，不要直接改代码：
1. 复现：写出最小复现步骤，并在本地复现成功（记录实际现象与期望现象的差异）。
2. 定位：通过日志/断点/二分方式定位到具体代码位置，说明"根本原因"而非表象。
3. 影响面：这个问题还会在哪些场景出现（列出同类调用点）。
4. 修复：给出最小修复，避免顺手重构。
5. 回归测试：新增一个能捕获该问题的自动化测试（先确认它在修复前失败、修复后通过）。
6. 验证：运行质量门禁 + 相关的 E2E。
7. 回报：若这是同类问题的第 2 次出现，请补充一个"防止复发"的措施（lint 规则、类型约束或测试基建）。
```

### OPS-6 里程碑收口验收

```text
【OPS-6 里程碑收口】
当前里程碑：<M0..M8>
请执行收口验收，不要新增功能：
1. 逐条对照 docs/PLAN.md 中该里程碑的「验收标准」，给出「通过 / 不通过 / 无法验证」三态结论，
   每条必须附**可核查的证据**（命令 + 输出摘要 / 测试名 / 截图说明 / 基准数据）。
2. 执行通用出口标准（DoD）全部检查项。
3. 执行 AGENTS.md §4 质量门禁，记录全部命令结果。
4. 统计本阶段新增/修改的测试数量与覆盖率变化。
5. 生成 docs/acceptance/<里程碑>.md（若已有则更新），并在回报中给出"未通过项清单 + 修复计划"。
6. 明确说明：本里程碑结束时仓库是否处于「可运行、可打包」状态。
⚠️ 禁止为了让验收通过而降低标准或删除测试；发现不达标要如实报告。
```

### OPS-7 交互级验收（每个里程碑必做，重点）

```text
【OPS-7 交互级验收】
背景：历史上多次出现"视觉正常、lint 通过，但交互实际坏掉"的回归。
因此本项目的验收**不能只看渲染与静态检查**，必须做交互级验收。
请针对本里程碑新增/修改的每个界面，逐项执行并记录结果：
1. 悬停与命中：每个可交互元素（节点、按钮、图标、列表项）是否真的能被鼠标命中？
   是否存在被遮挡（z-index / overlay / pointer-events）导致"看得见点不到"的情况？
   逐一用 Playwright 的 elementFromPoint 或 hover + 断言来验证，禁止目测。
2. 点击进入：点击后是否进入预期界面/状态？是否存在点击无反应、重复触发、
   或点击后状态与 UI 不一致的情况？
3. 面板联动：跨面板的联动是否正确（例如选中提交 → 详情面板更新 →
   文件列表更新 → diff 更新；切换仓库 → 所有面板同步切换而不是残留旧数据）。
4. 状态一致性：UI 显示的标识是否与真实数据一致（尤其是名称/ID 映射：
   例如节点 id 与内部标记名不一致会导致"看着对但拿不到数据"）。
   请为每个"名称/ID 映射"写一条断言测试。
5. 空态/错误态/加载态：三种非正常态是否都有合理呈现且不崩溃？
6. 键盘可达：不点鼠标能否完成主要流程？
7. 未捕获错误：全部交互结束后 window.__errs 必须为空（这是必过项）。
8. 跨引擎：在 Windows（WebView2）、macOS（WKWebView）、Linux（WebKitGTK）三处各验证一次
   与渲染/交互相关的重点界面（至少覆盖提交图、diff、三栏冲突编辑器）。
输出：docs/acceptance/<里程碑>-interaction.md，逐项「通过/不通过 + 证据（测试名或录屏说明）」，
并对不通过项立即修复（修复后重跑）。
```

### OPS-8 依赖升级与安全修复

```text
【OPS-8 依赖维护】
1. 运行 cargo outdated / pnpm outdated，列出可升级项。
2. 分类处理：
   - 安全漏洞（cargo audit / pnpm audit）：必须升级，若无法升级则给出隔离方案并创建 Issue；
   - 补丁与次版本：可升级，升级后跑全量测试；
   - 主版本（含破坏性变更）：单独 PR，说明迁移成本，不在同一 PR 混入其他改动。
3. 升级后必须：跑全量质量门禁 + E2E 的 P0 用例 + 手动验证受影响的核心流程。
4. 更新 docs/LICENSE-AUDIT.md 与 NOTICE（若依赖许可变化）。
5. 在回报中给出"升级清单 + 风险点 + 是否建议合入"。
约束：不得为了升级而顺手重构业务代码；不得引入付费依赖。
```

### OPS-9 文档同步

```text
【OPS-9 文档同步】
检查并同步文档与实现的一致性：
1. docs/API.md 中的命令清单 vs 实际注册的 Tauri commands（写脚本比对，输出差集）。
2. docs/PLAN.md 第 4 章功能表的状态 vs 实际实现状态（为每个功能标注：未开始/进行中/已完成/已变更）。
3. docs/manual/ 中的截图与步骤 vs 当前 UI（描述性检查，标注过时项）。
4. i18n 术语表 vs 新增文案术语是否一致。
5. CHANGELOG 是否覆盖本阶段全部用户可见变更。
6. README 的功能列表 vs 实际可用功能（不得宣传尚未实现的功能）。
输出差异清单并补齐；若发现文档描述的方案已变更，请在 PLAN 中以 ADR 形式记录决策。
```

### OPS-10 性能回归排查

```text
【OPS-10 性能回归】
现象：<指标退化描述，例如"10 万提交仓库首屏渲染从 1.8s 退化到 5.2s">
1. 用基准脚本复现并确认数据（至少 3 次取中位数），给出前后对照表。
2. 二分定位：用 git bisect（对性能基准脚本）或逐个排除最近改动，定位引入点。
3. 分析根因（给出证据：profiling、火焰图、耗时打点分布），不要凭猜测。
4. 修复并验证：修复后基准回到基线 ±5% 内。
5. 新增防护：为该项指标加上 CI 门禁（nightly 对比基线，退化 > 10% 失败）。
6. 若确认是"必要的性能代价"（例如新增了某个正确性保证），请提出 ADR 并调整基线数值，说明理由。
```

### OPS-11 每日/每周进度汇报

```text
【OPS-11 进度汇报】
请汇报当前进度，格式固定：
1. 已完成（本轮）：任务 ID + 一句话结果 + 验证方式。
2. 进行中：任务 ID + 完成度 + 预计剩余。
3. 阻塞：任务 ID + 阻塞原因 + 需要人类做什么（若无写"无"）。
4. 风险预警：发现的新风险或偏差（含对工期的影响估计）。
5. 需要审批的事项：按 §2.3 格式列出（若无写"无"）。
6. 下轮计划：接下来要执行的 1–3 个任务 ID。
要求：≤ 40 行；数据要具体（测试通过数、覆盖率、耗时），禁止空泛表述。
```

---

## 6. 人类审批点总表

> 除下列审批点外，代理可自行决策并继续。总数约 **24 个**，全流程你的总投入预计 **6–12 小时**。

| 阶段 | 审批点 | 你要做的事 | 预计耗时 |
| --- | --- | --- | --- |
| 开工前 | 待决策项 | 确认 `docs/PLAN.md` §15.6 的 D-01 ~ D-10（已给出建议，可全部采纳） | 20 min |
| 开工前 | 仓库 | 创建 GitHub 公共仓库、开启分支保护、添加 Secrets 占位 | 15 min |
| M0 | T0.1 | 提供仓库地址与组织名 | 2 min |
| M0 | T0.3 | 确认原创图标方案（红线 R2） | 10 min |
| M0 | T0.4 | 确认主界面布局原创性（红线 R3） | 10 min |
| M0 | T0.9 | 审阅 AGENTS.md 与 README 免责声明文案 | 10 min |
| M0 | T0.10 | 在 GitHub 设置分支保护规则 | 5 min |
| M0 | KO-0 | 批准 M0 执行提案 | 10 min |
| M1 | T1.2 | 确认 Git 读引擎选型（若差分测试发现严重不一致） | 10 min |
| M1 | KO-1 | 批准 M1 执行提案 | 10 min |
| M2 | T2.2 | 确认提交图视觉方案（原创性） | 10 min |
| M2 | T2.6 | 是否提供真实 GitHub 测试仓库（可拒绝，用本地 bare 仓库替代） | 5 min |
| M2 | KO-2 | 批准 M2 执行提案 | 10 min |
| M3 | T3.2 | 确认三栏冲突编辑器交互方案 | 15 min |
| M3 | T3.6 | 确认可视化 rebase 交互方案 | 15 min |
| M3 | T3.8 | 确认快照磁盘占用阈值（200MB / 2GB） | 5 min |
| M3 | KO-3 | 批准 M3 执行提案（本阶段风险最高） | 15 min |
| M4 | T4.3 | 创建 GitHub OAuth App 并提供 Client ID | 10 min |
| M4 | T4.8 | 首次真实 PR 合并时确认（避免误操作） | 3 min |
| M4 | KO-4 | 批准 M4 执行提案 | 10 min |
| M5 | T5.1 | PTY Spike 结论与降级方案（若失败） | 10 min |
| M5 | T5.3 | 确认终端危险命令的默认拦截级别 | 5 min |
| M5 | KO-5 | 批准 M5 执行提案 | 10 min |
| M6 | T6.1 | 确认 wasmtime 体积影响是否可接受 | 5 min |
| M6 | T6.3 | 确认插件面板 UI 呈现方式（A/B/C） | 10 min |
| M6 | T6.8 | 是否需要支持企业自建 CA | 3 min |
| M6 | KO-6 | 批准 M6 执行提案 | 10 min |
| M7 | T7.1 | macOS 无公证下自动更新的 Spike 结论与降级方案 | 10 min |
| M7 | T7.6 | 遥测方案决策（建议：完全不做） | 5 min |
| M7 | T7.7 | 审阅用户文档准确性与措辞 | 20 min |
| M7 | T7.10 | 批准 v1.0.0 发布（展示 Release Notes 与产物） | 15 min |
| M7 | KO-7 | 批准 M7 执行提案 | 10 min |
| M8 | T8.2 | 是否申请 SignPath 免费签名计划 | 5 min |
| M8 | T8.3 | 执行第三方仓库提交（Winget / Flathub / Snap / AUR / Homebrew tap） | 30 min |
| M8 | T8.5 | 确认 GPG 密钥生成与离线备份方案 | 15 min |
| M8 | T8.6 | 审阅全部推广素材的合规性与文案 | 30 min |
| M8 | T8.7 | 是否建即时社区（建议：仅 Discussions） | 5 min |
| M8 | T8.8 | 确认 15 个 good first issue 内容 | 20 min |
| M8 | KO-8 | 批准 M8 执行提案 | 10 min |

**其他常规操作（每个里程碑结束时）**：
- 执行 OPS-6（里程碑收口验收）+ OPS-7（交互级验收）→ 你只需看结论与未通过项。
- 执行 OPS-11（进度汇报）→ 你只需花 5 分钟浏览。

---

## 7. 一键执行顺序（拷贝顺序）

> 按此顺序逐条复制提示词。括号内为「审批点」，标 ✅ 的表示该任务需要你先批准或提供信息。

```text
【准备阶段】
  0. 确认 docs/PLAN.md §15.6 的 10 项待决策                                    ✅
  1. 创建 GitHub 公共仓库 + 分支保护 + Secrets 占位                              ✅
  2. KO-0                                                                      ✅

【M0 地基】约 3 周（代理执行）
  3. T0.1 ✅(提供仓库地址)   4. T0.2   5. T0.3 ✅(图标)   6. T0.4 ✅(布局)
  7. T0.5   8. T0.6   9. T0.7   10. T0.8   11. T0.9 ✅(免责声明)   12. T0.10 ✅(分支保护)
  13. T0.11   14. T0.12
  15. OPS-6（M0 收口）+ OPS-7（交互级验收）

【M1 Git 核心闭环】约 6 周
  16. KO-1 ✅
  17. T1.1 → T1.12（依次执行；T1.2 可能需审批 ✅）
  18. OPS-6 + OPS-7

【M2 历史 / 分支 / 同步】约 6 周
  19. KO-2 ✅
  20. T2.1 → T2.10（T2.2 视觉 ✅、T2.6 测试仓库 ✅）
  21. OPS-6 + OPS-7

【M3 冲突 / Rebase / 快照（差异化核心）】约 7 周
  22. KO-3 ✅
  23. T3.1 → T3.11（T3.2 ✅、T3.6 ✅、T3.8 阈值 ✅）
  24. OPS-6 + OPS-7
  ★ 建议：此处可发布 v0.3 早期预览版（约 22 周节点），开始收集反馈
       → 执行 OPS-6 + OPS-7 + docs/PLAN.md §15.3「发布前检查清单」，
         并用 T7.3 的 beta 通道做简化版（只发 GitHub Releases，暂不做自动更新）

【M4 GitHub 集成】约 6 周
  25. KO-4 ✅
  26. T4.1 → T4.12（T4.3 OAuth App ✅、T4.8 合并确认 ✅）
  27. OPS-6 + OPS-7

【M5 终端 / 诊断 / 编辑器】约 5 周
  28. KO-5 ✅
  29. T5.1（先 Spike，可能需审批 ✅）→ T5.10（T5.3 拦截级别 ✅）
  30. OPS-6 + OPS-7

【M6 插件 / 主题 / 平台】约 6 周
  31. KO-6 ✅
  32. T6.1 → T6.10（T6.1 体积 ✅、T6.3 方案 ✅、T6.8 CA ✅）
  33. OPS-6 + OPS-7

【M7 发布工程】约 5 周
  34. KO-7 ✅
  35. T7.1（先 Spike ✅）→ T7.10（T7.6 遥测 ✅、T7.7 文档 ✅、T7.10 发布 ✅）
  36. OPS-6 + OPS-7 + OPS-9（文档同步）
  ★ v1.0.0 发布

【M8 零成本加固 / 分发 / 社区】持续
  37. KO-8 ✅
  38. T8.1 → T8.8（T8.2 SignPath ✅、T8.3 第三方提交 ✅、T8.5 GPG 密钥 ✅、
      T8.6 素材审阅 ✅、T8.7 社区 ✅、T8.8 Issue 确认 ✅）
  39. OPS-6 + OPS-7
  ★ 社区启动

【日常（贯穿全程）】
  - 每个任务结束：代理按 AGENTS.md §5 回报；你只需看「需要人类审批」段落
  - 每周：OPS-11（进度汇报）
  - 有 Bug：OPS-5
  - 感觉跑偏：OPS-2
  - 代理卡住：OPS-3（你按 §2.3 回复决定）
  - 每月：OPS-8（依赖维护）+ OPS-9（文档同步）
```

**你的最小介入模式（懒人模式）**：
```text
如果只想做最少的事：
1. 一次性批准 §15.6 的 10 项决策（全部采纳建议）。
2. 每次代理请求审批时，回复"批准你的建议"。
3. 每个里程碑结束时执行 OPS-6 + OPS-7，只看「未通过项」。
4. v1.0.0 发布前做一次真实体验验收。
其余全部交给代理。
```

---

## 附：与计划书的对应关系

| 本文件 | 对应 `docs/PLAN.md` |
| --- | --- |
| §1.3 零成本约束 | §12.3、M8 章节、§8.5 |
| §2 通用模板 | §0.2 AI 编码代理执行公约 |
| §3 KO 提示词 | §7 里程碑计划（各里程碑的"目标/交付物/任务分解"） |
| §4 逐任务提示词 | §7 各里程碑的"给编码 Agent 的提示词"（本文件为其完整展开版） |
| §5 OPS-6/7 | §10 测试与质量、§15.4 里程碑验收清单 |
| §6 审批点总表 | §7.0 DoD、§12.4 执行模型 |
| 合规相关约束 | §9 合规与知识产权（红线 R1–R8） |

---

*文档结束。全部 95 条任务提示词 + 9 条阶段启动提示词 + 11 条运维提示词，共 115 条，可直接按 §7 的顺序逐条执行。*

