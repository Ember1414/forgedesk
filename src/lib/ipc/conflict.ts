/**
 * 冲突状态机命令（T3.1）：`git_conflict_*`。
 *
 * 形状与 Rust 侧 `forgedesk_domain::git::conflict` 的 serde 契约一一对应
 * （camelCase；`Option` 序列化为 **null** 不是 undefined——判断一律按 null 设计，
 * 见 `StashPanel` 的教训）。
 */
import { invokeCommand } from './client';

/** 冲突来源的操作类型（Rust 侧 `ConflictOpKind` 的 camelCase）。 */
export type ConflictOpKind = 'merge' | 'rebase' | 'cherryPick' | 'revert';

/** 冲突文件的类别（由 index stage 的存在性推导，与工作区标记无关）。 */
export type ConflictKind =
  'text' | 'binary' | 'deletedByUs' | 'deletedByThem' | 'addedByBoth' | 'addedByUs' | 'addedByThem';

/** 一个 stage 上的 blob 快照；`content` 为 null = 二进制、非 UTF-8 或超过 2 MiB。 */
export interface ConflictBlob {
  readonly size: number;
  readonly isBinary: boolean;
  readonly encodingHint: string | null;
  readonly content: string | null;
}

/** 一个冲突文件的三方版本。 */
export interface ConflictFile {
  readonly path: string;
  readonly kind: ConflictKind;
  readonly base: ConflictBlob | null;
  readonly ours: ConflictBlob | null;
  readonly theirs: ConflictBlob | null;
  readonly worktreeExists: boolean;
}

/** 冲突状态（`files` 是仍未解决的清单；全部消失时 `canContinue` 为真）。 */
export interface ConflictState {
  readonly opKind: ConflictOpKind | null;
  readonly opInProgress: boolean;
  readonly currentStep: number | null;
  readonly totalSteps: number | null;
  readonly headName: string | null;
  readonly intoBranch: string | null;
  readonly files: readonly ConflictFile[];
  readonly canContinue: boolean;
  readonly canAbort: boolean;
  readonly canSkip: boolean;
}

/** continue / skip 的结果：`conflicts` 非空 = 又停在了新的冲突上（正常结果）。 */
export interface ConflictContinueOutcome {
  readonly oid: string | null;
  readonly conflicts: readonly string[];
}

/** abort 的结果；`snapshotId` 关联中止前的 PreHeadMove 快照。 */
export interface ConflictAbortOutcome {
  readonly headOid: string | null;
  readonly headRef: string | null;
  readonly snapshotId: number | null;
}

/** 采集冲突状态。只读探测：无进行中操作时返回空态（`opKind: null`）。 */
export function gitConflictState(repoId: number): Promise<ConflictState> {
  return invokeCommand<ConflictState>('git_conflict_state', { repoId });
}

/** 标记文件已解决（后端 `git add` 并校验 stage 清空）。 */
export function gitConflictMarkResolved(repoId: number, paths: readonly string[]): Promise<void> {
  return invokeCommand<void>('git_conflict_mark_resolved', { repoId, paths: [...paths] });
}

/** 继续进行中的操作；再次停在冲突上是正常结果，不是错误。 */
export function gitConflictContinue(repoId: number): Promise<ConflictContinueOutcome> {
  return invokeCommand<ConflictContinueOutcome>('git_conflict_continue', { repoId });
}

/** 中止进行中的操作（后端先打快照再 abort 并校验回滚）。 */
export function gitConflictAbort(repoId: number): Promise<ConflictAbortOutcome> {
  return invokeCommand<ConflictAbortOutcome>('git_conflict_abort', { repoId });
}

/** 跳过当前提交（只有 rebase 支持）。 */
export function gitConflictSkip(repoId: number): Promise<ConflictContinueOutcome> {
  return invokeCommand<ConflictContinueOutcome>('git_conflict_skip', { repoId });
}
