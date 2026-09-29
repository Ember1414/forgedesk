/**
 * 储藏与历史操作的 IPC 封装（T2.8）。
 *
 * # 契约要点（详见 docs/API.md「储藏与历史操作」）
 *
 * - **冲突是结果不是错误**：`git_stash_apply` / `git_stash_pop` / `git_cherry_pick` /
 *   `git_revert` 在冲突时正常返回，`conflicts` 里是冲突文件——界面据此把用户送到
 *   冲突页，而不是弹一个红色错误。
 * - **`stashed=false` 不是失败**：干净工作区点"储藏"就是这个结果。
 * - **重置是两段式**：先 `gitResetPrepare` 拿计划（决定将被丢弃的东西），
 *   确认后 `gitResetExecute(planId, confirmationWord)`。计划只能用一次，
 *   HEAD 变过即 `PLAN_STALE`——那种情况下重新预览即可，不是故障。
 * - **`git_stash_drop` / `git_stash_clear` 不可逆**：返回值里的 `dropped` 带 oid
 *   （`gc` 回收之前还能按它找回），界面要在确认框里展示将丢掉什么。
 */
import { invokeCommand } from './client';

/** 一条 stash 记录。 */
export interface StashEntry {
  /** `stash@{n}` 里的 n（列表里的位置，任何 stash 操作都会重排）。 */
  readonly index: number;
  /** 这条 stash 提交的 oid（长期引用用它，不用 index）。 */
  readonly oid: string;
  /** 它基于哪个提交（diff 的比较对象）。 */
  readonly baseOid?: string;
  /** 描述信息。 */
  readonly message: string;
  /** 创建时间（Unix 秒）。 */
  readonly createdAt?: number;
  /** 是否包含未跟踪文件（`-u` 创建的）。 */
  readonly includesUntracked: boolean;
  /** 未跟踪文件所在的第三个父提交（只有 `-u` 创建的才有）。 */
  readonly untrackedOid?: string;
}

/** `git stash` 的子操作结果（冲突时 `conflicts` 非空）。 */
export interface StashOutcome {
  /** 冲突的路径（相对仓库根）。 */
  readonly conflicts: readonly string[];
}

/** 储藏的结果。 */
export interface StashSaveOutcome {
  /** 是否真的产生了新条目（`false` = 没有可储藏的内容，不是失败）。 */
  readonly stashed: boolean;
  /** 新产生的那条。 */
  readonly entry?: StashEntry;
}

/** 丢弃的结果（`dropped` 里的 oid 是唯一能找回的线索）。 */
export interface StashDiscardOutcome {
  readonly dropped: readonly StashEntry[];
}

/** 某条 stash 的 diff：相对 base 的变更 +（`-u` 时）未跟踪文件那份。 */
export interface StashShowOutcome {
  readonly entry: StashEntry;
  /** 相对 base 的变更（**不含**未跟踪文件——它们在第三个父提交里）。 */
  readonly diff: unknown;
  /** 未跟踪文件的变更（只有 `-u` 创建的 stash 才有）。 */
  readonly untracked?: unknown;
}

/** 计划里的一条提交摘要。 */
export interface CommitSummary {
  readonly oid: string;
  readonly subject: string;
  readonly authorTime?: number;
}

/** 被丢弃的提交与远端的关系。 */
export interface ResetRemoteImpact {
  /** 当前分支的上游（没有上游时为 `null`：所有提交都只存在于本地）。 */
  readonly upstream: string | null;
  /** 将被丢弃、且**远端也找不到**的提交数。 */
  readonly notOnRemote: number;
}

/** 重置的影响摘要（执行前的预览）。 */
export interface ResetPlan {
  readonly planId: string;
  readonly mode: 'soft' | 'mixed' | 'hard';
  readonly targetOid: string;
  readonly targetSubject: string;
  /** 计划生成时的 HEAD（与执行时不一致即 `PLAN_STALE`）。 */
  readonly headBefore: string;
  /** 将被丢弃的提交（最多 30 条，按时间从新到旧）。 */
  readonly discarded: readonly CommitSummary[];
  readonly discardedTruncated: boolean;
  /** 将被丢弃的提交总数（精确值）。 */
  readonly discardedCount: number;
  /** 将被丢弃的已暂存改动。 */
  readonly lostStaged: readonly unknown[];
  /** 将被丢弃的工作区改动（仅 `--hard`）。 */
  readonly lostWorktree: readonly unknown[];
  /** 会被覆盖的未跟踪文件（仅 `--hard`）。 */
  readonly untrackedToRemove: readonly string[];
  readonly remote: ResetRemoteImpact;
  /** 是否需要输入确认词。 */
  readonly requiresConfirmation: boolean;
  /** 需要输入的确认词（后端给出，界面照着展示）。 */
  readonly confirmationWord?: string;
  /** 执行前是否必须打快照。 */
  readonly snapshotRequired: boolean;
}

/** 重置的执行结果。 */
export interface ResetOutcome {
  readonly mode: ResetPlan['mode'];
  readonly headBefore: string;
  readonly headAfter: string;
  readonly discardedCount: number;
  readonly snapshotId?: number;
}

/** 一条 reflog 记录。 */
export interface ReflogEntry {
  /** `HEAD@{n}` 里的 n。 */
  readonly index: number;
  /** 这次移动之后指向的提交。 */
  readonly oid: string;
  /** 哪个引用的日志（缺省 `HEAD`）。 */
  readonly reference?: string;
  /** 操作说明（checkout / commit / reset …）。 */
  readonly message: string;
}

/** 储藏当前改动。 */
export function gitStashSave(
  repoId: number,
  spec: {
    readonly message?: string;
    readonly includeUntracked?: boolean;
    readonly keepIndex?: boolean;
    readonly paths?: readonly string[];
  },
): Promise<StashSaveOutcome> {
  return invokeCommand<StashSaveOutcome>('git_stash_save', {
    repoId,
    spec: {
      message: spec.message ?? null,
      includeUntracked: spec.includeUntracked ?? false,
      keepIndex: spec.keepIndex ?? false,
      paths: spec.paths ?? [],
    },
  });
}

/** stash 列表（新的在前）。 */
export function gitStashList(repoId: number): Promise<readonly StashEntry[]> {
  return invokeCommand<readonly StashEntry[]>('git_stash_list', { repoId });
}

/** 某条 stash 的 diff（相对 base + 未跟踪文件）。 */
export function gitStashShow(repoId: number, index: number): Promise<StashShowOutcome> {
  return invokeCommand<StashShowOutcome>('git_stash_show', { repoId, index });
}

/** 应用某条 stash 并保留它（冲突时 `conflicts` 非空，该条仍在）。 */
export function gitStashApply(
  repoId: number,
  index: number,
  restoreIndex = false,
): Promise<StashOutcome> {
  return invokeCommand<StashOutcome>('git_stash_apply', {
    repoId,
    spec: { index, restoreIndex },
  });
}

/** 应用某条 stash 并删除它（冲突时**不会**删除）。 */
export function gitStashPop(
  repoId: number,
  index: number,
  restoreIndex = false,
): Promise<StashOutcome> {
  return invokeCommand<StashOutcome>('git_stash_pop', {
    repoId,
    spec: { index, restoreIndex },
  });
}

/** 丢弃某条 stash（不可逆）。 */
export function gitStashDrop(repoId: number, index: number): Promise<StashDiscardOutcome> {
  return invokeCommand<StashDiscardOutcome>('git_stash_drop', { repoId, index });
}

/** 丢弃全部 stash（不可逆）。 */
export function gitStashClear(repoId: number): Promise<StashDiscardOutcome> {
  return invokeCommand<StashDiscardOutcome>('git_stash_clear', { repoId });
}

/** 从某条 stash 创建分支并应用它（会切换分支；pop 冲突时的正规出路）。 */
export function gitStashBranch(
  repoId: number,
  index: number,
  name: string,
): Promise<{ readonly branch: string }> {
  return invokeCommand('git_stash_branch', { repoId, spec: { index, name } });
}

/** 拣选提交（`revision` 可以是区间）。冲突时返回 `MergeOutcome`。 */
export function gitCherryPick(
  repoId: number,
  spec: { readonly revision: string; readonly recordSource?: boolean; readonly noCommit?: boolean },
): Promise<unknown> {
  return invokeCommand('git_cherry_pick', {
    repoId,
    spec: {
      revision: spec.revision,
      recordSource: spec.recordSource ?? false,
      noCommit: spec.noCommit ?? false,
    },
  });
}

/** 反转提交（合并提交必须给 `mainline`）。 */
export function gitRevert(
  repoId: number,
  spec: {
    readonly revision: string;
    readonly mainline?: number;
    readonly noCommit?: boolean;
  },
): Promise<unknown> {
  return invokeCommand('git_revert', {
    repoId,
    spec: {
      revision: spec.revision,
      mainline: spec.mainline ?? null,
      noCommit: spec.noCommit ?? false,
    },
  });
}

/** 生成重置计划（只读，不写仓库）。 */
export function gitResetPrepare(
  repoId: number,
  spec: { readonly revision: string; readonly mode: ResetPlan['mode'] },
): Promise<ResetPlan> {
  return invokeCommand<ResetPlan>('git_reset_prepare', { repoId, spec });
}

/** 执行重置计划（`--hard` 必须带计划给出的确认词）。 */
export function gitResetExecute(
  repoId: number,
  planId: string,
  confirmation?: string,
): Promise<ResetOutcome> {
  return invokeCommand<ResetOutcome>('git_reset_execute', {
    repoId,
    planId,
    confirmation: confirmation ?? null,
  });
}

/** reflog（新的在前）。 */
export function gitReflog(repoId: number, limit = 100): Promise<readonly ReflogEntry[]> {
  return invokeCommand<readonly ReflogEntry[]>('git_reflog', { repoId, limit });
}

/** 把 reflog 里的某一条恢复成**新分支**（不移动任何现有引用）。 */
export function gitReflogCreateBranch(
  repoId: number,
  index: number,
  name: string,
): Promise<{ readonly branch: string }> {
  return invokeCommand('git_reflog_create_branch', { repoId, index, name });
}
