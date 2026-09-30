/**
 * 合并命令（T3.4）：`git_merge_prepare` / `git_merge_execute` / `git_merge_continue`。
 *
 * 两段式契约：prepare 返回计划（预览），execute 只认 planId 且一次有效；
 * HEAD 变了会得到 `PLAN_STALE`。形状与 Rust 侧 `MergePlanDto` / `MergeOutcome`
 * 的 serde 契约一一对应（camelCase；`Option` 序列化为 **null** 不是 undefined）。
 */
import { invokeCommand } from './client';
import type { MergeKind } from './sync';

/** 合并策略（Rust 侧 `MergeStrategy` 的 camelCase）。 */
export type MergeStrategy = 'merge' | 'noFf' | 'squash' | 'fastForwardOnly' | 'ours' | 'theirs';

/** 快进裁决。 */
export type FfVerdict = 'upToDate' | 'fastForward' | 'trueMerge';

/** 计划里一条 source 独有提交。 */
export interface MergePlanCommit {
  readonly oid: string;
  readonly subject: string;
  readonly authorTime: number | null;
}

/** 合并计划（prepare 的产物；预览的全部内容）。 */
export interface MergePlan {
  readonly planId: string;
  readonly source: string;
  readonly strategy: MergeStrategy;
  readonly verdict: FfVerdict;
  readonly sourceOnlyCommits: readonly MergePlanCommit[];
  readonly sourceCommitCount: number;
  /** 预检是否可用（git 太旧时 false，界面退化为"执行后再报冲突"）。 */
  readonly previewAvailable: boolean;
  /** 预检发现的冲突文件。 */
  readonly conflicted: readonly string[];
  readonly defaultMessage: string;
  readonly equivalentCommand: string;
}

/** `git_merge_prepare` 的请求体。 */
export interface MergeRequest {
  readonly source: string;
  readonly strategy: MergeStrategy;
}

/** `git_merge_execute` 的请求体。 */
export interface MergeExecuteRequest {
  readonly planId: string;
  readonly message?: string;
}

/** 生成合并计划（只读预览；不碰工作区）。 */
export function gitMergePrepare(repoId: number, request: MergeRequest): Promise<MergePlan> {
  return invokeCommand<MergePlan>('git_merge_prepare', { repoId, spec: request });
}

/** 执行合并计划。冲突是结果不是错误（`kind === 'conflicted'`）。 */
export function gitMergeExecute(
  repoId: number,
  request: MergeExecuteRequest,
): Promise<{
  kind: MergeKind;
  oid: string | null;
  conflicts: readonly string[];
  snapshotId: number | null;
}> {
  return invokeCommand('git_merge_execute', { repoId, spec: request });
}

/** 冲突解决后的"继续合并"（可选编辑合并信息）。 */
export function gitMergeContinue(
  repoId: number,
  message?: string,
): Promise<{
  kind: MergeKind;
  oid: string | null;
  conflicts: readonly string[];
  snapshotId: number | null;
}> {
  return invokeCommand('git_merge_continue', {
    repoId,
    ...(message === undefined ? {} : { message }),
  });
}
