/**
 * 远端同步与 Remote 管理（T2.6）：命令 DTO 与具名封装。
 *
 * # 类型来源
 *
 * 镜像 `crates/domain/src/git/spec.rs`（`FetchSpec` / `PullSpec` / `PushSpec`
 * 与 `FetchOutcome` / `PullOutcome` / `PushOutcome`）与
 * `crates/domain/src/git/refs.rs`（`Remote` / `RefUpdate`）。
 * 形状是 **camelCase**（docs/API.md §1：DTO 字段一律 `serde(rename_all = "camelCase")`），
 * 改任何一侧必须同步另一侧。
 *
 * # 为什么 spec 的字段全是可选的
 *
 * Rust 侧的三个 spec 都带 `#[serde(default)]`：缺省字段用后端默认值填充
 * （`strategy` 缺省 `fastForwardOnly`、`remote` 缺省当前分支的上游）。
 * 让前端只写"这次真正要改的字段"，比每次拼一个完整对象更不容易出错。
 *
 * # 长任务
 *
 * `git_fetch` / `git_pull` / `git_push` 立即返回 `{ jobId }`，进度与结果经
 * `job:progress` / `job:done` / `job:failed` 推送（见 `./jobs.ts` 与
 * docs/API.md §3）；被拒绝的 push 会在 `job:failed` 里带 `PUSH_REJECTED` 与三条
 * 修复动作（`action.id` 为 `fetch-first` / `force-with-lease` / `cancel`）。
 */
import { invokeCommand } from './client';

// ---------------------------------------------------------------- DTO：引用与远端

/** 远端 URL 的协议类别。 */
export type RemoteKind = 'https' | 'ssh' | 'git' | 'file' | 'other';

/** 一个远端（`git_remote_list`）。 */
export interface Remote {
  readonly name: string;
  readonly fetchUrl: string;
  /** 与 fetch URL 不同时才非空。 */
  readonly pushUrl: string | null;
  readonly kind: RemoteKind;
}

/** 引用更新的结果类别。 */
export type RefUpdateKind = 'new' | 'updated' | 'deleted' | 'upToDate' | 'rejected';

/** 一次引用变更的明细。 */
export interface RefUpdate {
  /** 引用短名（如 `main`、`origin/main`）。 */
  readonly name: string;
  readonly oldOid: string | null;
  readonly newOid: string | null;
  readonly kind: RefUpdateKind;
  /** 被拒绝的原因（原始 stderr 片段，已脱敏）。 */
  readonly reason: string | null;
}

/** fetch 的结果。 */
export interface FetchOutcome {
  readonly remote: string;
  readonly updates: readonly RefUpdate[];
}

/** 拉取策略（与后端 `PullStrategy` 的 serde 形状一致）。 */
export type PullStrategy = 'fastForwardOnly' | 'merge' | 'rebase';

/** 合并 / 变基的结果类别。 */
export type MergeKind = 'alreadyUpToDate' | 'fastForward' | 'mergeCommit' | 'squash' | 'conflicted';

/** 合并 / 变基的结果。 */
export interface MergeOutcome {
  readonly kind: MergeKind;
  /** 合并后的 HEAD oid；冲突时为 null。 */
  readonly oid: string | null;
  /** 冲突文件（相对仓库根的路径；`kind === 'conflicted'` 时非空）。 */
  readonly conflicts: readonly string[];
}

/** pull 的结果。 */
export interface PullOutcome {
  readonly fetch: FetchOutcome;
  readonly strategy: PullStrategy;
  /** 本地本来就已经是最新（此时不会有合并结果）。 */
  readonly upToDate: boolean;
  readonly merge: MergeOutcome | null;
}

/** push 被拒绝的原因。 */
export interface PushRejection {
  readonly name: string;
  readonly reason: string;
  /** 是否属于"非快进"这一类（只有这一类才提供 force-with-lease）。 */
  readonly nonFastForward: boolean;
}

/** push 的结果。 */
export interface PushOutcome {
  readonly remote: string;
  readonly updates: readonly RefUpdate[];
  readonly rejections: readonly PushRejection[];
}

/** `job:done` 里同步任务的结果（只有对应的那个字段会被填）。 */
export interface SyncJobResult {
  readonly remote?: string;
  readonly fetch?: FetchOutcome;
  readonly pull?: PullOutcome;
  readonly push?: PushOutcome;
}

// ---------------------------------------------------------------- DTO：spec

/** fetch 参数（缺省字段由后端补）。 */
export interface FetchSpec {
  /** 远端名；缺省用当前分支的上游远端（没有上游时用 `origin`）。 */
  readonly remote?: string | null;
  readonly prune?: boolean;
  readonly refspecs?: readonly string[];
  readonly tags?: boolean;
  readonly depth?: number | null;
}

/** pull 参数（缺省字段由后端补）。 */
export interface PullSpec {
  readonly remote?: string | null;
  readonly branch?: string | null;
  readonly strategy?: PullStrategy;
  /** 工作区不干净时自动储藏并在完成后恢复。 */
  readonly autostash?: boolean;
  readonly allowUnrelated?: boolean;
}

/** push 参数（缺省字段由后端补）。 */
export interface PushSpec {
  readonly remote?: string | null;
  readonly branch?: string | null;
  readonly setUpstream?: boolean;
  /** 仅当远端仍指向我们预期的提交时才强推（**没有**裸 force）。 */
  readonly forceWithLease?: boolean;
  readonly tags?: boolean;
  /** 推到不同名的远端分支（`<branch>:<remoteBranch>`）。 */
  readonly remoteBranch?: string | null;
  readonly dryRun?: boolean;
}

/** 长任务的创建结果。 */
export interface SyncJobRef {
  readonly jobId: string;
}

// ---------------------------------------------------------------- 命令

/** 拉取远端引用（长任务；不改本地历史，因此不快照）。 */
export function gitFetch(repoId: number, spec: FetchSpec = {}): Promise<SyncJobRef> {
  return invokeCommand<SyncJobRef>('git_fetch', { repoId, spec });
}

/**
 * 拉取并合并 / 变基（长任务）。
 *
 * 后端在**执行前**打 `PreSync` 快照；产生冲突时任务**成功结束**，
 * 结果里的 `merge.kind === 'conflicted'` 与 `merge.conflicts` 才是冲突的事实来源。
 */
export function gitPull(repoId: number, spec: PullSpec = {}): Promise<SyncJobRef> {
  return invokeCommand<SyncJobRef>('git_pull', { repoId, spec });
}

/**
 * 推送（长任务）。
 *
 * 被拒绝（non-fast-forward）时任务**失败结束**，错误码 `PUSH_REJECTED`
 * 且带三条修复动作；本模块只负责发起，动作的执行由同步条按 `action.id` 接线。
 */
export function gitPush(repoId: number, spec: PushSpec = {}): Promise<SyncJobRef> {
  return invokeCommand<SyncJobRef>('git_push', { repoId, spec });
}

/** 远端列表。错误：`NOT_FOUND`。 */
export function gitRemoteList(repoId: number): Promise<readonly Remote[]> {
  return invokeCommand<readonly Remote[]>('git_remote_list', { repoId });
}

/** 新增远端。错误：`VALIDATION`（名称或 URL 形状非法）。 */
export function gitRemoteAdd(repoId: number, name: string, url: string): Promise<void> {
  return invokeCommand<void>('git_remote_add', { repoId, name, url });
}

/** 删除远端（同时清理它的远端跟踪引用）。 */
export function gitRemoteRemove(repoId: number, name: string): Promise<void> {
  return invokeCommand<void>('git_remote_remove', { repoId, name });
}

/** 重命名远端。错误：`VALIDATION`（新名非法）。 */
export function gitRemoteRename(repoId: number, oldName: string, newName: string): Promise<void> {
  // 后端参数名是 `old` / `new`（`new` 是保留字，不能做形参名，只能做键）
  return invokeCommand<void>('git_remote_rename', { repoId, old: oldName, new: newName });
}

/** 改写远端 URL。错误：`VALIDATION`（URL 形状非法）。 */
export function gitRemoteSetUrl(repoId: number, name: string, url: string): Promise<void> {
  return invokeCommand<void>('git_remote_set_url', { repoId, name, url });
}

// ---------------------------------------------------------------- 纯函数（可单测）

/**
 * 把 `job:done` 的 `result`（`unknown`）收敛成同步结果。
 *
 * 事件载荷是 `unknown`：后端换了形状、或事件来自别的任务时，界面应该退化成
 * "没有结果可展示"，而不是抛错把整条同步流程搞崩。
 */
export function readSyncResult(result: unknown): SyncJobResult {
  if (typeof result !== 'object' || result === null) {
    return {};
  }
  return result as SyncJobResult;
}

/** 真正发生了变化的引用（`upToDate` 不算变化）。 */
export function changedUpdates(updates: readonly RefUpdate[]): readonly RefUpdate[] {
  return updates.filter((update) => update.kind !== 'upToDate');
}

/** 冲突文件清单（没有冲突时为空数组）。 */
export function pullConflicts(outcome: PullOutcome | undefined): readonly string[] {
  return outcome?.merge?.conflicts ?? [];
}
