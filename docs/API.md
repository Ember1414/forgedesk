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

`PATH_NOT_REPO`、`GIT_CONFLICT`、`CONFLICT_UNRESOLVED`、`AUTH_REQUIRED`、`AUTH_EXPIRED`、`PERMISSION_DENIED`、
`NOT_FOUND`、`VALIDATION`、`NETWORK`、`RATE_LIMITED`、`PATCH_APPLY_FAILED`、`PLAN_STALE`、
`HOOK_REJECTED`、`PUSH_REJECTED`、`EMPTY_COMMIT`、`RESTORE_VERIFY_FAILED`、`KEYRING_UNAVAILABLE`、`STORAGE`、
`PTY_UNSUPPORTED`、`UNSUPPORTED_BY_ENGINE`、`CANCELLED`、`INTERNAL`

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
| [`workspace_status`](#workspace_status) | ReadOnly | T1.4 | 读取工作区状态（分组、分支头、操作状态） |
| [`workspace_stage`](#workspace_stage--workspace_unstage--workspace_discard--workspace_reveal) | Mutating | T1.4 / T1.6 | 暂存路径 / 块 / 行（成功后发布 repo:changed） |
| [`workspace_unstage`](#workspace_stage--workspace_unstage--workspace_discard--workspace_reveal) | Mutating | T1.4 / T1.6 | 取消暂存路径 / 块 / 行 |
| [`workspace_discard`](#workspace_stage--workspace_unstage--workspace_discard--workspace_reveal) | Mutating | T1.4 / T1.6 | 放弃工作区修改（前端必须先确认；快照 M3 接入） |
| [`workspace_reveal`](#workspace_stage--workspace_unstage--workspace_discard--workspace_reveal) | ReadOnly | T1.4 | 在系统文件管理器中显示文件 |
| [`commit_prepare`](#commit_prepare) | ReadOnly | T1.7 | 生成提交计划（不创建提交） |
| [`commit_execute`](#commit_execute) | Mutating | T1.7 | 执行提交计划（成功后发布 repo:changed） |
| [`commit_message_hint`](#commit_message_hint) | ReadOnly | T1.7 | 最近提交与分支风格提示（纯本地规则） |
| [`commit_amend_context`](#commit_amend_context) | ReadOnly | T1.8 | amend 语境：上一次提交信息 + 是否可能已推送 |
| [`commit_hooks_list`](#commit_hooks_list) | ReadOnly | T1.8 | 仓库里的钩子清单（仅展示） |
| [`snapshot_list`](#snapshot_list) | ReadOnly | T1.9 | 快照列表（新的在前） |
| [`snapshot_diff`](#snapshot_diff) | ReadOnly | T1.9 | 快照与当前状态的差异摘要（含未跟踪内容三分类） |
| [`snapshot_usage`](#snapshot_usage) | ReadOnly | T3.8 | 磁盘占用、配额与孤儿目录 |
| [`snapshot_estimate`](#snapshot_estimate) | ReadOnly | T3.8 | 下一次快照会备份多少未跟踪内容（危险操作对话框用） |
| [`snapshot_create`](#snapshot_create) | Mutating | T3.8 | 手动打点；返回内容备份的实情（体积 / 跳过 / 告警） |
| [`snapshot_restore`](#snapshot_restore) | Mutating | T1.9 | 回滚到快照（成功后发布 repo:changed） |
| [`snapshot_prune`](#snapshot_prune) | Mutating | T1.9 | 按保留策略清理旧快照 |
| [`snapshot_cleanup`](#snapshot_cleanup) | Mutating | T3.8 | 立即清理：孤儿目录 + 保留策略 + 总占用回收 |
| [`snapshot_restore_pending`](#snapshot_restore_pending) | ReadOnly | T3.9 | 未完成的回滚（崩溃恢复：上次回滚被强杀时留下） |
| [`snapshot_restore_abandon`](#snapshot_restore_abandon) | Mutating | T3.9 | 清除未完成回滚的标记（不回退任何东西） |
| [`audit_list`](#audit_list--audit_export--audit_prune) | ReadOnly | T1.11 | 分页查询操作历史（可按仓库 / 类型 / 时间筛选） |
| [`audit_export`](#audit_list--audit_export--audit_prune) | ReadOnly | T1.11 | 导出操作历史到临时文件（CSV / JSON），返回路径 |
| [`audit_prune`](#audit_list--audit_export--audit_prune) | Mutating | T1.11 | 按保留策略清理旧记录 |
| [`git_log_page`](#git_log_page) | ReadOnly | T2.1 | 提交历史分页 + 泳道布局 |

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

（已落地的事件见下表；`repo:changed`于 T1.4 引入。）

| 事件 | 载荷 | 用途 | 任务 | 状态 |
| --- | --- | --- | --- | --- |
| `repo:changed` | `{ repoId, kind, paths }` | 仓库数据已变化（应用自己的写操作，或文件监听发现的**外部**变化） | T1.4 / T1.10 | ✅ 已实现 |

**`repo:changed` 的载荷（T1.10 起带 `kind`）**：

```ts
interface RepoChangedPayload {
  repoId: number;
  /**
   * 变化类别，决定前端失效哪些查询（见 `src/lib/repoChanged.ts`）：
   *   - workspace：文件内容或暂存区变了；
   *   - refs：HEAD / 分支 / 引用变了（提交、回滚、外部 checkout）；
   *   - large：一个合并窗口内的变化量超过阈值，路径不再逐条列举。
   */
  kind: 'workspace' | 'refs' | 'large';
  /** 涉及的路径（**相对仓库根**；`large` 时为空）。 */
  paths: string[];
}
```

两条纪律：

- **路径是相对路径**：绝对路径里带着用户名与目录结构（隐私，红线 R8），
  而且前端要拿它与状态面板里的路径比对；
- **应用自己的操作与文件监听发同一个事件**：两边若各发一套，前端就得为
  "谁发的"写两套判断。因此 `kind` 的取值也共用（`WatchKind`）。

### 计划中的事件（未实现）


前端通过 `listen('<event>')` 订阅（封装在 `src/lib/ipc/`，组件不直接 import `@tauri-apps/api/event`）。

**命名约定**：`<域>:<动作>`，全小写、冒号分隔、用连字符连接多词（`git:state-changed`）。
域与命令的 `<domain>` 保持一致，便于按域检索。

| 事件 | 载荷 | 用途 | 首个落地任务 | 状态 |
| --- | --- | --- | --- | --- |
| `job:progress` | `{ jobId, phase, current, total, message? }` | 长任务进度（>500ms 的操作必须走 `JobRunner`） | T1.3 / M1 | ✅ `crates/commands/src/jobs.rs` |
| `job:done` | `{ jobId, result }` | 长任务成功结束 | T1.3 / M1 | ✅ `crates/commands/src/jobs.rs` |
| `job:failed` | `{ jobId, error: AppError }` | 长任务失败结束（错误形状同 §1.1） | T1.3 / M1 | ✅ `crates/commands/src/jobs.rs` |

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

### workspace_status

读取工作区状态（按面板分组预拆分：staged / unstaged / untracked / conflicted / ignored）。

- **能力等级**：ReadOnly
- **参数**：`repoId: number`（存储层记录 id）、`includeIgnored?: boolean`（默认 false）
- **返回**：`StatusReportDto`（branch / operation / 五个分组 / ignoredCount）
- **错误**：`NOT_FOUND`（记录不存在）、`STORAGE`（git 失败）
- **前端封装**：`workspaceStatus(repoId, includeIgnored?)`

### workspace_stage / workspace_unstage / workspace_discard / workspace_reveal

- **能力等级**：stage / unstage / discard = `Mutating`；reveal = `ReadOnly`
- **参数**：`repoId` + `spec`（粒度判别联合，见下）+ `view?`（补丁查看参数）
- **返回**：`null`；成功后发布 `repo:changed` 事件（payload `{ repoId, paths }`）
- **前端封装**：`workspaceStage / workspaceUnstage / workspaceDiscard / workspaceReveal`；调用点：`src/features/workspace/WorkspaceStatusPage.tsx`、`src/features/diff/DiffView.tsx`

#### 粒度（`spec.kind`）

```ts
type StageScope =
  | { kind: 'files'; paths: string[] }
  | { kind: 'hunks'; path: string; hunkIndices: number[] }
  | { kind: 'lines'; path: string; selections: { hunkIndex: number; lines: number[] }[] };

// discard 的整文件粒度需要区分两类路径：前者 git 可恢复，后者只能从磁盘删除
type DiscardScope =
  | { kind: 'files'; tracked: string[]; untracked: string[] }
  | { kind: 'hunks'; path: string; hunkIndices: number[] }
  | { kind: 'lines'; path: string; selections: { hunkIndex: number; lines: number[] }[] };

interface PatchView {
  contextLines?: number;      // `-U<n>`，缺省 3
  ignoreWhitespace?: boolean; // `-w`，缺省 false
  detectRenames?: boolean;    // `-M`，缺省 true
}
```

#### 两个通道（T1.6 的关键设计）

| 粒度 | 通道 | 为什么 |
| --- | --- | --- |
| `files` | `git add` / `git reset` | 更快，且能处理未跟踪文件与模式变更（它们没有可裁剪的补丁） |
| `hunks` / `lines` | `git diff` → 裁剪 → `git apply --cached [--reverse]` | `git add` 的最小粒度是文件，只有补丁通道能表达"这个文件里只暂存这几行" |

补丁通道固定"先 `--check` 再应用"，两次用**同一份字节**；`--check` 失败时不写任何东西，
因此失败后仓库状态一定与调用前一致。`discard` 的块级 / 行级用同一份补丁反向应用到**工作区**
（不碰索引：已暂存的内容保留）。

- **`view` 必须与打开 diff 时用的参数一致**：hunk 的划分取决于上下文行数，
  参数不同会让"第 2 块"在后端对应到另一块。后端据此重新生成补丁并对下标做越界校验，
  宁可返回错误也不会"照着错位的下标写内容"。
- **`hunkIndex` / `lines` 的下标**与该文件 `workspace_diff` 返回的 `hunks[].lines` 一一对应
  （`lines` 是行在该 hunk `lines` 数组中的位置，0 基，上下文行与 `\ No newline` 标记也参与计数）。
- **限制**：
  - 二进制文件只允许整文件粒度（`VALIDATION`）；
  - 未跟踪文件没有补丁（`git diff` 不含它们），只允许整文件粒度，行级会返回 `VALIDATION`；
  - 不支持"选取一行的一部分"（那需要字符级 hunk，属于 M5）；
  - 重命名是文件级属性：该文件只要有块被暂存 / 取消暂存，重命名就一起生效；
  - 新增文件被部分**撤销**暂存时，后端会去掉 `--- /dev/null` 与 `new file mode`（否则
    git 会把它当成"整份删除"）；删除文件被部分**暂存**同理（不会把文件删掉）。
- **错误**：
  - `NOT_FOUND`：仓库记录不存在；
  - `VALIDATION`：路径为空 / 含 NUL / 选择超过 4096 项 / hunk 或行下标越界 /
    该侧没有变更 / 二进制文件用了行级；
  - `PATCH_APPLY_FAILED`：补丁被 git 拒绝（界面上的 diff 已经不是仓库现在的样子）。
    `detail` 是 git 的原始 stderr（已脱敏），`hint` 是本次实际使用的 git 开关，
    `actions` 里带一个指向 `workspace_status` 的刷新动作（`args.repoId` 已填好），
    因为这类失败的唯一有效修复是"刷新状态再选一次"；
  - `GIT_CONFLICT`：冲突路径不可放弃（M3 处理）。

### commit_prepare

生成提交计划（**不创建提交**）。这是红线 R7"计划预览 → 快照 → 执行 → 可回滚"里的第一步：
用户先把"将要提交什么、会执行哪个命令、会跑哪些钩子"看清楚，再由 [`commit_execute`](#commit_execute) 落地。

- **能力等级**：`ReadOnly`。它只读索引与配置，不创建提交、不写审计、不发事件——界面因此
  不需要任何确认对话框。实现上会执行 `git write-tree`（会往对象库写一个树对象，gc 会回收），
  但**不动索引、引用与工作区**：能力等级说的是"是否改变用户的数据"，不是"是否碰了磁盘"。
- **参数**：`repoId: number`、`spec: PrepareCommitRequest`

```ts
interface PrepareCommitRequest {
  message: string;              // 提交信息**首行**（subject）
  description?: string;         // 正文；空字符串与缺省等价
  amend?: boolean;              // 默认 false
  amendMode?: 'includeStaged' | 'messageOnly'; // 默认 includeStaged（T1.8）
  signOff?: boolean;            // 默认 false（--signoff；与 GPG 签名是两件事）
  noVerify?: boolean;           // 默认 false（跳过钩子必须由用户显式选择）
  sign?: 'auto' | 'yes' | 'no'; // 默认 auto（跟随仓库/全局配置）
  author?: { name: string; email: string }; // 覆盖作者身份（amend 保留原作者时用）
}
```

**`amendMode` 的两种语义（T1.8）**：`git commit --amend` 提交的是**当前索引**，
所以"只改提交信息"并不是它的默认行为：

| 取值 | 行为 | 实现 |
| --- | --- | --- |
| `includeStaged`（默认） | 把暂存区并入上一次提交 | 普通的 `git commit --amend` |
| `messageOnly` | 只替换提交信息，索引内容不进提交 | 在**隔离索引**（`GIT_INDEX_FILE`）上先 `read-tree HEAD`、校验树一致，再 `--amend` |

两者的结果完全不同（"改一个错别字" vs "多提交三个文件"），因此它是一个显式参数
而不是布尔开关；未知取值返回 `VALIDATION`，不静默降级。

- **返回**：`CommitPlanDto`

```ts
interface CommitPlanDto {
  planId: string;                    // 执行时原样回传
  repoId: number;
  files: { path: string; indexStatus: string }[]; // indexStatus：A/M/D/R/U，界面按它分组
  message: string;                   // 完整提交信息（首行 + 空行 + 正文）
  description: string | null;
  author: { name: string; email: string } | null;
  sign: 'auto' | 'yes' | 'no';
  signOff: boolean;
  noVerify: boolean;
  amend: boolean;
  amendMode: 'includeStaged' | 'messageOnly';
  headPushed: boolean;               // HEAD 是否（可能）已在某个远程跟踪分支上
  hooks: string[];                   // 将要执行的钩子（按 git 的调用顺序）
  equivalentCommand: string;         // 可直接粘贴到终端的等价 git 命令
  headOid: string | null;            // 空仓库为 null
  indexFingerprint: string;          // 索引的树 oid（git write-tree）
  createdAtMs: number;
  expiresAtMs: number;               // createdAtMs + 5 分钟
  subject: string;                   // 首行（界面计数器用）
  subjectChars: number;
  warnings: string[];                // 不阻断的建议：'subjectTooLong'
}
```

- **`equivalentCommand` 的形式**：文件数 ≤ 20 时按 POSIX sh 规则生成
  `git commit [--amend] [--signoff] [--no-verify] [--gpg-sign] [--author=...] -m "..." -m "..."`
  （`\`、`"`、`$`、反引号会被转义，换行不转义）；超过 20 个文件时改为两步说明
  （把信息写入文件后用 `-F <message-file>`），因为几百字符的命令没人会去核对。
- **`hooks` 的准确性**：目录由 `git rev-parse --git-path hooks` 解析，因此 `core.hooksPath`
  （husky 默认设置它）会被正确考虑；只列出 `pre-commit` / `prepare-commit-msg` / `commit-msg`
  且带执行位（Windows 上按文件存在）的那些。`post-commit` 不列：它在提交之后运行，
  失败不会让提交失败。
- **错误**：
  - `VALIDATION`：信息为空或只有空白、签名模式未知、作者姓名/邮箱形状不合法、信息超过 IPC 上限；
  - `EMPTY_COMMIT`：没有可提交的内容（索引为空，或索引与 HEAD 相同）。
    归到这个码而不是通用 `VALIDATION`，是为了让界面说"先暂存一些改动"，
    而不是让用户去检查自己写的提交信息；
  - `GIT_CONFLICT`：仓库有未解决的冲突。冲突检查**排在指纹之前**：`git write-tree` 遇到
    未合并条目会直接失败，那样用户拿到的是一句 git 内部报错，而不是真正的原因；
  - `NOT_FOUND`：仓库记录不存在。
- **前端封装**：`commitPrepare(repoId, spec)`；调用点：`src/features/commit/CommitPanel.tsx`

### commit_execute

执行一份提交计划。**能力等级**：`Mutating`；成功后发布 `repo:changed`
（载荷 `{ repoId, paths }`，让状态面板与历史刷新）。

- **参数**：`planId: string`
- **返回**：`CommitOutcomeDto`

```ts
interface CommitOutcomeDto {
  oid: string;
  subject: string;
  snapshotId: number | null;  // M3 之前为 null
  paths: string[];            // 本次提交涉及的路径
}
```

- **执行序列（顺序即语义）**：
  1. 取出计划 —— **取走即失效**，同一 `planId` 只能执行一次；
  2. 有效期校验（TTL 5 分钟）；
  3. **索引指纹校验**：与 `prepare` 时不同即拒绝。界面上的 diff 是几秒前取的，而
     "用户刚在终端里又 `git add` 了一次"是完全正常的用法；不校验就会提交出用户没看过的内容；
  4. 写审计 `operation_records`（begin）；
  5. `SnapshotManager.create("pre-commit")`（M3 / T1.9 之前注入的是"未启用"实现）；
  6. `git commit`（提交信息走 stdin 的 `--file=-`，不落临时文件）；
  7. 审计收尾（exit code、stderr 摘要、快照 id、reversible）。
- **错误**：
  - `PLAN_STALE`：planId 未知或已用过、已过期、索引被外部改过。
    前两者不带动作，第三者带一个指向 `workspace_status` 且已填好 `repoId` 的刷新动作；
  - `HOOK_REJECTED`：钩子拒绝了提交。`detail` 是 git 的**原始输出**（展开即见钩子说了什么），
    `hint` 是钩子名清单。**不返回 `actions`**：`FixAction` 的语义是"点击后调用某个 Tauri 命令"，
    而"查看输出"等于展开 detail、"禁用钩子重试"等于用 `noVerify: true` 重新走
    prepare + execute——两者都不是单命令。伪造一个指向无关命令的按钮，用户点下去只会
    再撞一次错（比没有按钮更糟），因此这两个动作由前端实现；
  - 其余错误原样透传（`GIT_CONFLICT`、`PERMISSION_DENIED`、`STORAGE`…）。
- **审计的诚实标注**：M3 之前没有快照，因此记录里 `snapshot_id` 为 `NULL`、
  `reversible` 为 `false` —— 审计不会假装有快照。`reversible` 的判定就是
  `snapshot_id.is_some()`。
- **前端封装**：`commitExecute(planId)`；调用点：`src/features/commit/CommitPreviewDialog.tsx`

### commit_message_hint

提交信息的风格提示。**能力等级**：`ReadOnly`。

- **参数**：`repoId: number`
- **返回**：

```ts
interface MessageHintDto {
  recentMessages: string[];    // 最近 20 条提交的首行（新 → 旧）
  template: string | null;     // 例如 'feat: '（分支名形如 feat/xxx 时）
  branchStyle: string | null;  // 分支名斜杠前那一段
}
```

- **规则**（**纯本地、无任何模型推理**，红线 R1）：最近 20 条提交的首行；当前分支名形如
  `<prefix>/<rest>` 时给出 `template = "<prefix>: "`。更复杂的推断（Conventional Commits 的
  scope、团队自定义前缀）靠猜只会猜错，而错的提示比没有提示更烦人。
- **用途**：让用户看到"这个仓库习惯怎么写"，**不是替他写**。
- **错误**：`NOT_FOUND`、`STORAGE`。
- **前端封装**：`commitMessageHint(repoId)`；调用点：`src/features/commit/CommitPanel.tsx`

### commit_amend_context

amend 之前需要的上下文。**能力等级**：`ReadOnly`。

界面打开"Amend 上一次提交"时**一次**拿全三样东西：上一次提交的信息（用来预填）、
它是否（可能）已经在远端、以及是哪些远程分支。分成几次问会多几次 IPC 往返，
而且几次之间仓库可能变化——于是会出现"信息来自提交 A、推送状态来自提交 B"这类
自相矛盾的界面。

- **参数**：`repoId: number`
- **返回**：

```ts
interface AmendContext {
  subject: string | null;   // 上一次提交的首行
  body: string | null;      // 上一次提交的正文
  headOid: string | null;   // 上一次提交的 oid
  pushed: boolean;          // 是否**可能**已推送
  pushedRefs: string[];     // 命中的远程跟踪分支短名，如 ['origin/main']
}
```

- **空仓库返回全 `null` / `false`**，不是错误：那是正常状态，界面据此把 amend 开关
  置为不可用即可，让用户对着一句错误发愣没有意义。
- **`pushed` 的语义边界**：判定依据是本地的 `refs/remotes/*`
  （`git for-each-ref --contains HEAD refs/remotes`，见 `crates/git-engine` 的
  `remote_refs_containing`）。它可能过期，也无法判断远端是否仍保有那个对象，
  因此界面文案只能是"**可能**已推送"，据此提示 force-with-lease 的必要性，
  但不能用来断言"一定推过"或"一定没推过"。
- **错误**：`NOT_FOUND`（仓库记录不存在）、`STORAGE`。
- **前端封装**：`commitAmendContext(repoId)`；调用点：`src/features/commit/CommitPage.tsx`

### commit_hooks_list

仓库里的钩子清单（**仅展示，不编辑**）。**能力等级**：`ReadOnly`。

- **参数**：`repoId: number`
- **返回**：`{ name: string; executable: boolean; commitHook: boolean }[]`
- **为什么列全部而不是只列提交相关的三个**：用户来看这个列表，想知道的多半是
  "为什么提交被拒/很慢"或"我装了哪些工具"，`pre-push`、`post-checkout` 同样可能是
  答案；`commitHook` 标出"这次提交会不会跑它"。而"**这一次**提交会执行哪些"
  由 `CommitPlan.hooks` 回答（它只含存在且会被执行的三类）。
- **目录来自引擎**（`git rev-parse --git-path hooks`），因此 `core.hooksPath`
  （husky 默认设置它）会被正确考虑。
- `.sample` 与点文件一律不列：`git init` 会放一批示例进去，它们永远不会被执行。
  `executable` 为假时界面必须如实说明"git 会忽略它"（Unix 看执行位，Windows 看是否存在）。
- **错误**：`NOT_FOUND`、`STORAGE`。
- **前端封装**：`commitHooksList(repoId)`；调用点：`src/features/commit/CommitPage.tsx`

### snapshot_list / snapshot_diff / snapshot_usage / snapshot_estimate / snapshot_create / snapshot_restore / snapshot_prune / snapshot_cleanup

快照与回滚（M1 / T1.9；T3.8 补未跟踪内容备份与磁盘控制；T3.11 把备份范围扩到
"工作区里与 HEAD 不同的已跟踪文件"，并记下 stash 栈与本地分支引用）。这是红线 R7
"计划预览 → 快照 → 执行 → 可回滚"的最后一环：每个快照是一组**可独立校验的
git 事实**（HEAD oid、索引树、自定义 ref 锚点），T3.8 之后还多了一份
**内容逐字节备份**（未跟踪文件 + 未提交的工作区修改）——它落在应用缓存目录，
不污染用户仓库。

**备份的三条纪律**（T3.8）：

1. 先复制到 `.tmp-*` 临时目录、入库后再改名成 `<backup_root>/<repo_id>/<snapshot_id>`，
   于是正式目录"要么完整、要么不存在"（同文件系统内的 rename 是原子的）；
2. 候选内容超过单份上限时**整体跳过并告警**（`untrackedBackupSkipped`），
   绝不"备到上限为止"——半份备份比没有备份更危险，因为用户会以为它是全的；
3. 单仓库总占用超过上限时按 LRU 回收，ref、数据库记录与备份目录**一起删**，
   不留孤儿。备份被清掉后快照本身仍然可用（HEAD 与索引的恢复不依赖它），
   只是未跟踪内容回不来——这一点由 `snapshot_usage` 与回滚报告如实说明。

**默认磁盘策略（待人类确认）**：单份 200 MiB、单仓库 2 GiB；
`0` 表示不限制。被 gitignore 覆盖的文件默认**不备份**（它们通常是构建产物）。

**锚点为什么是自定义 ref**：HEAD 移走之后，没有任何引用指着的提交会被 `git gc`
回收，回滚从此永远失败。`refs/forgedesk/snapshots/<id>` 指向快照时刻的 HEAD 提交，
git 因此不会回收它。**刻意不用** `git reflog` / `HEAD@{n}`：reflog 会被外部操作
改写或清空，不能作为唯一依据。

#### snapshot_list

- **能力等级**：`ReadOnly`
- **参数**：`repoId: number`、`limit?: number`（缺省 50，夹在 `1..=200`）
- **返回**：`{ id, label, kind, headOid, branch, detached, createdAtMs }[]`（新的在前；
  `kind` 是稳定短名：`pre-commit` / `pre-restore` / `manual` / `pre-sync` / `pre-head-move`）
- **错误**：`NOT_FOUND`、`INTERNAL`

#### snapshot_diff

- **能力等级**：`ReadOnly`
- **参数**：`repoId`、`snapshotId`
- **返回**：

```ts
interface SnapshotDiff {
  headChanged: boolean;
  indexChanged: boolean;
  currentHeadOid: string | null;      // 空仓库为 null
  currentIndexTreeOid: string | null; // 索引有未合并条目时为 null（这本身就是"已变化"）
  refMissing: boolean;                // 锚点丢失 = 不可恢复
  // T3.8：未跟踪内容的三分类（界面必须分开说，它们的后果完全不同）
  untrackedRestorable: string[];      // 有备份且当前缺失/内容不同 → 回滚会写回
  untrackedMissing: string[];         // 记录过但没有备份、当前也不存在 → 回滚找不回来
  untrackedExtra: string[];           // 当前有、快照里没有 → 回滚不会删除
}
```

- **用途**：回滚确认框的内容来源。**前端必须在确认前展示它**——
  让用户对一个看不懂的东西说"是"，等于没有闸门。
- **错误**：`NOT_FOUND`（快照不存在或不属于该仓库）、`INTERNAL`

#### snapshot_restore

- **能力等级**：`Mutating`；成功后发布 `repo:changed`
- **参数**：`repoId`、`snapshotId`
- **执行序列（顺序即语义；T3.9 起分阶段记录）**：
  1. **锚点校验**：锚点 ref 不在就直接返回 `NOT_FOUND`——一个字节都不动；
  2. `protection`：给当前状态打 `pre-restore` **保护点**。打不出来就中止回滚：
     宁可不动，不可无保护地动；
  3. `head`：`git reset --hard <head_oid>`；
  4. `index`：`git read-tree <index_tree_oid>` 把索引恢复到快照的树
     （`reset --hard` 只能把索引带到 HEAD 的树，恢复不了"已暂存未提交"的内容）；
  5. `untracked`：按清单把备份目录里的文件写回工作区，
     已逐字节相同的文件跳过（重复回滚是安全的 no-op）；
  6. `verify`：用读引擎核对 HEAD 与索引树——读与写是两条独立实现
     （T1.2 差分测试保证一致），让考生批自己的卷子是无效的。
- **任一步失败**：立即停止后续阶段，然后
  1. 退回保护点 → 结局 `rolledBack`（仓库回到动手之前，用户什么都不用做）；
  2. 退回也失败 → 结局 `emergency`，报告里带**可复制、可执行的恢复指引**
     （`emergency.commands` 是纯 `git ...` 命令，不含占位符；`notes` 给上下文），
     并把"用户想回到的快照 id"与"指引的目标快照 id / 备份目录"一起显式展示。
  退回之后还会用**仓库指纹**（T3.9 的 `RepoFingerprint`：HEAD + 未跟踪集合）
  再核一次，对不上同样降级为 `emergency`：git 命令返回成功不等于状态真的对了。
- **幂等**：对同一快照重复回滚是安全的 no-op（内容一致、HEAD 不动），
  因此崩溃恢复的"继续"直接复用本命令（见 `snapshot_restore_pending`）。
- **返回**：
  `{ restoredSnapshotId, headOid, indexTreeOid, preRestoreSnapshotId, untrackedPaths,
     untrackedRestored, untrackedFailed, untrackedExtra,
     stashRestored, stashFailed, branchesRestored, branchesFailed, verified,
     outcome, stages, reportLines, emergency }`
  - `outcome`：`completed` / `rolledBack` / `emergency`；
  - `stages`：每阶段的结果（`stage` / `ok` / `detail` / `durationMs`）——
    界面据此展示"哪一步成了、哪一步没成、为什么"；
  - `reportLines`：人话报告行（与日志同源）；
  - `emergency`：仅 `emergency` 时非 null。
- **失败不再一律返回 `Err`**：只有"连第一步都没开始"（快照不存在、锚点丢失、
  保护点打不出来）才报错——那些情况下没有任何阶段性事实可说。
  审计里 `rolledBack` / `emergency` 记为**失败**记录，避免操作历史谎报成功。
- **未跟踪内容的边界（界面必须如实转述）**：
  - 快照时刻**被备份过**的文件会写回（内容逐字节一致）；没有备份的（v1 记录、
    超限被跳过、备份目录被清掉）**找不回来**，逐条列在 `untrackedFailed` 里；
  - 当前存在、快照里没有的未跟踪文件**不会被删除**，列在 `untrackedExtra` 里
    交给用户决定；
  - 内容恢复失败**不触发**回退：HEAD 与索引才是回滚的主体，个别文件被占用
    不该把整次回滚推倒重来。`verified` 说的是"内容备份的校验结果"。
- **stash 栈与分支引用（T3.11）**：快照记下栈上每条 stash 的 oid 与本地分支引用，
  回滚把 `drop` / `clear` 丢掉的条目用 `git stash store` 重新登记、把被删的分支
  重建出来——**只补缺失的，绝不动已存在的引用**。这两类只是引用（不复制字节）：
  提交对象被 `gc` 回收之后放不回去，那时 `stashFailed` / `branchesFailed`
  逐条如实列出，不会假装成功。
- **错误**：`NOT_FOUND`（快照不存在 / 锚点丢失）、`INTERNAL`（保护点打不出来等）。
  校验失败不再走错误通道，而是体现在 `outcome` 与 `stages` 里。
- **前端封装**：`snapshotRestore(repoId, snapshotId)`；调用点：`src/features/snapshots/SnapshotsPage.tsx`

#### snapshot_restore_pending

- **能力等级**：`ReadOnly`
- **参数**：`repoId`
- **返回**：`{ snapshotId, stage, startedAtMs } | null`（`null` = 干净）
- **用途**：崩溃恢复。回滚的每个阶段开始前都会把
  `restore_in_progress` / `restore_stage` / `restore_started_at` 写进
  `snapshots` 表，正常结束时清掉——**留下标记就意味着进程被强杀了**。
  界面据此提示"检测到上次回滚未完成（停在'{{阶段}}'），继续 / 清除提示"。
- **错误**：`NOT_FOUND`、`INTERNAL`
- **前端封装**：`snapshotRestorePending(repoId)`；调用点：`src/features/snapshots/SnapshotsPage.tsx`

#### snapshot_restore_abandon

- **能力等级**：`Mutating`；记录审计（`snapshot_restore_abandon`）
- **参数**：`repoId`
- **返回**：被清掉的标记数
- **语义**：**不回退任何东西**——它只把"上次没走完"标记为已处理。
  用户之所以敢选它，是因为界面已经把当时的快照 ID 与停在哪一步摆在他面前了。
- **错误**：`NOT_FOUND`、`INTERNAL`
- **前端封装**：`snapshotRestoreAbandon(repoId)`；调用点：`src/features/snapshots/SnapshotsPage.tsx`

#### snapshot_create

- **能力等级**：`Mutating`（**不改仓库**：它写数据库与备份目录；刻意**不**发布
  `repo:changed`——发了会让历史页白刷一遍）
- **参数**：`repoId`、`label?: string`（展示用：去控制字符、截到 64 字符，空则回落 `manual`）
- **返回**：完整的结果，而不是一个 id——界面据此说明"这次打点包含什么、不包含什么"

```ts
interface SnapshotOutcome {
  id: number;
  backupBytes: number;      // 内容备份的字节总数
  backedUp: number;         // 备份成功的文件数
  untrackedTotal: number;   // 快照时刻的未跟踪文件总数（含未备份的）
  skipped: string[];        // 没有进备份的未跟踪路径
  warnings: SnapshotWarning[];
  pruned: number[];         // 顺手清理掉的旧快照
}

interface SnapshotWarning {
  // kind 是判别字段，其余字段按类型给（扁平结构：界面不必处理缺字段的分支）
  kind: 'untrackedBackupSkipped'   // 超限整体跳过：count / bytes / limit
      | 'untrackedBackupPartial'   // 个别文件复制失败：paths / detail
      | 'backupDirUnavailable'     // 备份目录不可用：detail
      | 'spaceReclaimed'           // 总占用超限的 LRU 回收：removed / freedBytes
      | 'orphansRemoved';          // 清理了崩溃残留：count
  count: number | null;
  bytes: number | null;
  limit: number | null;
  paths: string[];
  detail: string | null;
  removed: number[];
  freedBytes: number | null;
}
```

- **审计**：`snapshot_create`，`args` 带 `label` 与 `kind`；记录关联这份新快照
- **错误**：`NOT_FOUND`（仓库记录不存在）、`INTERNAL`（空仓库没有可锚定的提交、
  写库或写备份目录失败）
- **前端封装**：`snapshotCreate(repoId, label?)`；调用点：`src/features/snapshots/SnapshotsPage.tsx`

#### snapshot_usage

- **能力等级**：`ReadOnly`
- **参数**：`repoId`
- **返回**：
  `{ repoId, snapshotCount, backupBytes, maxSnapshotBytes, maxRepoBytes, orphanDirs }`
  （`orphanDirs` = 磁盘上有、数据库里没有对应记录的备份目录；上限 `0` = 不限制）
- **错误**：`NOT_FOUND`、`INTERNAL`
- **前端封装**：`snapshotUsage(repoId)`；调用点：`src/features/snapshots/SnapshotsPage.tsx`

#### snapshot_estimate

- **能力等级**：`ReadOnly`
- **参数**：`repoId`
- **返回**：
  `{ untrackedCount, untrackedBytes, ignoredCount, ignoredBytes, includeIgnored,
     limitBytes, withinLimit, wouldSkip }`
- **用途**：危险操作对话框在**动手前**告诉用户"这次打点会跳过 N 个未跟踪文件
  （X MB）"，而不是等操作执行完才发现快照里没有它们
- **错误**：`NOT_FOUND`、`INTERNAL`
- **前端封装**：`snapshotEstimate(repoId)`（尚未接线到各危险操作对话框：
  T3.10 的操作历史与回滚入口会统一消费它）

#### snapshot_prune

- **能力等级**：`Mutating`（删除快照记录、锚点 ref 与备份目录——只删一头都会留下
  假快照或永远不被 gc 的孤儿目录）
- **参数**：`repoId`
- **保留策略**：使用内置默认（每仓库 50 条或 30 天，先到者生效）；把策略暴露成
  设置项属于设置页的工作（磁盘阈值已随 T3.8 落地为常量 +  `snapshot_usage` 可见）
- **返回**：`number[]`（被清理的快照 id）
- **错误**：`NOT_FOUND`、`INTERNAL`

#### snapshot_cleanup

- **能力等级**：`Mutating`
- **参数**：`repoId`
- **与 `snapshot_prune` 的分工**：prune 只按保留策略（条数 / 天数）走；cleanup 还会
  ①清掉复制中途崩溃留下的孤儿目录（`.tmp-*` 与"记录已不存在"的目录），
  ②按**总占用上限**做 LRU 回收
- **返回**：`{ orphansRemoved, reclaimed, freedBytes, remainingBytes }`
- **审计**：`snapshot_cleanup`（记录的是结果 DTO：回收清单与释放体积）
- **错误**：`NOT_FOUND`、`INTERNAL`
- **前端封装**：`snapshotCleanup(repoId)`；调用点：`src/features/snapshots/SnapshotsPage.tsx`

### audit_list / audit_export / audit_prune

操作审计（T1.11）。每一次**写操作**都会留一条记录，这三个命令负责把它读出来、
导出、以及按保留策略清理。

- **能力等级**：`audit_list` = `ReadOnly`；`audit_export` = `ReadOnly`（**写临时文件**，
  不改仓库、不改库表）；`audit_prune` = `Mutating`（删除本地记录）
- **记录的形状**（`operation_records` 表）：

| 字段 | 含义 |
| --- | --- |
| `opType` | 操作类型短名（见下表） |
| `repoId` | 仓库 id；全局操作（克隆 / 初始化 / 导出 / 清理）为 `0` |
| `argsJson` | 参数摘要（**已脱敏**、≤ 2KB 的 JSON） |
| `startedAtMs` / `endedAtMs` | 开始与结束时间（Unix 毫秒） |
| `durationMs` | 耗时（由前两者算出，界面显示用） |
| `exitCode` | git 的退出码 |
| `stderrSummary` | 失败摘要（**已脱敏**，优先用 git 的原始输出） |
| `snapshotId` / `reversible` | 关联的快照；`reversible` 的判定就是"有没有快照" |
| `result` | `ok` / `failed` / `running`。**`running` = 只有开始没有收尾**，即"应用崩在写操作中间"，界面必须能一眼看出 |

- **操作类型**：`commit`、`stage`、`unstage`、`discard`、`clone`、`init`、`forget`、
  `close`、`snapshot_restore`、`snapshot_prune`、`snapshot_create`、`snapshot_cleanup`、
  `snapshot_restore_abandon`、`audit_export`、`audit_prune`。

#### audit_list

- **参数**：`repoId?`（缺省 = 全部仓库）、`opType?`（空白 = 不筛）、`fromMs?`、`toMs?`、
  `limit?`（夹在 `1..=500`，缺省 100）、`offset?`
- **返回**：`{ total: number; entries: AuditEntryDto[] }`（新 → 旧；`total` 不受分页影响）
- **错误**：`STORAGE`

#### audit_export

- **参数**：`repoId?`、`opType?`、`format`（`csv` | `json`）、`fromMs?`、`toMs?`
- **返回**：`{ path, rows, format }`
- **文件写到**临时目录**（`forgedesk-audit-<时间戳>.<ext>`），返回路径由界面显示并允许复制。
  "让用户选目录"需要文件对话框插件（M7）；在那之前返回临时路径比假装已保存到用户选的位置诚实。
- **CSV 的开头写 UTF-8 BOM**：没有它 Excel 会按本地代码页解码，中文全变乱码。
  所有字段都引号包裹并转义（`argsJson` 里有换行与逗号）。
- 一次导出超过 50000 条会被拒绝（`VALIDATION`）：那是误点的特征，不是用法。
- **导出本身也会被记录**（`audit_export`）：谁把历史倒出去过是审计的一部分。
- **错误**：`VALIDATION`（格式未知 / 命中条数过多）、`STORAGE`（写文件失败）

#### audit_prune

- **参数**：无
- **返回**：`{ removed, retentionDays, retentionRows }`
- **保留策略**（可配置，缺省 90 天 / 10000 条，两个条件是"或"）：

| 设置键 | 取值 | 缺省 | 后端收敛到 |
| --- | --- | --- | --- |
| `audit.retentionDays` | 数字（天） | `90` | `[1, 3650]` |
| `audit.retentionMax` | 数字（条） | `10000` | `[100, 1000000]` |

  `<= 0` 表示**关掉该条款**（而不是"保留 0 天"——那会清空整张表）。
- **执行时机**：应用启动时自动执行一次（失败只记日志），以及用户点"清理旧记录"时。
  查询路径上不做删除：看一眼历史不该是写操作。
- **清理本身也会被记录**（`audit_prune`）：删历史的人不该是匿名的。
- **错误**：`STORAGE`

#### 拦截点（为什么能保证"每一次写操作都留痕"）

写操作的记录统一发生在**命令层**（`crates/commands/src/audit.rs` 的 `record`）：
这一层同时看得见 IPC 参数、仓库 id 与操作类型，而且每个 `#[tauri::command]`
就是一次"用户按下按钮"。放在服务层要改十几个方法签名，还要处理"服务之间互相调用"
（`StagingService` 会转调 `WorkspaceService`），最后必然是重复记录或者漏记。

唯一的例外是 `commit_execute`：它的参数摘要（提交信息首行、文件数、钩子清单、索引指纹）
只存在于服务内部的计划里，因此那条记录由 `CommitService` 自己写——
但用的是**同一个** `AuditLog` 类型与同一套脱敏/截断规则，不是第二套机制。

**审计失败不会让操作失败**：`begin` 写不进库时只记日志，操作照常执行。
用户要的是提交成功，不是审计写成功。

- **前端封装**：`auditList / auditExport / auditPrune`；调用点：`src/features/settings/AuditHistoryPanel.tsx`
  （设置 → 高级 → 操作历史）

### git_log_page

分页拉取提交历史，并随每页附带**泳道布局**（每个提交落在哪条泳道、什么颜色、与父提交的边怎么连）。
"加载下一页"需要的是（提交、图位置、边）三件套，拆成两次 IPC 会让两次调用之间拿到的数据
不一致——因此查询与布局在**同一个调用**里完成（布局本身是 `domain::history::layout` 纯函数），
返回值由 T2.2 的提交图渲染直接消费。

现状：服务层与布局已落地（`crates/services/src/history.rs`、`crates/domain/src/history/layout.rs`），
**命令已接线**（定义在 `crates/commands/src/history.rs`，已注册到 `src-tauri/src/main.rs` 的 `invoke_handler`）。

- **能力等级**：`ReadOnly`（只读遍历提交图：不改仓库状态、不写数据库，无需快照/审计）
- **参数**：

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `repoId` | `number` | 是 | 仓库记录 id（`repo_open` 返回的 `recordId`） |
| `query` | `HistoryQuery` | 否 | 查询参数对象；缺省 = 第一页、每页 100 条、其余字段为空/false |

`query` 的 12 个字段：

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `revision` | `string \| null` | 否 | 起始引用（分支名、tag、oid）；缺省 HEAD |
| `allBranches` | `boolean` | 否 | 包含所有引用（`--all`） |
| `paths` | `string[]` | 否 | 路径过滤（文件历史）；相对仓库根 |
| `author` | `string \| null` | 否 | 作者过滤：姓名或邮箱的子串，**忽略大小写** |
| `since` | `number \| null` | 否 | 时间下界（Unix 秒，**含**）；与 `until` 构成闭区间 |
| `until` | `number \| null` | 否 | 时间上界（Unix 秒，**含**） |
| `messageContains` | `string \| null` | 否 | 提交信息包含的子串；**字面匹配、区分大小写**（不做正则与模糊） |
| `firstParentOnly` | `boolean` | 否 | 只看 first-parent 链；**同时作用于查询与布局**（两者不一致会画出断头路） |
| `followRenames` | `boolean` | 否 | 跟随重命名（文件历史）；**必须恰好给一条 `paths`**，否则 `VALIDATION`。**仅 CLI 引擎支持**（`git --follow`），libgit2 引擎返回 `UNSUPPORTED_BY_ENGINE` |
| `collapseMergedBranches` | `boolean` | 否 | 折叠已合并分支；缺省 `false`。**尽力而为**：仅末页生效（本轮穷尽全部提交、`nextCursor` 为 `null` 的那次调用）；窗口不完整（后面还有页）时静默回退为普通布局。`firstParentOnly` 下折叠没有呈现对象，同样不生效 |
| `revisions` | `string[]` | 否 | 分支多选（T2.3）：遍历这些 tip 的**并集**；非空时 `revision` 与 `allBranches` 被忽略（前端把三者建模成互斥选项）。修订可以是分支短名或 oid |
| `caseInsensitive` | `boolean` | 否 | `messageContains` 忽略大小写；缺省 `false`（与旧契约一致） |
| `mergesOnly` | `boolean` | 否 | 只显示合并提交（`--merges` 口径） |
| `myCommitsOnly` | `boolean` | 否 | 只显示"我的提交"：服务层把仓库身份（`user.email`，跟随 local→global 解析链）翻译成 author 过滤；**未配置身份时返回 `VALIDATION`**（静默当成"全部提交"会让用户以为开关坏了）。与显式 `author` 同设时本开关**优先**（两个 author 过滤器在 git 里是 OR 关系，叠加会产生意外并集） |
| `pageSize` | `number` | 否 | 每页条数；缺省 100，被夹在 `1..=500`（上限用于防止一次 IPC 拉走全部提交——PLAN §5.5 的 IPC 上限约束）。**越界是钳制，不是报错** |
| `cursor` | `number \| null` | 否 | 游标 = 下一页第一行的全局序号（0 起始）；**首页省略**（语义见下） |

- **返回**：`HistoryPage`

```ts
interface HistoryPage {
  commits: Commit[];         // 本页提交，新 → 旧
  layout: GraphLayout;       // 本页的泳道布局；rows 与 commits 一一对应
  nextCursor: number | null; // 下一页游标；null = 末页（没有更多了）
}

interface GraphLayout {
  rows: GraphRow[];   // 每个提交的图上位置；row 已由服务层平移成**全局行号**（第二页的第一行是全历史的第 cursor 行，不是本页第 0 行）
  edges: GraphEdge[]; // 孩子 → 父的边；父不在本页窗口内（分页边界）时边照发，渲染层据此画"继续向下"的线
  laneCount: number;  // 用到的泳道数（渲染宽度）
}

interface GraphRow {
  oid: string;        // 提交 oid
  lane: number;       // 泳道（0 基，左侧为 0；lane 在生命周期内不变，刷新与翻页后同分支同色）
  row: number;        // 全局行号（0 基）；滚动、迷你地图与跨页选中都按它工作
  colorIndex: number; // 颜色索引 = lane % 8
  isMerge: boolean;   // 是否合并提交（父提交多于一个）
  hidden: boolean;    // 该行属于被折叠的合并分支（默认 false）；行不删除、lane 与边不动，是否隐藏与如何呈现由前端（T2.2）决定
  collapsed: string[]; // 本行为可折叠的 merge 时，记录被折叠分支第二父的 tip oid（默认空数组）
}

interface GraphEdge {
  fromOid: string;  // 孩子（更新的那个，行号在上）
  toOid: string;    // 父（更旧的那个；不在窗口内也发）
  fromLane: number; // 孩子的泳道
  toLane: number;   // 父的泳道
  kind: string;     // straight（主线继续）/ merge（合并支线）/ branch（支线汇入）；serde `#[serde(rename_all = "lowercase")]` 已落实
}
```

DTO 定义在 `crates/services/src/history.rs`（`HistoryQuery` / `HistoryPage`）与
`crates/domain/src/history/layout.rs`（`GraphLayout`）；`Commit`（oid / parents / author /
committer / refs / signature / subject / body）定义在 `crates/domain/src/git/commit.rs`，
本文档不重复展开。字段名按 §1 通用约定走 camelCase（`HistoryQuery` 的 serde `rename_all = "camelCase"`
已落实，全字段带 `#[serde(default)]`；`GraphRow` 已派生 serde，
`hidden` / `collapsed` 带 `#[serde(default)]`，旧调用方缺这两个字段时向后兼容；
`EdgeKind` 序列化为小写字符串 `"straight"` / `"merge"` / `"branch"`；
`SignatureStatus` 序列化为 camelCase 字符串，如 `"unsigned"`、`"untrustedGood"`）。

**游标语义**：`cursor` 是下一页第一行的**全局序号**（0 起始），首页省略（后端按 0 处理）。
用序号而不是 oid 的真正价值在**增量刷新**：新提交到达时只有序号变化的区间需要局部重排
（T2.9 的布局缓存）。代价要如实说明：**深分页的代价是 O(已加载行数)**（`git log --skip=N`
与 libgit2 的 walk 都如此——git 本身的行为，任何客户端都绕不开）。`nextCursor === null`
即末页。

**折叠已合并分支**：`collapseMergedBranches` 开启且本次返回为末页时，服务层对每个 merge
判定“第二父的全部严格祖先是否都在第一父的可达集内”，通过的 merge 做折叠标记：
merge 行的 `collapsed` 记录第二父（被折叠分支的 tip）的 oid；tip 行置 `hidden`——
重复合并里 tip 已可从主线到达时不置 `hidden`，只记 `collapsed`。**后端只做标记**——
行不删除、lane 与边不动（全局行号与游标语义都建立在“行不删”上），是否隐藏、如何呈现
由前端（T2.2）决定；窗口不完整或 `firstParentOnly` 模式下整体放弃，静默回退为普通布局。

- **错误**：
  - `NOT_FOUND`：repoId 无效（记录不存在）；
  - `VALIDATION`：`followRenames` 开启但路径数 ≠ 1；`myCommitsOnly` 开启但仓库
    未配置 `user.email`。`pageSize` 越界**不**返回
    `VALIDATION`——由服务层直接钳制到 `1..=500`（见 `pageSize` 参数行）；
  - `STORAGE`：仓库读取失败。
- **前端封装**：由前端代理回填（`src/lib/ipc/history.ts`）
- **调用点**：由前端代理回填（`src/lib/ipc/history.ts`）

### git_commit_detail

读取**一次提交**的详情：元数据（含正文与签名状态）、指向它的 refs、相对指定父提交的
统计与文件清单、以及一组状态标记（`isHead` / `isPushed` / `isMerge`）。由 T2.4 的
提交详情面板消费。

现状：服务层已落地（`crates/services/src/commit_detail.rs`），**命令已接线**
（`crates/commands/src/commit_detail.rs`，已注册到 `src-tauri/src/main.rs`）。

- **能力等级**：`ReadOnly`（只读展示：不改仓库状态、不写数据库，无需快照/审计）
- **参数**：

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `repoId` | `number` | 是 | 仓库记录 id |
| `oid` | `string` | 是 | 完整 40 位十六进制提交 oid；形状非法直接 `VALIDATION`（不进引擎） |
| `parentIndex` | `number \| null` | 否 | 与哪个父提交比较：0 = 第一父（缺省），1 = 第二父（合并提交双父 diff 切换）。**非根提交**越界返回 `VALIDATION`；**根提交**只允许 0（0 表示"与空树比较"） |

- **返回**：`CommitDetail`

```ts
interface CommitDetail {
  meta: {
    oid: string;
    shortOid: string;      // 7 位
    parents: string[];     // 顺序与 Git 一致（第一个是 first-parent）；根提交为空数组
    author: CommitSignature;   // { name, email, time }，与 git_log_page 的 Commit 相同形状
    committer: CommitSignature;
    subject: string;
    body: string | null;   // show 带 %b；列表查询没有这个字段
    signature: SignatureStatus;  // 与 git_log_page 的枚举一致
  };
  refs: string[];          // %D 原文（如 "HEAD -> main"、"tag: v1.0.0"）
  stats: {
    filesChanged: number;  // 相对 parentIndex 指定的父提交
    insertions: number;    // 二进制文件不计行数
    deletions: number;
  };
  files: Array<{           // 文件级清单；行级内容按需经 workspace_diff 拉取（target: "between"）
    path: string;
    oldPath: string | null;
    kind: string;          // added / deleted / modified / renamed / copied / typeChanged / unknown
    binary: boolean;
    additions: number;
    deletions: number;
    truncated: boolean;
  }>;
  isMerge: boolean;        // 父提交数 > 1
  isHead: boolean;         // == 当前 HEAD
  isPushed: boolean;       // 被任一远端跟踪分支包含（`git branch -r --contains` 口径）
  webUrl: string | null;   // 由 origin 的 fetch URL 推断（仅 GitHub / GitLab.com / Bitbucket.org；
                           // 自建实例形态无法保证，猜错比不给更糟）；提交页路径由前端拼接
  parentIndex: number;     // 本次文件清单实际使用的父下标
}
```

**引擎路由（有意的不对称）**：元数据走 **CLI** 的 `show`——`refs`（`%D`）与签名状态
（`%G?`）只有 CLI 给得出（libgit2 的 `Commit` 有意留空这两项，见
`crates/git-engine/src/engine/libgit2_engine.rs` 的 `to_commit`）；统计与文件清单走
**CLI 的 diff**（T1.5 起的唯一数据源）；`isHead` 走 libgit2 的 `head_oid`；
`isPushed` 走 CLI 的 `remote_refs_containing`（libgit2 未实现该读取）。
合并提交**必须**显式 `Between{父, 提交}`：CLI 对 `Commit` 目标（`git show`）在 merge 上
输出 combined diff，回答的是"冲突解决了什么"，不是"相对某父改了什么"——文件清单要的是
后者。双引擎一致性由 `crates/git-engine/tests/differential.rs` 的
`commit_detail_inputs_are_consistent_across_engines` 钉住（含不对称契约本身）。

- **错误**：
  - `NOT_FOUND`：repoId 无效，或提交不存在；
  - `VALIDATION`：oid 不是完整 40 位十六进制；`parentIndex` 越界（非根提交 ≥ 父数，根提交 ≠ 0）。
- **单文件 diff / 复制为 patch**：**不新增命令**——复用 `workspace_diff` /
  `workspace_diff_patch`（`DiffRequest` 已支持 `target: "between"`，`from` = 所选父 oid、
  `to` = 提交 oid；父 oid 由本命令的 `meta.parents` 下发）。避免"同一份数据两个入口"。
- **失效**：`repo:changed` 的 `refs` / `large` 类别按 `[COMMIT_DETAIL_QUERY_KEY, repoId]`
  前缀失效（`isHead` / `isPushed` / refs 都会随引用移动而变化）。
- **前端封装**：由前端代理回填（`src/lib/ipc/commitDetail.ts`）
- **调用点**：由前端代理回填（`src/features/history/CommitDetailPanel.tsx`）

### git_log_authors

列出仓库作者（T2.3 的作者筛选下拉）：按**邮箱**去重（git 身份里稳定的是邮箱），
组内姓名取出现最多者（并列取字典序），输出按提交数降序、并列按邮箱升序。
范围与 `--all` 一致——作者筛选作用于全仓库。

现状：服务层已落地（`crates/services/src/history.rs` 的 `HistoryService::authors`），
**命令已接线**（`crates/commands/src/history.rs`，已注册到 `src-tauri/src/main.rs`）。
汇总逻辑是纯函数（`domain::git::summarize_authors`，含单测）。

- **能力等级**：`ReadOnly`
- **参数**：`repoId`（`number`，必填）
- **返回**：`AuthorSummary[]`

```ts
interface AuthorSummary {
  name: string;        // 组内出现最多的姓名
  email: string;       // 去重键
  commitCount: number; // --all 范围内的提交数
}
```

**引擎路由**：只走 CLI（`git log --all --format=%an%x1f%ae%x1e -z`，上限 1 万条记录）；
libgit2 侧未实现该读取（与 `remote_refs_containing` 同一先例），差分测试钉住
"CLI 解析正确 + libgit2 如实拒绝"。

- **错误**：`NOT_FOUND`（repoId 无效）。
- **前端封装**：由前端代理回填（`src/lib/ipc/history.ts`）
- **调用点**：由前端代理回填（`src/features/history/HistoryPage.tsx` 的筛选栏）

#### 与任务书 `git_log_search` 的关系（有意偏差）

任务书 T2.3 第 2 条要求独立的 `git_log_search` 命令。实现上**不设**第二对命令：
关键词搜索 = `git_log_page` 的 `messageContains` + `caseInsensitive`（同一份分页
数据、同一个游标语义，避免"同一份数据两个入口"）。"高亮匹配 / 上一处 / 下一处"
是纯前端行为（在已加载行里定位），不涉及后端。

### 分支与标签管理（T2.5）

写命令全部**审计**（`branch.*` / `tag.*`），危险路径**快照**（`PreHeadMove`，与提交
路径共用同一个快照历史）；确认语义是**显式参数**（`confirmForce` / `confirmUnmerged`），
services 层在缺确认时拒绝——API 直调绕不过 UI。

现状：服务层已落地（`crates/services/src/branch.rs`），命令已接线
（`crates/commands/src/branch.rs`，已注册到 `src-tauri/src/main.rs`）。

| 命令 | 能力 | 参数 | 返回 / 说明 |
| --- | --- | --- | --- |
| `git_branch_list` | ReadOnly | `includeRemote?: boolean` | `Branch[]`；**当前分支置顶 → 本地 → 远端**（与任务书分组要求一致） |
| `git_branch_compare` | ReadOnly | `a: string, b: string` | `{ ahead, behind, onlyInA: [oid, subject][] }`；删除确认清单的数据源 |
| `git_branch_create` | Write | `spec { name, startPoint?, checkout, trackUpstream? }` | 名称先过 ref-format 校验；`checkout=true` 时创建后**干净切换**（不干净报 `VALIDATION`） |
| `git_branch_switch` | Write | `target, strategy: "stash"\|"force"\|"clean", confirmForce?: boolean` | 返回快照 id（Force 时）。**三策略**：stash（含未跟踪，切换后自动恢复；恢复失败如实报错、stash 保留）、force（必须 `confirmForce=true`，先快照）、clean（不干净直接拒绝，对应"取消"） |
| `git_branch_rename` | Write | `spec { old, new, renameRemote }` | 新名过校验 |
| `git_branch_delete` | Write | `spec { names[], force, alsoDeleteRemote }, confirmUnmerged?: boolean` | `{ deleted: string[] }`；当前分支不可删；`force=true` 必须 `confirmUnmerged=true`（调用方应先用 `git_branch_compare` 把独有提交展示给用户），删除前打快照 |
| `git_branch_set_upstream` | Write | `spec { branch, upstream?: string }` | `upstream` 缺省 = 取消上游 |
| `git_tag_list` | ReadOnly | – | `Tag[]`（轻量/附注、消息、时间） |
| `git_tag_create` | Write | `spec { name, target?, message?, sign, force }` | `message` 有值 = 附注标签；轻量标签不可签名（`VALIDATION`）；重名需 `force` |
| `git_tag_delete` | Write | `spec { names[], alsoDeleteRemote }` | 远端删除经 push 通道（T2.6），本命令只删本地 |

**分支名校验**（实现要求 3）：`domain::git::validate_ref_name`——本地实现
`git check-ref-format` 全规则（空格、`~^:?*[]\`、连续点/斜杠、以点或斜杠开头结尾、
`.lock` 分量、`@{`、单 `@`、控制字符），错误给**人话原因**（如"不能包含连续的点"），
作为 `VALIDATION` 的 detail 返回。20 个非法用例 + 合法名测试在 domain 单测里。

**引擎路由**：分支/标签写操作全部走 CLI（libgit2 侧如实返回
`UNSUPPORTED_BY_ENGINE`，差分测试钉住）；`branch_compare` 用
`git rev-list --left-right --count` + `git log a --not b`（机器可读格式）。

- **错误**：`VALIDATION`（名称非法 / 无确认 / 不干净工作区 / 当前分支不可删 /
  轻量标签带签名）、`NOT_FOUND`（repoId 无效）、`Internal`（git 拒绝，如未合并 `-d`）。
- **失效**：写命令完成后由前端统一失效 `[BRANCHES_QUERY_KEY, repoId]` 与
  `[LOG_QUERY_KEY, repoId]`（分支图变化）。
- **前端封装**：由前端代理回填（`src/lib/ipc/branches.ts`）

### 远端同步与 Remote 管理（T2.6）

fetch / pull / push 都是**长任务**（走 `JobRunner`：立即返回 `jobId`，进度经
`job:progress`、结果经 `job:done`、失败经 `job:failed`）；Remote CRUD 是瞬时命令，
同步返回。写命令全部**审计**（`remote.*`），pull 会移动 HEAD，因此**先打 `PreSync` 快照**
（快照在 services 层，与提交路径共用同一个快照历史；快照失败不阻断但记 warn）。

现状：服务层 `crates/services/src/sync.rs`，命令 `crates/commands/src/sync.rs`
（已注册到 `src-tauri/src/main.rs`），前端封装 `src/lib/ipc/sync.ts`，
调用点：仓库页顶部同步条。

| 命令 | 能力 | 参数 | 返回 / 说明 |
| --- | --- | --- | --- |
| `git_fetch` | Network | `repoId, spec { remote?, prune, refspecs[], tags, depth? }` | `jobId`；`job:done.result = { remote, fetch }`；写审计（`sync.fetch`，T2.10 起） |
| `git_pull` | Mutating | `repoId, spec { remote?, branch?, strategy, autostash, allowUnrelated }` | `jobId`；`job:done.result = { remote, pull }`；`strategy ∈ "fastForwardOnly" \| "merge" \| "rebase"`，缺省 `fastForwardOnly`（最安全）；写审计（`sync.pull`，T2.10 起），`pull.snapshotId` 关联执行前的 `PreSync` 快照 |
| `git_push` | Network | `repoId, spec { remote?, branch?, setUpstream, forceWithLease, tags, remoteBranch?, dryRun }` | `jobId`；`job:done.result = { remote, push }`；被拒见下方 `PUSH_REJECTED` |
| `git_remote_list` | ReadOnly | `repoId` | `Remote[]`（`{ name, fetchUrl, pushUrl?, kind }`；`kind ∈ "https" \| "ssh" \| "git" \| "file" \| "other"`） |
| `git_remote_add` | Mutating | `repoId, name, url` | `()`；名称（单段、无空白、不含 ref 禁用字符）与 URL 形状先校验 |
| `git_remote_remove` | Mutating | `repoId, name` | `()`；同时清理该远端的远端跟踪引用 |
| `git_remote_rename` | Mutating | `repoId, old, new` | `()`；新名先校验 |
| `git_remote_set_url` | Mutating | `repoId, name, url` | `()`；URL 形状先校验（可达性只有网络操作能验证） |

> `git_tag_delete` 的 `alsoDeleteRemote` 与分支的远端删除都经 push 通道（本任务），
> 不走单独命令。

**`job:done` 的结果形状（同步专用）**：

```ts
interface SyncJobResult {
  /** 实际使用的远端名（缺省 origin）。 */
  remote?: string;
  /** 只有 git_fetch 会填。 */
  fetch?: FetchOutcome; // { remote, updates: RefUpdate[] }
  /** 只有 git_pull 会填。 */
  pull?: PullOutcome; // { fetch, strategy, upToDate, merge?: { kind, oid?, conflicts[] }, snapshotId? }
  /** 只有 git_push 会填。 */
  push?: PushOutcome; // { remote, updates: RefUpdate[], rejections[] }
}
```

**五类结果的判定（前端只按这些字段分支，不要解析 `message` 文本）**：

| 情况 | 后端行为 |
| --- | --- |
| 正常 | `job:done`；`fetch.updates` / `push.updates` 逐条给出引用变更（`kind ∈ "new" \| "updated" \| "deleted" \| "upToDate" \| "rejected"`） |
| pull 冲突 | `job:done`，`pull.merge.kind = "conflicted"` 且 `merge.conflicts` 非空——**不是错误**：git 的非零退出码代表"留在冲突状态等用户解决"（M3 的冲突向导读的就是这份清单） |
| push 被拒（non-fast-forward） | `job:failed`，`error.code = "PUSH_REJECTED"`，`error.actions` 固定三条：先拉取（`git_fetch`）/ `--force-with-lease` / 取消。**其它拒绝原因**（权限、hook）不提供强推选项——那只会误导用户（强推同样会被拒） |
| push 被拒后选择覆盖 | 前端走**两步**（红线 R7）：先 `git_fetch` 拿到远端真实状态，再弹确认框展示"领先/落后"（即会被覆盖掉多少），用户确认后才 `git_push{forceWithLease:true}`。不允许跳过拉取直接强推——那会让 `--force-with-lease` 退化为无条件覆盖 |
| 断网 / 认证失败 | `job:failed`，`error.code = "NETWORK"` / `"AUTH_REQUIRED"`（进程层固化 `GIT_TERMINAL_PROMPT=0`，需要交互输入的远端立即失败并转为结构化错误） |
| 取消 | `job:failed`，`error.code = "CANCELLED"`；取消令牌透传到进程层（kill 子进程），已被取消的令牌不会启动操作 |

**红线 R7 的落点**：`PushSpec` 里**没有**裸 force 字段，参数构造器只可能产出
`--force-with-lease`（单测断言四种组合都不出现裸 `--force` / `-f`）；push 的远端
位置参数始终显式给出（缺省 `origin`），否则 git 会把 refspec 当仓库名。

- **错误**：`VALIDATION`（远端名 / URL 非法）、`NOT_FOUND`（repoId 无效）、
  `PUSH_REJECTED`、`NETWORK`、`AUTH_REQUIRED`、`CANCELLED`。
- **失效**：fetch 完成后失效 `[BRANCHES_QUERY_KEY, repoId]`（远端跟踪引用变了）；
  pull / push 完成后还需失效 `[STATUS_QUERY_KEY, repoId]` 与 `[LOG_QUERY_KEY, repoId]`
  （HEAD 与提交图变了）。键的唯一来源是 `src/lib/queryKeys.ts`。

### 凭据与认证诊断（T2.7）

密文存在**系统凭据库**（Windows Credential Manager / macOS Keychain / Linux Secret Service，
runtime 需要 gnome-keyring/KWallet 之类的提供者）；本地只留一份**索引**
（`<应用数据目录>/credentials.index.json`：provider / host / login / 类型 / 时间，**不含密文**）。
系统凭据库没有统一的枚举接口，"我保存过哪些账号"只能靠索引（红线 R8）。
固定 keyring service 名为 `org.forgedesk.app`，条目名（account）为 `<provider>:<host>:<login>`。

**网络操作怎样拿到凭据**：`git_fetch` / `git_pull` / `git_push` 发起前按"本次实际会联系的远端"
（显式 `remote` → 当前分支的上游远端 → `origin`）解析凭据，并以**应用自身作为 `GIT_ASKPASS`
辅助程序**注入：`<应用可执行文件> --askpass "<git 的提示语>"`，答案经环境变量
`FORGEDESK_ASKPASS_USERNAME` / `FORGEDESK_ASKPASS_SECRET` 交给**那一次**子进程。
`GIT_ASKPASS` 的默认值是空串（禁止任何 askpass 接管，防环境注入），只有这条通道能覆盖它；
`GIT_TERMINAL_PROMPT=0` 始终生效——答案缺失时 git 直接失败，不会把应用挂住。
SSH 远端（`git@host:path` / `ssh://`）与本地路径**永不**注入令牌（认证走密钥与 agent）。

**连续失败保护**：同一 host 连续 3 次认证失败后，后端在发起网络操作**之前**直接返回
`AUTH_REQUIRED`（`message` 说明不再自动重试，`hint` 是 host），直到用户保存/更新该 host 的凭据
（`credentials_save` 成功即清零计数）。`clone` 尚未接入本通道（已知缺口，见 T2.7 交接说明）。

| 命令 | 能力 | 参数 | 返回 / 说明 |
| --- | --- | --- | --- |
| `credentials_list` | ReadOnly | — | `CredentialMeta[]`：`{ key: { provider, host, login }, kind, createdAtMs }`，**不含密文** |
| `credentials_save` | Mutating | `provider, host, login, kind, secret` | `CredentialMeta`；`kind ∈ "pat" \| "oauth" \| "password"`；provider/login 非空且不含 `:`，空 `secret` → `VALIDATION` |
| `credentials_delete` | Mutating | `provider, host, login` | `()`；幂等（删不存在的条目也成功） |
| `credentials_status` | ReadOnly | — | `{ backend, mode, count, indexPath?, vaultPath?, vaultExists, keyringUnavailableReason? }`；`backend ∈ "systemKeyring" \| "encryptedVault" \| "memory"`，`mode ∈ "systemKeyring" \| "vaultUnlocked" \| "vaultLocked"`；写-读-删一条哨兵来探测系统凭据库 |
| `credentials_vault_create` | Mutating | `passphrase` | `()`；创建加密保险库（Argon2id + AES-256-GCM）并切换过去，同时把 `credentials.backend` 记为 `encryptedVault`；**已存在保险库文件时拒绝**（覆盖等于悄悄清空已有凭据）→ `VALIDATION` |
| `credentials_vault_unlock` | Mutating | `passphrase` | `()`；解锁并切换；口令错或文件被改 → `STORAGE`（两者在 AES-GCM 下不可区分，这是刻意的） |
| `credential_test_remote` | ReadOnly | `url?` 或 `repoId + remote?` | `{ refs }`（远端引用条数，空仓库为 0）；`git ls-remote`，超时 5s；失败按 stderr 分类（见下） |
| `credentials_ssh_inventory` | ReadOnly | — | `{ directory?, keys[], agent }`；`keys[] = { publicPath?, privatePath?, keyType?, comment? }`（**不读私钥内容**，只判存在性），`agent = { kind: "ready" \| "noIdentities" \| "notRunning" \| "unknown", keys[], reason? }`（`ssh-add -l`，超时 5s，退出码 1/2 是**正常结局**而不是错误） |

**回退方案（系统凭据库不可用时）**：探测失败 → `credentials_status.keyringUnavailableReason` 有值 →
界面提示可"改用加密文件存储" → `credentials_vault_create` 建库并切换 → 选择记在全局设置
`credentials.backend`（`"systemKeyring"` / `"encryptedVault"`）。下次启动时若该键是
`encryptedVault` 且库文件存在，应用进入 `vaultLocked`（不会拿空 keyring 冒充"没有凭据"），
由设置页引导解锁。保险库口令**不保存**：忘记口令等于其中的凭据不可恢复（`hint` 里给出库文件路径）。

**连续失败保护**：同一 host 连续 3 次认证失败后，后端在发起网络操作**之前**直接返回
`AUTH_REQUIRED`（`message` 说明不再自动重试，`hint` 是 host），直到用户保存/更新该 host 的凭据
（`credentials_save` 成功即清零计数）。

**认证与网络失败的错误码**（§1.1 已登记）：`AUTH_REQUIRED` / `AUTH_EXPIRED`（HTTPS 凭据）、
`SSH_HOST_KEY_UNVERIFIED`（主机指纹未信任或已变化）、`SSH_KEY_REJECTED`（公钥被拒 / 找不到
identity 文件）、`TLS_CERTIFICATE_REJECTED`（自签名或证书链不完整）、`PROXY_FAILED`（代理
拒绝连接或 407）。分类在 `ErrorCode::classify`（纯字符串逻辑，有 ≥ 10 条真实 stderr 样本的单测），
顺序上 SSH/TLS/代理**先于**通用的认证与网络规则——因为 `Permission denied (publickey)`
里也含 `permission denied`。这四类码各带一个"测试连接"动作
（`command = "credential_test_remote"`，`args = { repoId }`）：领域层只产出动作骨架，
`repoId` 由命令层在错误离开任务时补上——前端会照 `args` 原样 invoke，缺参数的按钮点下去
只会得到一条 `VALIDATION`。已有自带动作的错误（如 `PUSH_REJECTED` 的三条路径）不会被叠加。

**SSH 侧**（`credentials_ssh_inventory`）：盘点 `~/.ssh` 下的密钥与 `ssh-add -l` 的结果，
回答 `Permission denied (publickey)` 之后最常被问的那一半——"本地到底有哪些密钥、agent 里
加载了哪几把"。**不读私钥内容**（只对私钥文件做存在性判断），因此"这是一把私钥"是基于
命名惯例的**候选**判定，界面文案不得写成断言。它不能回答"服务端是否接受这把公钥"，
那只有 `credential_test_remote` 实际连一次才能知道。

`repo_clone` 也在同一通道上：克隆时还没有仓库，因此按 **spec 里的 URL** 解析凭据
（这正是"第一次接触远端"的路径，私有仓库没有凭据必然失败）。

> OAuth 设备码登录与多账号模型已在 T4.3/T4.4 接线，见下一节「托管平台账号」；
> 令牌的密文存储复用本节的两套后端（系统 keyring / 加密保险库）。

### 托管平台账号（T4.3/T4.4）

多账号模型：`accounts` 表存元数据（`{ id, provider, host, login, avatarUrl, scopes, createdAt }`），
**令牌密文永远在凭据库**（keyring account 为 `provider:host:login`，红线 R8）。
同一 `provider+host+login` 重复登录沿用旧 id 与创建时间，并覆盖旧令牌。

**Device Flow（T4.3）**：`account_device_flow_start` 返回三步引导数据（复制码 → 打开浏览器 →
自动轮询）。`device_code` 是秘密，**只存在于后端会话表**，前端拿不到（类型上不可序列化）。
`account_device_flow_wait` 是长任务（`JobRunner`）：轮询节奏（interval、`slow_down` +5s、
过期判定）遵循 RFC 8628，结果经 `job:done` / `job:failed` 送达，取消走 `job_cancel`。

**client_id**：Device Flow 需要 OAuth App 的 client_id（公开值），从全局设置键
`provider.github.clientId` 读取；未配置时两个登录命令直接返回 `VALIDATION`
（`hint` 是该设置键），不发起网络请求。

**令牌失效（T4.5 起）**：用已保存令牌发起的 API 请求收到 401 时，错误码为
`AUTH_EXPIRED`——界面对账号条目给出"重新登录"引导；401 不会无限重试（重试策略明确排除 4xx）。

| 命令 | 能力 | 参数 | 返回 / 说明 |
| --- | --- | --- | --- |
| `account_login_with_pat` | Network | `host, token` | `Account`；令牌经 `/user` 校验后落 keyring（`kind=pat`）+ `accounts` 表；host 为空或 token 为空 → `VALIDATION`；令牌无效 → `AUTH_EXPIRED`，**两处落地均无残留** |
| `account_device_flow_start` | Network | `host, scopes?` | `DeviceFlowSession`：`{ flowId, userCode, verificationUri, verificationUriComplete?, expiresInSecs, intervalSecs }`（**不含 device_code**）；client_id 未配置 → `VALIDATION` |
| `account_device_flow_wait` | Network | `flowId` | `jobId`（长任务）；`job:done` 载荷 `{ account }`；令牌落 keyring（`kind=oauth`）+ 账号表；用户拒绝 → `AUTH_REQUIRED`；流程/设备码过期 → `AUTH_EXPIRED`；取消（`job_cancel`）→ `CANCELLED`；未知/已消费的 `flowId` → `NOT_FOUND` |
| `account_list` | ReadOnly | — | `Account[]`（按创建时间排序） |
| `account_remove` | Mutating | `accountId` | `()`；先删凭据库条目再删账号行；未知 id → `NOT_FOUND` |

### Pull Request（T4.7：列表 / 详情 / review / 行内评论 / 合并）

PR 命令走与仓库命令同一套 HTTP 底座与账号解析（绑定账号 → host 默认账号）。
合并的错误语义（docs/PLAN.md"失败返回可读原因"）：

| 状态 | 语义 | 错误码 / hint |
| --- | --- | --- |
| 405 | 不可合并（冲突未解 / 分支保护 / 已合并） | `VALIDATION` + `not-mergeable` |
| 409 | head 与服务器不一致 | `GIT_CONFLICT` + `conflict` |
| 422 | `expectedHeadSha` 预检不匹配（远端版 PLAN_STALE） | `VALIDATION` + `head-changed` |

GitHub 的可读 message 原样保留在 `message`/`detail`。删除源分支在合并**成功之后**
单独执行，删除失败不影响合并结果（`branchDeleted` 如实上报）。

| 命令 | 能力 | 参数 | 返回 / 说明 |
| --- | --- | --- | --- |
| `repo_pull_list` | Network | request：`{ host, owner, repo, repoId?, stateFilter?, page?, perPage? }` | `PullPage`：`{ items: PullSummary[], nextPage? }`；`stateFilter ∈ "open" / "closed" / "all"`（缺省 open） |
| `repo_pull_get` | Network | `host, owner, repo, number, repoId?` | `PullDetail`：结构化字段 + `bodyHtml`（描述已消毒，原文不出后端）+ `mergeable`/`mergeableState`/`headSha`/变更统计 |
| `repo_pull_reviews` | Network | `host, owner, repo, number, repoId?` | `PullReview[]`：`{ id, author, state, body?, submittedAt? }` |
| `repo_pull_comments_list` | Network | `host, owner, repo, number, repoId?` | `PullComment[]`：`{ id, author, body, createdAt? }`（正文为 Markdown 原文，前端以纯文本渲染） |
| `repo_pull_comment_create` | Network | `host, owner, repo, number, body, repoId?` | `PullComment`；空正文 → `VALIDATION` |
| `repo_pull_review_submit` | Network | request：`{ host, owner, repo, number, event, body?, repoId? }` | `()`；`event ∈ "APPROVE" / "REQUEST_CHANGES" / "COMMENT"`；COMMENT 无正文 → `VALIDATION` |
| `repo_pull_files` | Network | request：`{ host, owner, repo, number, repoId?, page?, perPage? }` | `PullFilePage`：`{ items: PullFile[], nextPage? }`；`PullFile = { filename, previousFilename?, status, additions, deletions, changes?, patch?, hunks: PullDiffHunk[] }`；二进制/超大 diff 无 `patch`（`hunks` 为空） |
| `repo_pull_review_comments_list` | Network | `host, owner, repo, number, repoId?` | `PullReviewComment[]`：`{ id, inReplyTo?, author, body, path?, side?, line?, startLine?, startSide?, createdAt? }`（单页 100 条） |
| `repo_pull_review_comment_create` | Network | request：`{ host, owner, repo, number, path, side, line, startLine?, startSide?, body, repoId? }` | `PullReviewComment`；`side ∈ "LEFT" / "RIGHT"`（LEFT=旧文件 / RIGHT=新文件）；后端**先取当前 diff 本地校验**：路径不在 diff 上 → `VALIDATION` + `path-not-in-diff`，行号越界 / 多行跨 hunk → `VALIDATION` + `line-out-of-range`，均**不发出写请求**；无 `patch`（二进制/超大 diff）时放行由 GitHub 兜底 |
| `repo_pull_review_comment_reply` | Network | request：`{ host, owner, repo, number, commentId, body, repoId? }` | `PullReviewComment`；位置沿用被回复评论，无需锚点；空正文 / `commentId=0` → `VALIDATION` |
| `repo_pull_merge` | Network | request：`{ host, owner, repo, number, strategy, repoId?, commitTitle?, commitMessage?, expectedHeadSha?, deleteBranch?, headBranch? }` | `MergeOutcome`：`{ merged, sha?, message?, branchDeleted }`；`strategy ∈ "merge" / "squash" / "rebase"` |

`PullSummary`：`{ number, title, state, draft, merged, author, headLabel, baseLabel, htmlUrl,
createdAt?, updatedAt? }`。

`PullDiffHunk`：`{ oldStart, oldLines, newStart, newLines, header, lines: PullDiffLine[] }`，
`PullDiffLine = { kind: "context" | "added" | "removed" | "noNewline", content, oldNo?, newNo? }`
——与工作区 diff（T1.5）的 DTO 同名同义，前端同一套行级渲染。行内评论创建时
后端取同一时刻的 head（`commit_id`）：校验与锚定基于同一份 diff。

**多账号与仓库绑定（T4.5）**：克隆/push/fetch/pull 所用账号按远端 host 匹配已保存账号；
同一 host 有多个账号时，按"仓库绑定的账号 → URL 里的用户名 → 最早登录的账号"挑人。
绑定命令见下一节。

### 远端仓库与账号绑定（T4.5）

远端仓库走平台 REST（列表/搜索/星标/fork），分页用页码游标：`nextPage` 为 `null`
表示没有更多，无限滚动每次带 `page` 追加一页。需要登录的端点在"该 host 无已登录
账号"时直接返回 `AUTH_REQUIRED`（`hint` 是 host），不发注定失败的匿名请求；
`per_page` 上限 100（超限 `VALIDATION`）。

`repo_account_binding_*` 读写**仓库级设置** `accounts.preferredAccount`
（值为 `accounts` 表的账号 id）：绑定的账号对该仓库的 git 同步与平台 API 同时生效；
账号被删除时绑定随之失效（解析时按 id 找不到即视为未绑定）。

| 命令 | 能力 | 参数 | 返回 / 说明 |
| --- | --- | --- | --- |
| `repo_remote_list` | Network | `host, repoId?, scope?, page?, perPage?` | `RepoPage`：`{ items: RemoteRepo[], nextPage? }`；`scope ∈ "owned" \| "all"`（缺省 `owned`）；需要登录 |
| `repo_remote_starred` | Network | `host, repoId?, page?, perPage?` | `RepoPage`；星标列表；需要登录 |
| `repo_remote_search` | Network | `host, query, repoId?, page?, perPage?` | `RepoPage`；匿名可用（有账号走高配额）；空 query → `VALIDATION` |
| `repo_remote_star` | Network | `host, owner, repo, starred, repoId?` | `()`；加星/取消加星 |
| `repo_remote_fork` | Network | `host, owner, repo, repoId?` | `RemoteRepo`（GitHub 返回 202，副本异步创建中） |
| `repo_remote_readme` | Network | `host, owner, repo, repoId?` | `string`：**已消毒**的 HTML 片段（Markdown 在 Rust 侧白名单渲染，脚本/事件属性/`javascript:`·`data:` URL 一律清除，XSS 用例在 `services::readme`）；无 README → `NOT_FOUND` |
| `repo_account_binding_get` | ReadOnly | `repoId` | `Account?`（未绑定为 `null`） |
| `repo_account_binding_set` | Mutating | `repoId, accountId?` | `Account?`；`accountId` 为 `null` 解除绑定；账号不存在 → `NOT_FOUND` |

`RemoteRepo`：`{ id, owner, name, fullName, description?, htmlUrl, defaultBranch?, private,
fork, stars, pushedAt? }`。

> 克隆时选择账号：`repo_clone` 的请求体带 `loginHint`（登录名，可省），
> 与"每仓库绑定"独立——克隆时还没有仓库，绑定在克隆完成后的仓库设置里配置。

### 文件监听与设置键（T1.10）

监听**没有命令**：它的生命周期跟着仓库的打开与关闭走（`repo_open` / `repo_init` /
`repo_clone` 成功后启动，`repo_close` / `repo_forget` 停止，应用退出时全部停止）。
多一个手动开关只会让界面能进入"仓库开着但没监听而且没人知道为什么"的状态。

| 设置键 | 取值 | 缺省 | 作用 |
| --- | --- | --- | --- |
| `watch.autoRefresh` | `true` / `false` | `true` | 关掉后不启动监听，前端退回 15 秒轮询 |
| `watch.debounceMs` | 数字（毫秒） | `300` | 合并窗口；后端收敛到 `[50, 5000]` |

两条实现纪律：

- **写入即生效**：`settings_set` 命中这两个键时会重启 / 停止正在跑的监听，
  否则"改了设置要重启应用才生效"等同于这个设置不存在；
- **读不到就用默认值**：这两个值在"打开仓库"的路径上被读取，一个被手工改坏的
  设置不该让用户打不开仓库。

监听的行为（过滤、去抖动、溢出）在 `crates/platform/src/watcher.rs`：

| 情况 | 处理 |
| --- | --- |
| `.git/objects`、`.git/logs`、`.git/lfs`、`node_modules`、`target`、`dist`、`build` | 丢弃（每次 git 命令都会写，与界面无关） |
| `.git/index` | 归入 `workspace`（它描述的正是暂存状态） |
| `.git/HEAD`、`.git/refs/**`、`.git/packed-refs` 等 | 归入 `refs` |
| 只读访问、仅元数据变化（权限 / mtime） | 丢弃（`touch` 不改变 git 看到的内容） |
| 一个窗口内超过 2000 个路径 | 只发一次 `large`（不列路径），界面整体失效并说明原因 |

### 储藏与历史操作（T2.8）

全部是**本地**操作，因此都是同步命令（不进 `job:*` 通道）；写操作完成或进入冲突状态后
广播 `repo:changed`（`workspace` 或 `refs`，重置/拣选/反转这类"整仓库都可能变"的用 `large`）。
快照在 services 层动手之前打；审计在命令层写，操作类型为
`stash_save` / `stash_apply` / `stash_drop` / `stash_branch` / `reset` / `cherry_pick` /
`revert` / `reflog_branch`。

| 命令 | 能力 | 参数 | 返回 |
| --- | --- | --- | --- |
| `git_stash_save` | Mutating | `{ message?, includeUntracked, keepIndex, paths? }` | `{ stashed, entry?, snapshotId? }`。**`stashed=false` 不是失败**（没有可储藏的内容）；储藏前打 `pre-worktree-change` 快照，`snapshotId` 随结果返回（T2.10 起，同时也写入审计表） |
| `git_stash_list` | ReadOnly | `repoId` | `StashEntry[]`（`{ index, oid, baseOid, message, createdAt?, includesUntracked, untrackedOid? }`） |
| `git_stash_show` | ReadOnly | `repoId, index` | `{ entry, diff, untracked? }`：`diff` 是**相对 base** 的变更；`-u` 创建的 stash 里未跟踪文件**不在** `diff` 里，单独放在 `untracked`（与空树比较，全是新增）。界面对 `-u` 的 stash 必须两份都展示，只展示 `diff` 会让人以为 stash 是完整的 |
| `git_stash_apply` / `git_stash_pop` | Mutating | `{ index, restoreIndex? }`（`restoreIndex` = `--index`） | `StashOutcome { conflicts, snapshotId? }`。**冲突不是错误**：仓库已进入冲突状态，`conflicts` 列出冲突文件，`pop` 冲突时**不会**删除该条；操作前打 `pre-worktree-change` 快照（T2.10 起随结果返回） |
| `git_stash_drop` / `git_stash_clear` | Dangerous | `repoId[, index]` | `{ dropped: StashEntry[], snapshotId? }`（含 oid）。T2.8 曾按"工作区快照找不回 stash"选择不打点；T3.11 起快照会记下栈上每条的 oid，回滚用 `git stash store` 重新登记，因此**动手前打 `pre-worktree-change` 快照**并把 id 随结果与审计一起返回。被丢 oid 仍进审计 `args`（用户绕过界面直接 git 操作时的自救线索） |
| `git_stash_branch` | Mutating | `{ index, name }` | `{ branch }`；从 stash 的 base 建分支并应用（**会切换分支**，先打 `pre-head-move` 快照）。pop 冲突时的正规出路：新分支从 base 开始，一定干净 |
| `git_cherry_pick` | Mutating | `{ revision, recordSource?, noCommit? }`（`revision` 可以是 `A..B` 区间） | `MergeOutcome { kind, oid?, conflicts[], snapshotId? }`（复用 pull 的冲突形状）。`recordSource` = `-x`（提交信息里附来源） |
| `git_revert` | Mutating | `{ revision, mainline?, noCommit? }` | `MergeOutcome`。**反转合并提交必须给 `mainline`**（从 1 开始），猜错主父会反转出相反的结果，因此没有缺省值 |
| `git_reset_prepare` | ReadOnly | `{ revision, mode }`（`mode ∈ "soft" \| "mixed" \| "hard"`） | `ResetPlan`（见下）。不写仓库 |
| `git_reset_execute` | Mutating | `{ planId, confirmation? }` | `ResetOutcome { mode, headBefore, headAfter, discardedCount, snapshotId? }`。`--hard` 必须带 `confirmation="reset"`（大小写与首尾空白不敏感），否则 `VALIDATION` |
| `git_reflog` | ReadOnly | `repoId, limit?`（缺省 100，上限 1000） | `ReflogEntry[]`（`{ index, oid, branchName?, subject, timestamp? }`，新的在前） |
| `git_reflog_create_branch` | Mutating | `{ index, name }` | `{ branch }`。**不移动任何现有引用**，因此是 reflog 恢复的首选入口 |

**重置的两段式契约**（红线 R7）：

- 计划回答四件事：将被丢弃的提交（最多列 30 条，`discardedCount` 是精确总数）、
  将被丢弃的已暂存/工作区改动、会被覆盖的未跟踪文件（仅 `--hard`：`git reset --hard`
  会删掉"挡在路上"的未跟踪文件）、**远端是否已有这些提交**（`remote.notOnRemote`——
  它是"丢弃后还能不能从远端找回"的唯一依据；没有上游时等于提交总数）。
- `--soft` 不碰索引与工作区，因此计划里 `lostStaged`/`lostWorktree` 恒为空；
  `--mixed` 只丢已暂存改动。
- 执行前的三道闸：计划只能用一次（重复用 = `PLAN_STALE`）；执行时 HEAD 与计划生成时
  不一致 = `PLAN_STALE`（重新预览即可）；`--hard` 要输入确认词。
- 所有模式执行前都打 `pre-head-move` 快照（HEAD 都会动），`snapshotId` 随结果返回。

### 冲突状态机（T3.1）

冲突是**结果不是错误**（T2.8 起的约定）：merge / rebase / cherry-pick / revert 撞上冲突后，
仓库进入一个可查询、可继续、可中止的状态。数据来源是 **index stage**
（`git ls-files -u` 与 `git cat-file` 按 oid 读三方内容），不依赖工作区文件的
`<<<<<<<` 标记——标记可能被用户手动删掉而 index 仍然冲突。

服务 / 命令 / 前端位置：`crates/services/src/conflict.rs`、`crates/commands/src/conflict.rs`、
`src/lib/ipc/conflict.ts`、`src/features/repo/RepoConflictPage.tsx`。
冲突查询的引擎实现**只有 CLI**（stage 三方内容 + 2 MiB 阈值 + 二进制判定的语义以 git CLI
为准，libgit2 侧返回 `UNSUPPORTED_BY_ENGINE`，见 `docs/GIT-ENGINE-DIFF.md` §4）。

| 命令 | 能力 | 参数 | 返回/说明 |
| --- | --- | --- | --- |
| `git_conflict_state` | ReadOnly（`async`） | `{ repoId }` | `ConflictState`。无进行中操作时返回空态（`opKind: null`），不报错；冲突文件含 base/ours/theirs 三方 blob（内容超过 2 MiB、二进制或非 UTF-8 时 `content: null`）；rebase 进度从 `rebase-merge/msgnum`/`end` 读取 |
| `git_conflict_mark_resolved` | Mutating（`async`） | `{ repoId, paths: string[] }` | `null`。执行 `git add` 并校验这些路径的 stage 条目已清空；仍冲突时返回 `CONFLICT_UNRESOLVED`（`hint` 列出未解决路径）。写审计（`conflict_resolve`），不打快照（与暂存同一取舍） |
| `git_conflict_continue` | Mutating（`async`） | `{ repoId }` | `ConflictContinueOutcome { oid, conflicts }`。仍有未解决文件时返回 `CONFLICT_UNRESOLVED`；成功时 `oid` 为完成后的 HEAD；`conflicts` 非空 = 序列重放又停在新的冲突上（**正常结果**，界面刷新状态不弹错误）。merge 的 continue 是 `git commit --no-edit`；全部动作带 `GIT_EDITOR=true` 防止编辑器阻塞。写审计（`conflict_continue`），不打快照 |
| `git_conflict_abort` | Mutating（`async`） | `{ repoId }` | `ConflictAbortOutcome { headOid, headRef, snapshotId }`。**先打 `pre-head-move` 快照再 abort**（rebase 的回滚基线是 `orig-head`），abort 后校验 HEAD 与分支名回到操作前状态，不一致返回 `INTERNAL` 并带实际状态。写审计（`conflict_abort`），`snapshotId` 写进操作记录 |
| `git_conflict_skip` | Mutating（`async`） | `{ repoId }` | `ConflictContinueOutcome`。只有 rebase 支持（其余操作返回 `VALIDATION`）；跳过后撞上新的冲突同样是正常结果。写审计（`conflict_skip`） |
| `git_conflict_file_detail` | ReadOnly（`async`） | `{ repoId, path }` | `ConflictFileDetail`。打开编辑器时的惰性查询：三方 blob（2 MiB / 二进制 / 非 UTF-8 时 `content: null`）、工作区文件形状（`eol` / `bom` / `trailingNewline`，写回时保持）与 diff3 合并块（`blocks`：`context` / `resolved`（自动采用的段，含来源） / `conflict`）。非冲突路径返回 `VALIDATION` |
| `git_conflict_apply_resolution` | Mutating（`async`） | `{ repoId, path, spec: { content, eol, bom, trailingNewline } }` | `null`。把编辑器的结果文本写回工作区（**按原文件形状重建 EOL / BOM / 末尾换行**，前端只产出 LF 文本）→ `git add` → 校验 stage 清空。写审计（`conflict_resolve`，参数记路径与内容长度，不记内容本身） |
| `git_conflict_take_side` | Mutating（`async`） | `{ repoId, path, side: "ours" \| "theirs" }` | `null`。`git checkout --ours/--theirs` 恢复一方到工作区后标记已解决——二进制冲突与删除类冲突的"保留一方"路径。写审计（`conflict_resolve`） |
| `git_conflict_remove_file` | Mutating（`async`） | `{ repoId, path }` | `null`。以"删除该文件"解决删除类冲突（`git rm -f`：工作区与索引一起删；前端必须先确认）。写审计（`conflict_resolve`） |

事件：`git_conflict_mark_resolved` 成功后发 `repo:changed`（`workspace`）；
`git_conflict_continue` / `git_conflict_skip` / `git_conflict_abort` 成功后发
`workspace` + `refs`（可能移动 HEAD）。前端查询键 `[conflict, repoId]`
（`src/lib/repoChanged.ts` 的三类失效均已包含）。

错误码：`CONFLICT_UNRESOLVED`（T3.1 新增，与 `GIT_CONFLICT` 的区别：后者说"仓库里有冲突"，
前者说"你想继续，但这些文件还没解决"，`hint` 携带文件清单）。

### 合并流程（T3.4）

两段式契约（红线 R7 的计划形态）：prepare 生成计划（快进裁决、source 独有提交、
`merge-tree` 冲突预检、默认信息、等价命令），execute 只认 planId 且**取走即失效**；
prepare 与 execute 之间 HEAD 被外部改动 → `PLAN_STALE`。

服务 / 命令 / 前端位置：`crates/services/src/merge.rs`、`crates/commands/src/merge.rs`、
`src/lib/ipc/merge.ts`。预检数据源是 `git merge-tree --write-tree`（git ≥ 2.38；
太旧时 `previewAvailable: false`，界面退化为"执行后再报冲突"）与
`git merge-base`（快进裁决）。只有 CLI 实现（同冲突状态机的取舍）。

| 命令 | 能力 | 参数 | 返回/说明 |
| --- | --- | --- | --- |
| `git_merge_prepare` | ReadOnly（`async`） | `{ repoId, spec: { source, strategy? } }` | `MergePlanDto`。strategy ∈ `merge \| noFf \| squash \| fastForwardOnly \| ours \| theirs`，缺省 `merge`；`verdict ∈ upToDate \| fastForward \| trueMerge`；`conflicted` 非空 = 预检到冲突；source 不存在返回 `VALIDATION` |
| `git_merge_execute` | Mutating（`async`） | `{ repoId, spec: { planId, message? } }` | `MergeOutcome { kind, oid, conflicts, snapshotId }`。`kind ∈ alreadyUpToDate \| fastForward \| mergeCommit \| squash \| conflicted`（squash：变更进索引、HEAD 不动、无提交）；执行前打 `PreSync` 快照（`snapshotId` 随结果返回）；冲突时返回清单（不是错误）。写审计（`merge`） |
| `git_merge_continue` | Mutating（`async`） | `{ repoId, message? }` | `MergeOutcome`。冲突解决后的"继续合并"；`message` 提供时用 `git commit -m`（可编辑信息），否则沿用 `MERGE_MSG`。未解决冲突返回 `CONFLICT_UNRESOLVED`。写审计（`merge`） |

事件：execute / continue 成功后发 `repo:changed`（`workspace` + `refs`）。
合并进行中的检测走既有 `workspace_status` 的 `operation: "merge"`。

### rebase 执行（T3.7）

两段式 + 暂停语义：`git_rebase_preview_only` 预演（只读：装配区间图 → 校验 → 预览），
`git_rebase_execute` 执行（`PreHeadMove` 快照后注入 todo，结局三选一）。暂停是
**结果不是错误**：`kind: "pausedConflict"` 走冲突页（T3.1 状态机），`kind: "pausedEdit"`
由 `git_rebase_continue_edit` 在用户改完内容后恢复（`commit --amend` 接住暂存改动 +
`rebase --continue`，信息沿用原提交；不在 edit 停点时返回 `VALIDATION`——幂等保护）。

todo 注入机制：`GIT_SEQUENCE_EDITOR="cp '<file>'"`（git 把自己的 todo 路径作为
editor 的第一个参数追加，cp 完成替换；`GIT_EDITOR=true` 让 reword 编辑器原样退出）。
只有 CLI 实现（同冲突状态机/合并预检的取舍）。

| 命令 | 能力 | 参数 | 返回/说明 |
| --- | --- | --- | --- |
| `git_rebase_preview_only` | ReadOnly（`async`） | `{ repoId, spec: { base, head, steps[], allowFlattenMerges?, autosquash? } }` | `RebasePreview { surviving, dropped, reworded, squashed, affectedCount, touchesPushed, todoText }`。`touchesPushed` 为真时界面必须提示 force-with-lease；`todoText` 是等价 `git rebase -i` todo 内容（T3.6 面板底部展示）。计划非法返回 `VALIDATION`（detail 列规则短名） |
| `git_rebase_range` | ReadOnly（`async`） | `{ repoId, base, head }` | `[{ oid, parents, subject }]`，**从旧到新**、拓扑序；区间口径与 validate 一致（head 沿全部父链到 base，不含 base）。T3.6 面板打开时的初始清单——必须来自此命令而不是界面已加载的重叠分页数据（todo 漏列的区间提交会被 git 静默丢弃）。空区间返回 `VALIDATION` |
| `git_rebase_execute` | Dangerous（`async`） | 同上 | `RebaseOutcome`：`{ kind: "completed", oid, snapshotId }` / `{ kind: "pausedConflict", conflicts, snapshotId }` / `{ kind: "pausedEdit", oid, snapshotId }`。执行前打 `pre-head-move` 快照，`snapshotId` 随结果回传（`null` = 快照创建失败，界面须如实提示"本次没有回滚点"）。写审计（`rebase`） |
| `git_rebase_continue_edit` | Dangerous（`async`） | `{ repoId }` | `RebaseOutcome`（`snapshotId` 恒为 `null`：continue 时仓库处于 rebase 中间态，对半成品打快照会给出危险的回滚引导；回滚点是执行前那一次）。edit 暂停的恢复；不在 edit 停点返回 `VALIDATION`。写审计（`rebase`） |

事件：execute / continue 成功后发 `repo:changed`（`large` + `refs`——rebase 重写历史）。
域模型见 `crates/domain/src/git/rebase.rs`（validate 六规则 + todo 生成 + preview 纯函数 + 区间清单类型）。
前端封装：`src/lib/ipc/rebase.ts`（`gitRebasePreviewOnly` / `gitRebaseRange` / `gitRebaseExecute` /
`gitRebaseContinueEdit`）；调用点：`src/features/rebase/*`（拖拽面板），入口经
`graphSelectionStore.rebaseRequest` 由历史页右键菜单与提交详情面板发起。

---

## 4. 新增命令的检查清单

1. 命令定义在 `crates/commands/src/<domain>.rs`（**不要**定义在 `lib.rs`，见该文件顶部说明），
   并由 `lib.rs` 重导出；
2. 参数在后端二次校验，非法输入返回 `VALIDATION`；
3. 错误经 `to_app_error` 转换，不自行拼文案；
4. 在本文件登记：能力等级、参数表、返回结构、可能的错误码、前端封装名、调用点；
5. 补单测：正常路径 + 至少一个非法输入路径；
6. 若涉及仓库写操作，确认已接入 `SnapshotManager` + `AuditLog`（M1 起）。
