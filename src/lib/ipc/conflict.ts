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

/** 工作区文件的换行风格（写回时保持原文件形状）。 */
export type LineEnding = 'lf' | 'crlf' | 'cr';

/** "整个文件采用一方"的动作（二进制 / 删除类冲突的解决路径）。 */
export type TakeSide = 'ours' | 'theirs';

/** diff3 合并块（Rust 侧 `MergeBlock`，tag = type）。 */
export type MergeBlock =
  | { readonly type: 'context'; readonly lines: readonly string[] }
  | {
      readonly type: 'resolved';
      readonly lines: readonly string[];
      readonly source: 'ours' | 'theirs' | 'both';
    }
  | {
      readonly type: 'conflict';
      readonly base: readonly string[];
      readonly ours: readonly string[];
      readonly theirs: readonly string[];
    };

/** 单个冲突文件的完整详情（三方 blob + 工作区形状 + 合并块）。 */
export interface ConflictFileDetail {
  readonly path: string;
  readonly kind: ConflictKind;
  readonly base: ConflictBlob | null;
  readonly ours: ConflictBlob | null;
  readonly theirs: ConflictBlob | null;
  readonly worktreeExists: boolean;
  readonly eol: LineEnding;
  readonly bom: boolean;
  readonly trailingNewline: boolean;
  readonly blocks: readonly MergeBlock[];
}

/** 读取单个冲突文件的详情（打开编辑器时才调用）。 */
export function gitConflictFileDetail(repoId: number, path: string): Promise<ConflictFileDetail> {
  return invokeCommand<ConflictFileDetail>('git_conflict_file_detail', { repoId, path });
}

/** `git_conflict_apply_resolution` 的请求体。 */
export interface ApplyResolutionRequest {
  /** 编辑器产出的完整结果文本（LF 换行；EOL 由后端按原文件形状重建）。 */
  readonly content: string;
  readonly eol: LineEnding;
  readonly bom: boolean;
  readonly trailingNewline: boolean;
}

/** 写回结果文本并标记已解决（后端保持原文件的 EOL / BOM / 末尾换行）。 */
export function gitConflictApplyResolution(
  repoId: number,
  path: string,
  spec: ApplyResolutionRequest,
): Promise<void> {
  return invokeCommand<void>('git_conflict_apply_resolution', { repoId, path, spec });
}

/** 整个文件采用一方（二进制 / 删除类冲突）。 */
export function gitConflictTakeSide(repoId: number, path: string, side: TakeSide): Promise<void> {
  return invokeCommand<void>('git_conflict_take_side', { repoId, path, side });
}

/** 以"删除该文件"解决删除类冲突。 */
export function gitConflictRemoveFile(repoId: number, path: string): Promise<void> {
  return invokeCommand<void>('git_conflict_remove_file', { repoId, path });
}

/** 跳过当前提交（只有 rebase 支持）。 */
export function gitConflictSkip(repoId: number): Promise<ConflictContinueOutcome> {
  return invokeCommand<ConflictContinueOutcome>('git_conflict_skip', { repoId });
}
