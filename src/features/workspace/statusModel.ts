//! 状态面板的纯模型：分组计数、树/扁平行展开（T1.4）。
//!
//! 与组件分离的理由：视图切换与目录聚合是纯函数，
//! Vitest 可以在没有渲染的情况下穷举边界（空目录、重命名、深层嵌套）。

import type { WorkspaceFileChange, WorkspaceStatus } from '@/lib/ipc/workspace';

export type { WorkspaceFileChange, WorkspaceStatus };

export type WorkspaceView = 'tree' | 'flat';

/** 四个面板分组（不含 ignored：它默认不参与面板操作）。 */
export interface StatusGroups {
  readonly conflicted: readonly WorkspaceFileChange[];
  readonly staged: readonly WorkspaceFileChange[];
  readonly unstaged: readonly WorkspaceFileChange[];
  readonly untracked: readonly WorkspaceFileChange[];
}

/** 提取四个分组（后端 DTO 已预拆分，这里只做聚合便于计数与选择模型）。 */
export function groupsOf(status: WorkspaceStatus): StatusGroups {
  return {
    conflicted: status.conflicted,
    staged: status.staged,
    unstaged: status.unstaged,
    untracked: status.untracked,
  };
}

/** 变更文件是否还有工作区文件（决定"放弃"与"大小"列的呈现）。 */
export function hasWorktreeFile(entry: WorkspaceFileChange): boolean {
  return entry.worktreeStatus !== 'D';
}

/** 文件的目录部分（根级文件为空串）。 */
export function dirOf(path: string): string {
  const index = path.lastIndexOf('/');
  return index === -1 ? '' : path.slice(0, index);
}

/** 树视图的目录行。 */
export interface DirectoryRow {
  readonly type: 'dir';
  readonly dir: string;
  readonly count: number;
}

/** 列表/树视图的文件行。 */
export interface FileRowData {
  readonly type: 'file';
  readonly entry: WorkspaceFileChange;
}

export type StatusRow = DirectoryRow | FileRowData;

/**
 * 把分组展开成 VirtualList 的行序列。
 *
 * - `flat`：原样列出文件行；
 * - `tree`：按目录聚合，目录行可折叠（`collapsed` 里的目录不展开其文件），
 *   目录顺序按"该目录首次出现的顺序"保持稳定，避免刷新时跳变。
 */
export function buildRows(
  entries: readonly WorkspaceFileChange[],
  view: WorkspaceView,
  collapsed: ReadonlySet<string>,
): readonly StatusRow[] {
  if (view === 'flat') {
    return entries.map((entry): StatusRow => ({ type: 'file', entry }));
  }

  const order: string[] = [];
  const byDir = new Map<string, WorkspaceFileChange[]>();
  for (const entry of entries) {
    const dir = dirOf(entry.path);
    if (!byDir.has(dir)) {
      order.push(dir);
      byDir.set(dir, []);
    }
    byDir.get(dir)!.push(entry);
  }

  const rows: StatusRow[] = [];
  for (const dir of order) {
    const group = byDir.get(dir)!;
    rows.push({ type: 'dir', dir, count: group.length });
    if (!collapsed.has(dir)) {
      for (const entry of group) {
        rows.push({ type: 'file', entry });
      }
    }
  }
  return rows;
}

/** 状态字符的展示名 key（文案在 shell.workspace.status.*，由调用方经 t() 渲染）。 */
export function statusLabelKey(entry: WorkspaceFileChange): string {
  if (entry.kind === 'untracked') return 'workspace.status.untracked';
  if (entry.kind === 'unmerged') return 'workspace.status.conflict';
  if (entry.indexStatus === 'R' || entry.worktreeStatus === 'R') return 'workspace.status.renamed';
  if (entry.worktreeStatus === 'A' || entry.indexStatus === 'A') return 'workspace.status.added';
  if (entry.worktreeStatus === 'D' || entry.indexStatus === 'D') return 'workspace.status.deleted';
  if (entry.worktreeStatus === 'M' || entry.indexStatus === 'M') return 'workspace.status.modified';
  if (entry.worktreeStatus === 'T' || entry.indexStatus === 'T')
    return 'workspace.status.typeChanged';
  if (entry.worktreeStatus === 'C' || entry.indexStatus === 'C') return 'workspace.status.copied';
  return 'workspace.status.unknown';
}

/** 状态字符对应的语义色 token 类（颜色之外仍有文字标签，见 statusLabel）。 */
export function statusToneClass(entry: WorkspaceFileChange): string {
  const status = entry.worktreeStatus !== '.' ? entry.worktreeStatus : entry.indexStatus;
  switch (status) {
    case 'A':
      return 'text-success';
    case 'D':
      return 'text-danger';
    case 'U':
      return 'text-warning';
    default:
      return 'text-info';
  }
}

/** 全局统计（工具条的计数徽标）。 */
export function countsOf(groups: StatusGroups): {
  staged: number;
  unstaged: number;
  untracked: number;
  conflicted: number;
} {
  return {
    staged: groups.staged.length,
    unstaged: groups.unstaged.length,
    untracked: groups.untracked.length,
    conflicted: groups.conflicted.length,
  };
}

/** 从状态报告收集全部变更路径（全选用）。 */
export function allPathsOf(status: WorkspaceStatus): string[] {
  return [...status.conflicted, ...status.staged, ...status.unstaged, ...status.untracked].map(
    (entry) => entry.path,
  );
}
