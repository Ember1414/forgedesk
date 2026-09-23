# ForgeDesk IPC API 契约

> 本文件是**前后端唯一契约来源**。AGENTS.md §4 规定：每新增一个 Tauri Command
> 必须在此登记（能力等级、参数、返回、错误码），且参数必须在后端二次校验。
>
> 前端只能通过 `src/lib/ipc/index.ts` 调用命令（该文件之外禁止 import `@tauri-apps/api/*`，
> 由 ESLint 架构护栏强制）。

---

## 1. 通用约定

| 约定 | 说明 |
| --- | --- |
| 命名 | `<domain>_<action>`，如 `repo_open`、`git_commit_execute` |
| 返回 | 一律 `AppResult<T>`（即 `Result<T, AppError>`），**不允许**返回裸 `Result<T, String>` |
| 参数命名 | Rust 侧 `snake_case` → 前端 `camelCase`（Tauri 自动转换；DTO 字段用 `serde(rename_all = "camelCase")`） |
| 输入校验 | 所有外部输入（路径、URL、sha、分支名）在后端**二次校验**；前端校验只为即时反馈 |
| 长任务 | 超过 500ms 的操作必须走 `JobRunner` 并返回任务 id，进度通过事件推送（M1 起） |
| 审计 | 任何改变仓库状态的操作必须经 `SnapshotManager` + `AuditLog`（红线 R7，M1 起） |
| 能力等级 | `ReadOnly` / `Mutating` / `Network` / `Dangerous`（见 PLAN §5.12）；前端据等级决定是否需要确认对话框 |

### 1.1 错误形状（`AppError`）

所有错误都归一化为同一结构（Rust：`crates/domain/src/error.rs`；前端：
`src/lib/errors.ts` 的 `normalizeError`）：

```ts
interface AppError {
  code: ErrorCode;          // 稳定错误码（见下表），前端据此做 i18n
  message: string;          // 开发者可读描述（英文），不作为用户可见标题
  detail?: string;          // 原始细节，**已脱敏**（红线 R8）
  hint?: string;            // 后端给的针对性建议（缺省时前端用错误码的兜底建议）
  actions: FixAction[];     // 可点击的修复动作
  retryable: boolean;       // 是否建议重试
}

interface FixAction {
  id: string;
  labelKey: string;         // i18n key，如 errors.actions.refresh
  command: string;          // 点击后调用的 Tauri 命令
  args?: Record<string, unknown>;
}
```

**错误码清单**（只增不改；新增时同步 `src/lib/errors.ts` 的 `ERROR_CODES` 与
`src/lib/i18n/locales/*/errors.json`，由 `src/lib/errors.test.ts` 断言三者一致）：

`PATH_NOT_REPO`、`GIT_CONFLICT`、`AUTH_REQUIRED`、`AUTH_EXPIRED`、`PERMISSION_DENIED`、
`NOT_FOUND`、`VALIDATION`、`NETWORK`、`RATE_LIMITED`、`PATCH_APPLY_FAILED`、`PLAN_STALE`、
`HOOK_REJECTED`、`RESTORE_VERIFY_FAILED`、`KEYRING_UNAVAILABLE`、`STORAGE`、`PTY_UNSUPPORTED`、
`UNSUPPORTED_BY_ENGINE`、`INTERNAL`

**转换入口**：命令层不得自行拼装错误，一律经 `forgedesk_commands::error::to_app_error`——
错误分类在领域层（`ErrorCode::classify`，纯函数可单测），脱敏在 `forgedesk-diagnostics`
（`sanitize_log`）。用户可见文案**永远由前端按 `code` 走 i18n**，后端不产出中英文界面文案。

---

## 2. 已登记命令

| 命令 | 能力等级 | 里程碑 | 说明 |
| --- | --- | --- | --- |
| [`app_version`](#app_version) | ReadOnly | T0.1 | 应用版本与构建信息 |
| [`settings_get`](#settings_get--settings_set--settings_all) | ReadOnly | T0.7 | 读取单个设置项 |
| [`settings_set`](#settings_get--settings_set--settings_all) | Mutating | T0.7 | 写入（覆盖）设置项 |
| [`settings_all`](#settings_get--settings_set--settings_all) | ReadOnly | T0.7 | 读取某个范围的全部设置 |
| [`debug_throw_error`](#debug_throw_error) | ReadOnly | T0.6 | 触发受控失败，用于验证错误链路（**仅开发构建注册**） |

---

### app_version

返回应用版本与构建信息。用途：验证 IPC 通路；同时让用户在提 Issue 时能一键提供
版本 + 提交号 + 平台信息。

- **能力等级**：`ReadOnly`（无参数、无副作用）
- **参数**：无
- **返回**：

```ts
interface AppVersion {
  version: string;   // 语义化版本，如 "0.0.1"
  gitSha: string;    // 构建时的提交短哈希；不在 git 仓库中构建时为 "unknown"
  target: string;    // 形如 "x86_64-windows"
  profile: string;   // "debug" | "release"
}
```

- **错误**：正常路径不产生错误（失败即 `INTERNAL`）
- **前端封装**：`src/lib/ipc/index.ts` 的 `appVersion()`
- **调用点**：`src/features/system/VersionBadge.tsx`

---

### settings_get / settings_set / settings_all

本地设置读写。存储在应用数据目录的 SQLite `settings` 表中（见 `crates/storage`）。

**共同参数**

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `scope` | `"global"` \| `"repo"` | 是 | 设置归属范围 |
| `repoId` | `number` | scope=repo 时必填 | 仓库级设置对应的仓库 id；**缺失即报错**，不静默降级为全局 |

**值的契约**：`value` 一律是 **JSON 字符串**（由调用方序列化）。存储层不理解具体类型，
因此新增设置项不需要改后端、不需要迁移。`settings_set` 会校验 JSON 形状，
非法值返回 `VALIDATION`——在入口拦住比将来读取时 `JSON.parse` 抛错更容易定位。

**全局设置的唯一性**：`repo_id` 为 `NULL`，而 SQLite 中 `NULL` 在唯一约束里互不相等，
因此唯一性由迁移里的 `COALESCE(repo_id, -1)` 索引保证（详见 `0001_init.sql` 的注释）。

#### settings_get

- **能力等级**：`ReadOnly`
- **返回**：`string | null`（不存在返回 `null`，不是错误）
- **错误**：`VALIDATION`（scope 非法 / repo 缺 repoId）、`STORAGE`

#### settings_set

- **能力等级**：`Mutating`（只写本地配置，不涉及仓库状态，因此**不需要**快照）
- **返回**：`null`
- **错误**：`VALIDATION`（scope 非法 / key 为空 / value 不是合法 JSON）、`STORAGE`

#### settings_all

- **能力等级**：`ReadOnly`
- **返回**：`Record<string, string>`（key → JSON 字符串；键序稳定，便于对比与排查）
- **错误**：`VALIDATION`、`STORAGE`
- **用途**：应用启动时一次性拉取，避免逐个 key 往返

**前端封装**：`settingsGet` / `settingsSet` / `settingsAll`（`src/lib/ipc/index.ts`）
**调用点**：`src/stores/settingsStore.ts`（`load` / `setJson`）、`src/features/settings/GeneralSettingsPage.tsx`

---

### debug_throw_error

触发一个受控失败的演示错误。存在的理由：错误链路（后端分类 → 脱敏 → IPC → 前端 i18n →
Toast → 动作按钮）是基础设施，它坏掉时不会有任何业务功能报错，只会在真正出错那天集体失效，
因此需要一个可以随时触发的入口供端到端验证与将来的 E2E 测试使用。

- **能力等级**：`ReadOnly`（不读写仓库、不访问网络；仅构造并返回一个错误）
- **注册范围**：**仅 debug 构建**（`src-tauri/src/main.rs` 按 `cfg(debug_assertions)` 分流）。
  发布产物里不存在该命令——能让应用主动报错的入口没有理由出现在用户机器上。
- **参数**：

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `code` | `string` | 是 | 错误码字符串（大小写不敏感，见 §1.1 清单） |

- **返回**：永远返回 `Err`（这是它的用途）
- **错误**：
  - 传入合法错误码 → 返回该错误码的 `AppError`，并附带：
    - `detail`：含**假令牌**的原始输出（用于验证脱敏：界面上应显示 `ghp_«redacted»`）
    - `actions`：一个动作 `{ labelKey: "errors.actions.refresh", command: "app_version" }`（只读、无害）
  - 传入未知错误码 → `VALIDATION`。原始输入只进 `detail`（折叠展示、经脱敏），
    不进 `message`——避免把用户输入拼进会写入日志/通知的标题级字段。
- **前端封装**：`src/lib/ipc/index.ts` 的 `debugThrowError(code)`
- **调用点**：`src/ui/__dev__/ComponentsPage.tsx` 的「错误链路（AppError）」区块（仅开发构建）

---

## 3. 新增命令的检查清单

1. 命令定义在 `crates/commands/src/<domain>.rs`（**不要**定义在 `lib.rs`，见该文件顶部说明），
   并由 `lib.rs` 重导出；
2. 参数在后端二次校验，非法输入返回 `VALIDATION`；
3. 错误经 `to_app_error` 转换，不自行拼文案；
4. 在本文件登记：能力等级、参数表、返回结构、可能的错误码、前端封装名、调用点；
5. 补单测：正常路径 + 至少一个非法输入路径；
6. 若涉及仓库写操作，确认已接入 `SnapshotManager` + `AuditLog`（M1 起）。
