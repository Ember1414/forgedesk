/**
 * 唯一的 Tauri IPC 出口。
 *
 * 为什么必须集中在这里：`eslint.config.js` 中的 `no-restricted-imports` 规则
 * 禁止 `@tauri-apps/api/*` 出现在 `src/lib/ipc/` 之外的任何文件里
 * （AGENTS.md §6「前后端严格分层」）。
 *
 * 本模块是**桶文件**，把同目录下的几个模块统一导出，让业务代码只需
 * `import { … } from '@/lib/ipc'`：
 *
 * - `client.ts`：底层入口（`invokeCommand` / `listenEvent` / `isTauriRuntime`）；
 * - `repository.ts`：仓库生命周期命令（`repo_*`）；
 * - `jobs.ts`：长任务事件与取消（`job:*`）；
 * - `history.ts`：提交历史分页查询（`git_log_page`）；
 * - `commitDetail.ts`：提交详情（`git_commit_detail`）；
 * - `branches.ts`：分支与标签管理（T2.5）；
 * - `sync.ts`：远端同步与 Remote 管理（T2.6，长任务返回 `jobId`）；
 * - `historyOps.ts`：储藏 / 拣选 / 反转 / 重置 / reflog（T2.8，全部同步命令）。
 *
 * 约定：本文件中的类型必须与 Rust 侧 DTO 的 `serde(rename_all = "camelCase")`
 * 一一对应。
 */
export { invokeCommand, isTauriRuntime, listenEvent } from './client';
export type { Unlisten } from './client';
export { pickFolder } from './dialog';

import { invokeCommand } from './client';

export {
  repoClone,
  repoClose,
  repoDiscover,
  repoForget,
  repoInit,
  repoOpen,
  repoRecentList,
} from './repository';
export type {
  AuditFinding,
  BranchLabel,
  CloneRequest,
  InitRequest,
  JobRef,
  OpenedRepository,
  RecentRepository,
  RepoAudit,
  Repository,
  Worktree,
} from './repository';

export { auditExport, auditList, auditPrune } from './audit';
export type {
  AuditEntry,
  AuditExportFormat,
  AuditExportResult,
  AuditFilter,
  AuditPage,
  AuditPruneResult,
} from './audit';

export { gitCommitDetail } from './commitDetail';
export {
  gitBranchCompare,
  gitBranchCreate,
  gitBranchDelete,
  gitBranchRename,
  gitBranchSetUpstream,
  gitBranchSwitch,
  gitTagCreate,
  gitTagDelete,
  gitTagList,
} from './branches';
export {
  gitCherryPick,
  gitReflog,
  gitReflogCreateBranch,
  gitResetExecute,
  gitResetPrepare,
  gitRevert,
  gitStashApply,
  gitStashBranch,
  gitStashClear,
  gitStashDrop,
  gitStashList,
  gitStashPop,
  gitStashSave,
  gitStashShow,
} from './historyOps';
export { gitMergeContinue, gitMergeExecute, gitMergePrepare } from './merge';
export type {
  FfVerdict,
  MergeExecuteRequest,
  MergePlan,
  MergePlanCommit,
  MergeRequest,
  MergeStrategy,
} from './merge';
export {
  accountDeviceFlowStart,
  accountDeviceFlowWait,
  accountList,
  accountLoginWithPat,
  accountRemove,
} from './accounts';
export type { Account, DeviceFlowSession } from './accounts';
export {
  gitRebaseContinueEdit,
  gitRebaseExecute,
  gitRebasePreviewOnly,
  gitRebaseRange,
} from './rebase';
export type {
  RebaseExecuteRequest,
  RebaseOutcome,
  RebasePreview,
  RebaseRangeCommit,
  RebaseStepRequest,
  RebaseSurvivingCommit,
  ReorderAction,
} from './rebase';
export {
  gitConflictAbort,
  gitConflictApplyResolution,
  gitConflictContinue,
  gitConflictFileDetail,
  gitConflictMarkResolved,
  gitConflictRemoveFile,
  gitConflictSkip,
  gitConflictTakeSide,
  gitConflictState,
} from './conflict';
export type {
  ApplyResolutionRequest,
  ConflictAbortOutcome,
  ConflictBlob,
  ConflictContinueOutcome,
  ConflictFile,
  ConflictFileDetail,
  ConflictKind,
  ConflictOpKind,
  ConflictState,
  LineEnding,
  MergeBlock,
  TakeSide,
} from './conflict';
export type {
  CommitSummary,
  ReflogEntry,
  ResetOutcome,
  ResetPlan,
  ResetRemoteImpact,
  StashDiscardOutcome,
  StashEntry,
  StashOutcome,
  StashSaveOutcome,
  StashShowOutcome,
} from './historyOps';
export type {
  BranchComparison,
  BranchCreateSpec,
  BranchDeleteOutcome,
  BranchDeleteSpec,
  BranchRenameSpec,
  BranchSetUpstreamSpec,
  SwitchStrategy,
  Tag,
  TagCreateSpec,
  TagDeleteSpec,
} from './branches';

export {
  credentialTestRemote,
  credentialsDelete,
  credentialsList,
  credentialsSave,
  credentialsSshInventory,
  credentialsStatus,
  credentialsVaultCreate,
  credentialsVaultUnlock,
  probeUrlFor,
} from './credentials';
export type {
  CredentialBackend,
  CredentialInput,
  CredentialKind,
  CredentialMeta,
  CredentialMode,
  CredentialRef,
  CredentialsStatus,
  RemoteProbe,
  SshAgentKey,
  SshAgentStatus,
  SshInventory,
  SshKeyInfo,
} from './credentials';

export {
  changedUpdates,
  gitFetch,
  gitPull,
  gitPush,
  gitRemoteAdd,
  gitRemoteList,
  gitRemoteRemove,
  gitRemoteRename,
  gitRemoteSetUrl,
  pullConflicts,
  readSyncResult,
} from './sync';
export type {
  FetchOutcome,
  FetchSpec,
  MergeKind,
  MergeOutcome,
  PullOutcome,
  PullSpec,
  PullStrategy,
  PushOutcome,
  PushRejection,
  PushSpec,
  RefUpdate,
  RefUpdateKind,
  Remote,
  RemoteKind,
  SyncJobRef,
  SyncJobResult,
} from './sync';
export type {
  CommitDetail,
  CommitFileChange,
  CommitFileChangeKind,
  CommitMeta,
  CommitStats,
} from './commitDetail';

export { gitBranchList, gitLogAuthors, gitLogPage } from './history';
export type {
  AuthorSummary,
  Branch,
  Commit,
  CommitSignature,
  GraphEdge,
  GraphEdgeKind,
  GraphLayout,
  GraphRow,
  HistoryPage,
  HistoryQuery,
  SignatureStatus,
} from './history';

export {
  repoPullCommentCreate,
  repoPullCommentsList,
  repoPullFiles,
  repoPullGet,
  repoPullList,
  repoPullMerge,
  repoPullReviewCommentCreate,
  repoPullReviewCommentReply,
  repoPullReviewCommentsList,
  repoPullReviewSubmit,
  repoPullReviews,
} from './pulls';
export type {
  PullComment,
  PullDetail,
  PullDiffHunk,
  PullDiffLine,
  PullFile,
  PullFilePage,
  PullMergeOutcome,
  PullPage,
  PullReview,
  PullReviewComment,
  PullSummary,
  ReviewEvent,
} from './pulls';
export {
  listenActionsLogChunks,
  repoActionsJobLogs,
  repoActionsRunCancel,
  repoActionsRunJobs,
  repoActionsRunRerun,
  repoActionsRunsList,
  ACTIONS_LOG_CHUNK_EVENT,
} from './actions';
export type { ActionsLogChunkPayload, RunJob, RunPage, WorkflowRunSummary } from './actions';
export { repoDashboard, MAX_DASHBOARD_TARGETS } from './dashboard';
export type { PullsDigest, RepoDashboard, RunDigest } from './dashboard';
export { repoRateLimitRefresh, repoRateLimitState } from './rateLimit';
export type { RateLimitSnapshot } from './rateLimit';
export {
  createUtf8StreamDecoder,
  listenPtySpikeExit,
  listenPtySpikeOutput,
  ptySpikeClose,
  ptySpikeCreate,
  ptySpikeResize,
  ptySpikeThroughput,
  ptySpikeWrite,
  utf8ToBase64,
  EVENT_PTY_SPIKE_EXIT,
  EVENT_PTY_SPIKE_OUTPUT,
} from './ptySpike';
export type {
  PtySpikeExitPayload,
  PtySpikeInfo,
  PtySpikeOutputPayload,
  PtySpikeThroughput,
} from './ptySpike';

export {
  listenTermExit,
  listenTermOutput,
  systemOpenUrl,
  termClose,
  termCreate,
  termList,
  termOutputTail,
  termReportCommand,
  termResize,
  termScanCommand,
  termShellList,
  termWrite,
  EVENT_TERM_EXIT,
  EVENT_TERM_OUTPUT,
} from './terminal';
export type {
  TermCreateRequest,
  TermCreated,
  TermDanger,
  TermExitPayload,
  TermOutputPayload,
  TermReportRequest,
  TermShell,
  TermSummary,
} from './terminal';
export {
  repoIssueAssignees,
  repoIssueAssigneesSet,
  repoIssueBody,
  repoIssueCommentCreate,
  repoIssueCommentsList,
  repoIssueCreate,
  repoIssueEdit,
  repoIssueGet,
  repoIssueList,
  repoIssueStateSet,
} from './issues';
export type { Assignee, IssueComment, IssueDetail, IssuePage, IssueSummary } from './issues';
export {
  repoAccountBindingGet,
  repoAccountBindingSet,
  repoRemoteFork,
  repoRemoteList,
  repoRemoteReadme,
  repoRemoteSearch,
  repoRemoteStar,
  repoRemoteStarred,
} from './remoteRepos';
export type { RemoteRepo, RemoteRepoPage, RemoteRepoScope } from './remoteRepos';

export {
  cancelJob,
  JOB_DONE_EVENT,
  JOB_FAILED_EVENT,
  JOB_PROGRESS_EVENT,
  onJobDone,
  onJobFailed,
  onJobProgress,
  progressPercent,
} from './jobs';
export type { JobDonePayload, JobFailedPayload, JobProgressPayload } from './jobs';

/** 应用版本与构建信息（对应 Rust 侧 `forgedesk_commands::AppVersion`）。 */
export interface AppVersion {
  readonly version: string;
  readonly gitSha: string;
  readonly target: string;
  readonly profile: string;
}

/** 获取应用版本与构建信息。 */
export function appVersion(): Promise<AppVersion> {
  return invokeCommand<AppVersion>('app_version');
}

/**
 * 上报一条前端未捕获错误（生产环境用；开发与 E2E 收进 `window.__errs`）。
 *
 * 只在没有其他选择时调用：被 React / TanStack Query 接住的错误属于"已处理"，
 * 不该重复上报。
 */
export function logFrontendError(message: string, stack?: string): void {
  // 有意不返回 Promise：调用点在错误处理路径上，没人会 await 它，
  // 而"没人接的 Promise"本身会被 unhandledrejection 再抓一次（无限繁殖）
  void invokeCommand<null>('log_frontend_error', {
    message,
    ...(stack === undefined ? {} : { stack }),
  }).catch(() => {
    // 上报失败就到此为止：日志写不进去时不该再抛
  });
}

/** 设置的归属范围。 */
export type SettingsScope = 'global' | 'repo';

/**
 * 读取一个设置项（不存在返回 `null`）。
 *
 * 仓库级设置必须提供 `repoId`：后端会拒绝"repo 范围但没有 repoId"的调用，
 * 而不是静默降级成全局——那会让某个仓库的设置悄悄写到所有仓库上。
 */
export function settingsGet(
  scope: SettingsScope,
  key: string,
  repoId?: number,
): Promise<string | null> {
  return invokeCommand<string | null>('settings_get', {
    scope,
    key,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/**
 * 写入一个设置项。
 *
 * `value` 必须是 **JSON 字符串**（调用方序列化，后端只校验形状）。
 * 这样存储层不必理解每个设置项的类型，新增设置项不需要改后端。
 */
export function settingsSet(
  scope: SettingsScope,
  key: string,
  value: string,
  repoId?: number,
): Promise<void> {
  return invokeCommand<void>('settings_set', {
    scope,
    key,
    value,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/** 读取某个范围下的全部设置（启动时一次性拉取，避免逐个 key 往返）。 */
export function settingsAll(
  scope: SettingsScope,
  repoId?: number,
): Promise<Record<string, string>> {
  return invokeCommand<Record<string, string>>('settings_all', {
    scope,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/** 一行日志（后端已脱敏；结构化字段用于界面高亮与筛选）。 */
export interface LogLine {
  /** 时间戳（Unix 毫秒）；无法解析时为 null。 */
  timestamp: number | null;
  /** 级别（INFO / WARN / ERROR…）。 */
  level: string | null;
  /** 产生日志的模块。 */
  target: string | null;
  /** 消息正文。 */
  message: string;
  /** 整行原文（已脱敏），用于"贴到反馈里"。 */
  raw: string;
}

/** 在系统文件管理器中打开日志目录。 */
export function logsOpen(): Promise<void> {
  return invokeCommand<void>('logs_open');
}

/**
 * 读取末尾若干行日志（时间线顺序：旧 → 新）。
 *
 * `lines` 缺省 200，后端会夹在 1..=2000 之间——上限存在的意义是避免一次 IPC
 * 把整个日志文件拉进前端。
 */
export function logsTail(lines?: number): Promise<LogLine[]> {
  return invokeCommand<LogLine[]>('logs_tail', lines === undefined ? {} : { lines });
}

/**
 * 触发一个受控失败的演示错误（仅开发构建注册该命令）。
 *
 * 用途：验证"后端分类 → 脱敏 → IPC → 前端 i18n → Toast → 动作按钮"整条链路。
 * 它是基础设施的自检入口：链路坏掉时不会有任何业务功能报错，
 * 只会在真正出错那天集体失效，所以需要能随时主动触发。
 */
export function debugThrowError(code: string): Promise<void> {
  return invokeCommand<void>('debug_throw_error', { code });
}

/**
 * 触发一次真实 panic（仅开发构建注册）。
 *
 * 用途：验证 panic hook（生成 panic 日志）、会话标记（下次启动判定异常退出）
 * 与"主流程不崩溃"（panic 发生在后台线程，界面继续可用）。
 */
export function debugPanic(): Promise<void> {
  return invokeCommand<void>('debug_panic');
}
