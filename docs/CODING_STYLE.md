# ForgeDesk 编码风格

> 规则分三类，**必须区分对待**：
>
> - **强制**：由 lint / 测试 / CI 自动拦截，违反即失败。
> - **约定**：评审关注，没有自动拦截（写在这里是为了让后来者知道"为什么"）。
> - **禁止**：触碰红线（见 `../AGENTS.md` §2）或已被事故验证过的坏做法。
>
> 优先级：`../AGENTS.md` > 本文件 > `PLAN.md`。

---

## 1. 通用

| 项 | 规则 | 类别 |
| --- | --- | --- |
| 文档与注释语言 | 中文（本项目主力语言）；代码标识符、提交信息、错误 `message` 用英文 | 约定 |
| 注释内容 | 解释**为什么**（取舍、事故、边界），不复述"这行做了什么" | 约定 |
| 文件头 | 每个模块顶部有一段模块说明：职责、边界、以及关键的"为什么这么设计" | 约定 |
| 行尾与缩进 | LF、2 空格（前端）/ 4 空格（Rust，rustfmt 决定）；由 `.editorconfig` + prettier + rustfmt 统一 | 强制 |
| 提交粒度 | 一个提交一件事；能独立回滚 | 约定 |

**为什么注释要写"为什么"**：本仓库的历史事故（`.gitignore` 误吞 crate、JSON 日志漏脱敏）
损失都来自"看起来没问题"的代码。只写"做什么"的注释对下一个读代码的人零价值，
而一句"因为 Windows rename 不覆盖已有文件，所以这里要解决重名"能直接阻止重犯。

---

## 2. Rust

### 2.1 错误处理

| 规则 | 类别 |
| --- | --- |
| 命令层返回 `AppResult<T>`（`Result<T, AppError>`）；**禁止** `Result<T, String>` | 强制（`docs/API.md` 约定 + 评审） |
| 命令层不得自行拼错误，一律经 `forgedesk_commands::to_app_error` | 约定 |
| 生产代码禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `dbg!()` | 强制（workspace lints，CI 用 `-D warnings`） |
| 测试模块内可用 `unwrap/expect/panic`，但必须显式 `#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` | 强制 |
| 忽略返回值必须显式（`let _ = ...`），`unused_must_use` 是错误 | 强制 |
| `#![forbid(unsafe_code)]` 在每个 crate 的 `lib.rs` | 强制 |
| 公开项必须有文档注释（rustdoc），`missing_docs = warn` | 强制 |

**错误文案的分工**（T0.9 起统一，此前 `platform`/`storage` 有中英混用）：

| 字段 | 语言 | 用途 | 是否用户可见 |
| --- | --- | --- | --- |
| `code` | 枚举 | 前端据此走 i18n | — |
| `message` | **英文** | 开发者读；写 issue / 日志时可直接检索 | 间接（详情区） |
| `detail` | 英文/原始输出 | 原始细节（已脱敏），例如 `stderr`、路径 | 间接（详情区） |
| `hint` | **只放数据**（路径、命令、URL），不写建议性散文 | 补足上下文 | 是（原样显示） |

**为什么 `hint` 不许写散文**：后端不知道界面当前是中文还是英文，写中文建议会让英文界面里
突然冒出中文；而"建议"本身是可枚举的（保存失败、磁盘满、权限不足…），
因此建议性文案统一由前端按 `code` 提供，后端只用 `hint` 补数据。

### 2.2 命名

| 对象 | 风格 | 示例 |
| --- | --- | --- |
| 模块 / 文件 | `snake_case` | `logging.rs`、`sanitizing_writer.rs` |
| 类型 / trait | `UpperCamelCase` | `RotatingWriter`、`GitEngine` |
| 函数 / 变量 | `snake_case` | `should_rotate`、`cursor` |
| 常量 / 静态 | `SCREAMING_SNAKE_CASE` | `CURRENT_LOG_FILE`、`MAX_TAIL_LINES` |
| 错误码 | `UpperCamelCase` 枚举 + 稳定字符串 | `ErrorCode::PlanStale` → `PLAN_STALE` |
| 测试函数 | `snake_case` **英文句子**，描述被保证的行为 | `rotating_twice_within_the_same_second_produces_distinct_files` |

测试名的写法要求：读起来像一句断言（"在什么条件下，什么必然成立"），
而不是 `test_rotate_2()`。失败时只看名字就该知道坏掉了什么。

### 2.3 日志（`tracing`）

| 规则 | 类别 |
| --- | --- |
| 禁止 `println!` / `eprintln!`（生产代码）；用 `tracing` | 强制（`print_stdout`/`print_stderr` lint） |
| 结构化字段优先：`info!(repo_id = %id, "opened repository")`，便于 `logs_tail` 过滤 | 约定 |
| 级别：`error!` 用户可见失败 / `warn!` 可恢复异常（自动降级、清理失败）/ `info!` 生命周期（启动、迁移、操作完成）/ `debug!` 排查细节 | 约定 |
| 日志文案可用中文（面向维护者），但**不得**作为界面文案来源 | 约定 |
| 不得把令牌、密码、私钥、完整 URL 凭据写入日志；脱敏在写入层兜底，但不能故意写 | 强制（红线 R8） |
| 每条日志都要能被"没有上下文的人"读懂：带上对象标识（路径、repo id、命令名） | 约定 |

### 2.4 测试

| 规则 | 类别 |
| --- | --- |
| 新增领域逻辑必须有单测；纯函数优先（`domain` 不允许 IO，因此天然可测） | 强制（覆盖率底线见 `../AGENTS.md` §4） |
| 禁止在单测里访问真实网络与用户主目录 | 强制 |
| 临时文件必须用 `std::env::temp_dir()` + 进程内唯一名，用后清理 | 约定 |
| 时间相关逻辑要能"构造时间"而不是"等时间流逝"（例如把 `now` 作为参数传入） | 约定 |
| 端到端断言优于逐层单测：涉及格式/编码/脱敏的链路，必须有"真实组件 + 断言最终输出"的用例 | 强制（T0.8 的 JSON 脱敏泄漏就是靠它发现的） |

---

## 3. TypeScript / React

### 3.1 目录与命名

| 对象 | 位置 | 风格 |
| --- | --- | --- |
| 功能域 | `src/features/<域>/` | 一个页面一个文件，域内组件就近放 |
| 通用组件 | `src/ui/components/` | 文件 `kebab-case.tsx`，组件 `PascalCase` |
| hooks | 与使用者同层，文件名 `useXxx.ts` | `camelCase` |
| 状态 | `src/stores/`（Zustand）、TanStack Query（服务端状态） | — |
| IPC | **只能**在 `src/lib/ipc/index.ts` 调用命令 | — |
| 开发专用页面 | `src/ui/__dev__/`（不进入生产构建） | — |

### 3.2 组件结构

```tsx
import { ... } from '外部依赖';
import { ... } from '@/内部别名';        // 别名只允许 @/

/** 模块/组件说明：职责与关键取舍。 */
export interface FooProps { ... }        // props 显式导出

export function Foo({ ... }: FooProps) { // 具名导出，不用 default
  ...
}
```

| 规则 | 类别 |
| --- | --- |
| 具名导出；禁止 `export default`（便于重命名与静态分析） | 约定 |
| props 用 `readonly` 字段的 interface 显式声明并导出 | 约定 |
| `exactOptionalPropertyTypes` 已开启：可选 prop 用条件展开 `{...(x === undefined ? {} : { x })}`，不要传 `undefined` | 强制（类型检查拦截） |
| 只导出"外部真的要用"的东西；内部辅助函数不导出（测试需要时在文件头注明原因） | 约定 |
| 状态为空的界面用 `PlaceholderPage` 并**标注归属任务号**（`T1.4`），避免被误认为已完成 | 约定 |

### 3.3 状态归属（单一真相源）

| 状态类型 | 归属 | 例子 |
| --- | --- | --- |
| 服务端/仓库状态 | TanStack Query | 工作区状态、提交历史、远程仓库列表 |
| 纯 UI 状态 | Zustand（`src/stores/`） | 侧栏折叠、主题、面板位置、当前仓库 id |
| 组件局部状态 | `useState` | 输入框内容、展开/收起 |
| Git 状态 | **禁止**进 Zustand（`../AGENTS.md` §6） | 由 Query 从后端取 |

### 3.4 i18n

| 规则 | 类别 |
| --- | --- |
| 所有用户可见文案走 `t('key')`；**禁止**硬编码中文或英文界面文案 | 强制（`pnpm i18n:lint`） |
| 命名空间：`common`（通用词）、`shell`（外壳与页面框架）、`errors`（错误标题与建议）；新增域时在 `src/lib/i18n/index.ts` 注册 | 约定 |
| key 命名：小驼峰 + 层级（`settings.advanced.logsTitle`），同级同义复用而不是复制 | 约定 |
| 中英 key 必须一一对应且无空文案 | 强制（`src/lib/i18n/i18n.test.ts`） |
| 例外豁免：在文件头注释 `// i18n-ignore-file`（整文件）或行尾 `// i18n-ignore`（单行），并写明理由 | 约定 |
| 开发者页面（`src/ui/__dev__/`）的说明文字可豁免（不进入生产构建） | 约定 |

### 3.5 视觉与可访问性

| 规则 | 类别 |
| --- | --- |
| **禁止硬编码颜色**（`#fff`、`rgb(...)`、Tailwind 调色板类如 `bg-slate-800`）；只能用 `src/ui/tokens.css` 的语义工具类（`bg-surface`、`text-fg-muted`、`border-line`） | 强制（评审 + `check:contrast` 覆盖 token 层） |
| 新增语义色必须同时：加 token、加明暗两套、通过 `pnpm check:contrast`（WCAG AA） | 强制 |
| 图标按钮必须有可访问名称（`IconButton label` 必填）；纯装饰图标 `aria-hidden` | 强制 |
| 交互必须只有一种实现：同一能力不要在两处各写一份（临时实现要在文件头标注并被后续任务替换） | 约定 |
| 键盘可达：对话框/菜单支持 `Esc` 关闭并把焦点交回触发元素；折叠后仍要保留无障碍名称（`sr-only`） | 约定 |

### 3.6 前端测试

| 规则 | 类别 |
| --- | --- |
| 新增核心组件/状态必须有 Vitest 用例 | 强制 |
| `it` 名称用**中文句子**描述用户可见行为（与 Rust 的英文测试名相反，原因：前端断言面向界面行为，中文更贴近评审语言） | 约定 |
| 断言优先用 `getByRole` + 可访问名称与可见文本，避免断言 class 或内部状态（`aria-current`、`disabled` 等语义属性属于例外） | 约定 |
| 不 mock 被测组件内部的子组件；只 mock IPC/网络边界（`@/lib/ipc`、`fetch`） | 约定 |
| 涉及 store 的断言：每个用例前复位（`initialXxxState`），并在 `afterEach` 先 `cleanup()` 再复位，避免 act 之外的更新噪音 | 约定 |

---

## 4. 提交信息

格式：**Conventional Commits**，subject 英文祈使句，正文讲"为什么"。

```text
feat(logging): M0 T0.8 -- file logging, rotation, panic capture and log viewing

Until now logs existed only in a terminal. That is useless on the day a user
reports a problem: the window is closed and the evidence is gone.

- File logs are JSON Lines so timestamp, level and target survive the round trip.
- Rotation resolves name collisions instead of failing: Windows rename does not
  overwrite an existing target, so two rotations within the same second would
  otherwise silently stop rotating.
```

| 规则 | 类别 |
| --- | --- |
| 类型：`feat` / `fix` / `chore` / `docs` / `test` / `refactor` / `perf` / `ci` | 强制（评审） |
| scope 用模块名（`logging`、`app-shell`、`ci`） | 约定 |
| 正文至少包含：**动机**（为什么现在做）与**关键取舍**（为什么不选另一条路） | 约定 |
| 破坏性变更用 `!` 标记并在正文说明迁移方式 | 约定 |

---

## 5. 相关文档

- 架构与依赖规则：[`ARCHITECTURE.md`](./ARCHITECTURE.md)
- IPC 契约（命令与事件）：[`API.md`](./API.md)
- 本地环境、陷阱与排查：[`DEV-ENV.md`](./DEV-ENV.md)
- 参与方式：[`../CONTRIBUTING.md`](../CONTRIBUTING.md)
