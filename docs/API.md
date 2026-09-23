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
  hint?: string;            // 只放**数据**（路径/命令/URL），不写建议性散文（见 CODING_STYLE §2.1）
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
`UNSUPPORTED_BY_ENGINE`、`CANCELLED`、`INTERNAL`

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
| [`logs_open`](#logs_open) | ReadOnly | T0.8 | 在系统文件管理器中打开日志目录 |
| [`logs_tail`](#logs_tail) | ReadOnly | T0.8 | 读取末尾若干行日志（已脱敏） |
| [`repo_discover`](#repo_discover) | ReadOnly | T1.3 | 从任意目录向上发现仓库 |
| [`repo_open`](#repo_open) | ReadOnly | T1.3 | 打开仓库：审计 + 版本检查 + 登记 |
| [`repo_clone`](#repo_clone) | Network | T1.3 | 克隆仓库（长任务，返回 jobId） |
| [`repo_init`](#repo_init) | Mutating | T1.3 | 初始化仓库（可生成 .gitignore / LICENSE） |
| [`repo_recent_list`](#repo_recent_list) | ReadOnly | T1.3 | 最近打开的仓库 |
| [`repo_forget`](#repo_forget) | Mutating | T1.3 | 从最近列表移除（不删磁盘文件） |
| [`repo_close`](#repo_close) | ReadOnly | T1.3 | 关闭仓库（结束会话内的"已打开"状态） |
| [`job_cancel`](#job_cancel) | ReadOnly | T1.3 | 取消一个正在运行的长任务 |
| [`debug_throw_error`](#debug_throw_error) | ReadOnly | T0.6 | 触发受控失败，用于验证错误链路（**仅开发构建注册**） |
| [`debug_panic`](#debug_panic) | ReadOnly | T0.8 | 触发真实 panic，用于验证崩溃留档（**仅开发构建注册**） |

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

### logs_open

在系统文件管理器中打开日志目录。

- **能力等级**：`ReadOnly`（不读写仓库、不修改数据；只是打开一个文件夹）
- **参数**：无
- **返回**：`null`
- **错误**：`NOT_FOUND`（目录不存在）、`INTERNAL`（缺少平台打开命令，例如 Linux 上没有 `xdg-open`）。
  错误里会带上**日志目录的绝对路径**，用户可以自己打开它——这类失败原因无法从错误码推断，
  但用户完全可以自助。
- **前端封装**：`logsOpen()`；调用点：设置 → 高级、日志对话框

---

### logs_tail

读取末尾若干行日志，按时间线顺序（旧 → 新）返回。

- **能力等级**：`ReadOnly`
- **参数**：

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `lines` | `number` | 否 | 需要的行数；缺省 200，被夹在 `1..=2000`（上限用于防止一次 IPC 拉走整个日志文件） |

- **返回**：`LogLine[]`

```ts
interface LogLine {
  timestamp: number | null;  // Unix 毫秒；无法解析时为 null
  level: string | null;      // INFO / WARN / ERROR…
  target: string | null;     // 产生日志的模块
  message: string;           // 消息正文
  raw: string;               // 整行原文（已脱敏），用于粘贴到反馈里
}
```

- **错误**：`STORAGE`（日志文件读不到）
- **脱敏**：返回的每一行都经过 `sanitize_log`。**读取路径同样是出境路径**，
  不能因为"写入时脱敏过"就放行——文件里可能混入历史版本写入的内容或第三方库的原始输出。
- **前端封装**：`logsTail(lines?)`；调用点：`src/features/logs/LogViewer.tsx`

---

### repo_discover

从任意目录向上查找仓库（含裸仓库）。**不写库、不审计**。

- **能力等级**：`ReadOnly`
- **参数**：

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `path` | `string` | 是 | 任意目录；可在仓库的子目录里。前后空白会被裁掉，空值与含 NUL 的值返回 `VALIDATION` |

- **返回**：`Repository`

```ts
interface Repository {
  workdir: string | null;        // 裸仓库为 null
  gitDir: string;
  isBare: boolean;
  isEmpty: boolean;              // 还没有任何提交
  head: string | null;           // HEAD 的分支短名；游离 HEAD / 空仓库为 null
  detached: boolean;
  upstream: string | null;
  defaultBranch: string | null;  // origin/HEAD 优先，其次当前分支
  isShallow: boolean;            // 浅克隆：历史不完整，历史视图需降级提示
  isLfs: boolean;                // 提示性字段，不参与数据完整性判断
  worktrees: readonly Worktree[]; // 主工作区在首位
  branchLabel:                 // 由后端判定，前端不自行拼
    | { kind: 'unborn' }
    | { kind: 'detached' }
    | { kind: 'named'; name: string };
}

interface Worktree {
  path: string;
  head: string | null;
  branch: string | null;
  detached: boolean;
  isBare: boolean;
  locked: boolean;
  prunable: boolean;
}
```

- **错误**：`PATH_NOT_REPO`（附一个 `repo_init` 动作与路径 `hint`）、`VALIDATION`
- **用途**：用户拖入子目录时先告诉界面"它属于哪个仓库、当前分支是什么"，
  再由界面决定是否真的打开
- **前端封装**：`repoDiscover(path)`；调用点：`src/ui/__dev__/RepositoryLifecyclePanel.tsx`

---

### repo_open

打开仓库：发现 → 仓库配置审计 → git 版本检查 → 登记到最近列表。

- **能力等级**：`ReadOnly`（只读仓库、只写本地登记表；不改仓库状态，因此不需要快照）
- **参数**：同 `repo_discover`
- **返回**：`OpenedRepository`

```ts
interface OpenedRepository {
  recordId: number;              // repositories 表主键；后续所有 repoId 参数都用它
  repository: Repository;
  audit: RepoAudit;
  gitVersion: string | null;     // 如 "2.54.0.windows.1"
  gitVersionSupported: boolean;  // 与最低版本 2.30 比较；版本未知时为 true
  needsGitUpgrade: boolean;
}

interface RepoAudit {
  findings: readonly AuditFinding[];
  hasDanger: boolean;            // 存在"会被 git 当命令执行"的配置
  maxSeverity: 'info' | 'warning' | 'danger' | null;
}

interface AuditFinding {
  id: string;      // fsmonitor | ssh_command | filter_clean | filter_smudge
                   // | filter_process | shell_alias | pager | editor | hooks_path
  severity: 'info' | 'warning' | 'danger';
  key: string;     // 命中的配置键，便于用户去 git config 里定位
  value: string;   // 命中的配置值（**已脱敏**，红线 R8）
  scope: string;   // local | worktree
}
```

- **错误**：`PATH_NOT_REPO`（附 `repo_init` 动作）、`VALIDATION`、`STORAGE`
- **三条约定**：
  1. **审计与版本检查不阻塞打开**：读不到 `.git/config` 时返回空报告而不是失败
     （否则用户会以为仓库坏了）；
  2. **只审计仓库级范围**（`local` / `worktree`）：`global` / `system` 是用户自己
     机器上的选择，报成警告只会训练用户忽略警告；
  3. **打开过程不执行任何 hook、不刷新索引**：只跑 `rev-parse` / `symbolic-ref` /
     `worktree list` / `config --list` / `--version`，全部是只读查询
     （由 `tests/repository_lifecycle.rs` 断言"打开后 `.git/index` 仍不存在"）。
- **前端封装**：`repoOpen(path)`；调用点：`src/ui/__dev__/RepositoryLifecyclePanel.tsx`

---

### repo_clone

克隆仓库（**长任务**）。

- **能力等级**：`Network`（访问远端并写本地磁盘）
- **参数**：`spec: CloneRequest`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `url` | `string` | 是 | HTTPS / SSH / `file://`；含空白或控制字符返回 `VALIDATION` |
| `into` | `string` | 是 | 目标目录；**已存在且非空时提前返回 `VALIDATION`**（不等传输结束） |
| `depth` | `number` | 否 | `--depth`；`0` 返回 `VALIDATION` |
| `branch` | `string` | 否 | `--branch`；按 `check-ref-format` 的关键规则校验 |
| `bare` | `boolean` | 否 | `--bare` |
| `recurseSubmodules` | `boolean` | 否 | `--recurse-submodules` |
| `singleBranch` | `boolean` | 否 | `--single-branch`（只取单个分支的引用） |

- **返回**：`{ jobId: string }`（立即返回，不等克隆完成）
- **进度与结果**：`job:progress` / `job:done`（`result` 是 `OpenedRepository`）/
  `job:failed`（`error` 是 `AppError`）；可经 `job_cancel` 取消
- **错误**：`VALIDATION`、`NETWORK`、`AUTH_REQUIRED`、`AUTH_EXPIRED`、
  `PERMISSION_DENIED`、`CANCELLED`
- **前端封装**：`repoClone(spec)`；调用点：`src/ui/__dev__/RepositoryLifecyclePanel.tsx`

---

### repo_init

初始化仓库，并按需生成 `.gitignore` / `LICENSE`。

- **能力等级**：`Mutating`（在磁盘上创建仓库并写文件；**不创建提交**，
  因此没有可回滚的既有状态，不需要快照）
- **参数**：`spec: InitRequest`

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `path` | `string` | 是 | 目标目录（不存在时创建） |
| `initialBranch` | `string` | 否 | `init -b`；缺省跟随用户的 `init.defaultBranch` |
| `bare` | `boolean` | 否 | 创建裸仓库 |
| `gitignore` | `string` | 否 | `rust` / `node` / `python` / `go` / `java`；未知值返回 `VALIDATION` 并列出支持项 |
| `license` | `string` | 否 | `MIT` / `Apache-2.0` / `BSD-3-Clause`；未知值同上 |
| `licenseHolder` | `string` | 否 | 版权持有者；会被清理成单行，缺省写 `<copyright holder>` |
| `licenseYear` | `number` | 否 | 版权年份；缺省取当前年份 |

- **返回**：`OpenedRepository`
- **错误**：`VALIDATION`、`STORAGE`、`INTERNAL`
- **三条约定**：
  1. **只生成文件，不替用户做决定**：不推荐许可证、不改 `Cargo.toml` / `package.json`
     里的许可声明；
  2. **只写新文件**：已存在的 `.gitignore` / `LICENSE` 一律不动（覆盖属于数据丢失）；
  3. **裸仓库 + 模板 → `VALIDATION`**：裸仓库没有工作区，静默跳过会让用户以为文件生成了。
- **前端封装**：`repoInit(spec)`；调用点：`src/ui/__dev__/RepositoryLifecyclePanel.tsx`

---

### repo_recent_list

最近打开的仓库（按 `last_opened_at` 倒序，最近在前）。

- **能力等级**：`ReadOnly`
- **参数**：

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `limit` | `number` | 否 | 条数；缺省 50，被夹在 `1..=200` |

- **返回**：`RecentRepository[]`

```ts
interface RecentRepository {
  id: number;                     // repositories 表主键
  path: string;                   // 规范化后的路径（已解析符号链接、去掉 Windows \\?\ 前缀）
  name: string;                   // 展示名；裸仓库去掉 .git 后缀
  defaultBranch: string | null;
  lastOpenedAt: number | null;    // Unix 毫秒
  createdAt: number;              // Unix 毫秒
  isOpen: boolean;                // 当前会话中是否已打开
}
```

- **错误**：`STORAGE`
- **前端封装**：`repoRecentList(limit?)`；调用点：`src/ui/__dev__/RepositoryLifecyclePanel.tsx`

---

### repo_forget

从最近列表移除一个仓库（**只删记录，不碰磁盘上的仓库**）。

- **能力等级**：`Mutating`（改本地登记表）
- **参数**：`repoId: number`
- **返回**：`null`
- **错误**：`NOT_FOUND`（id 不在列表里）、`STORAGE`
- **前端封装**：`repoForget(repoId)`

---

### repo_close

关闭一个已打开的仓库（结束会话内的"已打开"状态）。

- **能力等级**：`ReadOnly`（不改数据库、不碰仓库，只改本进程内的会话状态）
- **参数**：`repoId: number`
- **返回**：`null`
- **错误**：`NOT_FOUND`
- **语义**：与 `repo_forget` 的区别是**记录仍在列表里**，只是不再标记为已打开。
  重复关闭是幂等的。"哪些仓库正开着"是后端的事实（T1.10 起决定监听哪些目录），
  因此不放在前端 store 里。
- **前端封装**：`repoClose(repoId)`

---

### job_cancel

请求取消一个正在运行的长任务。

- **能力等级**：`ReadOnly`（只改本进程内的任务状态，不碰仓库、不碰网络）
- **参数**：`jobId: string`
- **返回**：`boolean` —— 它此前是否在运行。未知或已结束的 id 返回 `false`
  而**不是**报错：界面据此显示"任务已结束"，比一个错误提示更有用。
- **取消的可见结果**：任务以 `job:failed` 结束，`error.code` 为 `CANCELLED`
  （不是 `INTERNAL`——取消是用户主动的、预期内的结果）。
- **前端封装**：`cancelJob(jobId)`

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

### debug_panic

触发一次**真实的 panic**（在独立的后台线程里）。

- **能力等级**：`ReadOnly`（不改任何数据；只是让一个后台线程崩掉）
- **注册范围**：**仅 debug 构建**（同 `debug_throw_error`）。release 使用 `panic = "abort"`，
  崩溃由 M7/T7.5 的崩溃恢复处理——会话标记与 panic 文件正是为它准备的。
- **参数**：无
- **返回**：`null`（命令本身成功返回；崩溃发生在另一个线程）
- **用途（T0.8 验收）**：
  1. 后台线程崩溃、界面继续可用（验证隔离）；
  2. 日志目录出现 `panic-<时间戳>.log`（含消息、线程名、版本与调用栈，**已脱敏**）；
  3. 不清理 `session.lock` 直接退出应用，下次启动日志里会出现"上次会话未正常退出"。
- **前端封装**：`debugPanic()`；调用点：`src/ui/__dev__/ComponentsPage.tsx`（开发页面）

---

## 3. 事件登记表

前端通过 `listen('<event>')` 订阅（封装在 `src/lib/ipc/`，组件不直接 import `@tauri-apps/api/event`）。

**命名约定**：`<域>:<动作>`，全小写、冒号分隔、用连字符连接多词（`git:state-changed`）。
域与命令的 `<domain>` 保持一致，便于按域检索。

| 事件 | 载荷 | 用途 | 首个落地任务 | 状态 |
| --- | --- | --- | --- | --- |
| `job:progress` | `{ jobId, phase, current, total, message? }` | 长任务进度（>500ms 的操作必须走 `JobRunner`） | T1.3 / M1 | ✅ `crates/commands/src/jobs.rs` |
| `job:done` | `{ jobId, result }` | 长任务成功结束 | T1.3 / M1 | ✅ `crates/commands/src/jobs.rs` |
| `job:failed` | `{ jobId, error: AppError }` | 长任务失败结束（错误形状同 §1.1） | T1.3 / M1 | ✅ `crates/commands/src/jobs.rs` |
| `repo:changed` | `{ repoId, paths: string[] }` | 文件监听触发刷新 | T1.10 / M1 | ⬜ 未实现 |
| `git:state-changed` | `{ repoId, opState }` | 仓库正处于 rebase/merge/cherry-pick 中途 | T2.x / M2 | ⬜ 未实现 |
| `term:output` | `{ termId, bytes }` | 终端输出流 | T5.x / M5 | ⬜ 未实现 |
| `auth:expired` | `{ accountId }` | 令牌失效，提示重新登录 | T4.x / M4 | ⬜ 未实现 |
| `update:available` | `{ version, notes }` | 发现新版本 | T7.3 / M7 | ⬜ 未实现 |

**`job:*` 的实现约定**（T1.3 起）：

- 事件名与载荷形状由 `crates/commands/src/jobs.rs` 的 `TauriJobReporter` 决定，
  `forgedesk-jobs` 本身不依赖 Tauri（它只定义 `JobReporter` 出口）；
- `phase` 是**稳定短名**（`counting` / `compressing` / `receiving` / `resolving` /
  `writing` / `ref-update` / `other`），界面按它选 i18n 文案；
- `message` 是 git 的原始进度行，**已在 `ProgressSink` 入口脱敏**（红线 R8）——
  远端可以在 `remote:` 行里回显带凭据的 URL；
- 终态事件由 `JobRunner` 统一投递，任务体不得自己投递
  （否则会出现"任务体报成功、注册表还留着"这类不一致）；
- 取消的任务以 `job:failed` 结束，`error.code` 为 `CANCELLED`。

**事件与命令的边界**：

- 命令用于"前端发起、需要结果"的调用；事件用于"后端主动告知、可能多次发生"的推送。
- 事件**不携带用户可见文案**（`message?` 字段只放阶段标识或数据），文案仍由前端按 key 渲染。
- 事件载荷必须可序列化且**已脱敏**；进度事件不得包含文件内容或凭据。
- 事件必须在注册的同时考虑**退订**（组件卸载时 `unlisten`），否则泄漏到全局监听器集合里。

> 落地顺序说明：M0 阶段没有任何事件（当前只有命令），
> 上面这张表是 M1 起的契约；每落地一个就在 `状态` 列改为 ✅ 并补上对应实现位置。

---

## 4. 新增命令的检查清单

1. 命令定义在 `crates/commands/src/<domain>.rs`（**不要**定义在 `lib.rs`，见该文件顶部说明），
   并由 `lib.rs` 重导出；
2. 参数在后端二次校验，非法输入返回 `VALIDATION`；
3. 错误经 `to_app_error` 转换，不自行拼文案；
4. 在本文件登记：能力等级、参数表、返回结构、可能的错误码、前端封装名、调用点；
5. 补单测：正常路径 + 至少一个非法输入路径；
6. 若涉及仓库写操作，确认已接入 `SnapshotManager` + `AuditLog`（M1 起）。
