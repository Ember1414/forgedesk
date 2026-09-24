//! 工作区 IPC 封装（T1.4）：状态读取与文件级写操作 + `repo:changed` 订阅。
//!
//! 类型与后端 `crates/commands/src/workspace.rs` 的 DTO 一一对应（camelCase）；
//! 字段含义见 docs/API.md 的登记表。

import { invokeCommand, listenEvent } from '@/lib/ipc/client';
import type { Unlisten } from '@/lib/ipc/client';

/** 单条文件变更（后端已按面板分组，字段含义见 docs/API.md）。 */
export interface WorkspaceFileChange {
  readonly path: string;
  readonly oldPath?: string | null;
  /** `ordinary` / `renamed-or-copied` / `unmerged` / `untracked` / `ignored`。 */
  readonly kind: string;
  /** 索引侧状态字符（`.` 表示无变更）。 */
  readonly indexStatus: string;
  /** 工作区侧状态字符。 */
  readonly worktreeStatus: string;
  readonly isBinary: boolean;
  readonly isLfs: boolean;
  readonly isSubmodule: boolean;
  readonly sizeBytes?: number | null;
}

/** 分支头信息。 */
export interface WorkspaceBranch {
  readonly oid?: string | null;
  readonly head?: string | null;
  readonly detached: boolean;
  readonly upstream?: string | null;
  readonly ahead?: number | null;
  readonly behind?: number | null;
}

/** 工作区状态报告（后端已按分组预拆分）。 */
export interface WorkspaceStatus {
  readonly branch: WorkspaceBranch;
  /** `none` / `merge` / `rebase` / `cherry-pick` / `revert` / `bisect`。 */
  readonly operation: string;
  readonly staged: readonly WorkspaceFileChange[];
  readonly unstaged: readonly WorkspaceFileChange[];
  readonly untracked: readonly WorkspaceFileChange[];
  readonly conflicted: readonly WorkspaceFileChange[];
  readonly ignored: readonly WorkspaceFileChange[];
  readonly ignoredCount?: number | null;
}

/** `repo:changed` 事件载荷。 */
export interface RepoChangedPayload {
  readonly repoId: number;
  readonly paths: readonly string[];
}

/** 读取工作区状态；`includeIgnored` 时额外返回被忽略文件与计数。 */
export function workspaceStatus(repoId: number, includeIgnored = false): Promise<WorkspaceStatus> {
  return invokeCommand<WorkspaceStatus>('workspace_status', { repoId, includeIgnored });
}

/** 暂存路径。 */
export function workspaceStage(repoId: number, paths: readonly string[]): Promise<void> {
  return invokeCommand<void>('workspace_stage', { repoId, paths: [...paths] });
}

/** 取消暂存路径。 */
export function workspaceUnstage(repoId: number, paths: readonly string[]): Promise<void> {
  return invokeCommand<void>('workspace_unstage', { repoId, paths: [...paths] });
}

/** 放弃工作区修改（tracked 可恢复；untracked 是磁盘删除）。 */
export function workspaceDiscard(
  repoId: number,
  tracked: readonly string[],
  untracked: readonly string[],
): Promise<void> {
  return invokeCommand<void>('workspace_discard', {
    repoId,
    tracked: [...tracked],
    untracked: [...untracked],
  });
}

/** 在系统文件管理器中显示指定路径（打开其父目录）。 */
export function workspaceReveal(repoId: number, path: string): Promise<void> {
  return invokeCommand<void>('workspace_reveal', { repoId, path });
}

/** 订阅数据变化广播（自己的操作与 T1.10 的文件监听共用同一事件）。 */
export function onRepoChanged(handler: (payload: RepoChangedPayload) => void): Promise<Unlisten> {
  return listenEvent<RepoChangedPayload>('repo:changed', handler);
}

// ---------------------------------------------------------------- diff（T1.5）

/** hunk 内一行的类别（与后端 DiffLineKind 对应）。 */
export type DiffLineKind = 'context' | 'added' | 'removed' | 'noNewline';

/** hunk 内一行（后端已解析统一补丁）。 */
export interface DiffLine {
  readonly kind: DiffLineKind;
  readonly content: string;
  readonly oldNo?: number | null;
  readonly newNo?: number | null;
}

/** 一个 hunk。 */
export interface DiffHunk {
  readonly oldStart: number;
  readonly oldLines: number;
  readonly newStart: number;
  readonly newLines: number;
  readonly header: string;
  readonly lines: readonly DiffLine[];
}

/** 单个文件的行级 diff。 */
export interface FileDiff {
  readonly path: string;
  readonly oldPath?: string | null;
  readonly change: string;
  readonly binary: boolean;
  readonly additions: number;
  readonly deletions: number;
  /** 行级内容被截断（大文件保护；配合 forceFull 重新请求）。 */
  readonly truncated: boolean;
  readonly hunks: readonly DiffHunk[];
}

/** 一次 diff 查询的结果。 */
export interface DiffReport {
  readonly files: readonly FileDiff[];
  readonly truncatedFiles: number;
}

/** diff 查询条件（camelCase，与后端 DiffRequest 对应）。 */
export interface DiffRequest {
  readonly target: 'staged' | 'unstaged' | 'between' | 'since' | 'commit';
  readonly from?: string;
  readonly to?: string;
  readonly revision?: string;
  readonly paths?: readonly string[];
  readonly ignoreWhitespace?: boolean;
  readonly contextLines?: number;
  readonly detectRenames?: boolean;
  /** 跳过大文件截断（加载完整 diff）。 */
  readonly forceFull?: boolean;
}

/** 读取行级 diff（文本补丁由后端 git CLI 生成，与用户终端一致）。 */
export function workspaceDiff(repoId: number, spec: DiffRequest): Promise<DiffReport> {
  return invokeCommand<DiffReport>('workspace_diff', { repoId, spec });
}

/**
 * 生成原始补丁字节（复制 / 导出 .patch）。
 *
 * 返回字节数组而不是字符串：补丁里的路径与内容都可能是非 UTF-8，
 * 前端按 UTF-8 解码显示（解码失败的路径占位符与 git 终端行为一致）。
 */
export function workspaceDiffPatch(repoId: number, spec: DiffRequest): Promise<number[]> {
  return invokeCommand<number[]>('workspace_diff_patch', { repoId, spec });
}
