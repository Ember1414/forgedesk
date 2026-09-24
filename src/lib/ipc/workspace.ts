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
