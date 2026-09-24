//! 状态面板纯模型的测试（T1.4）。

import { describe, expect, it } from 'vitest';

import {
  allPathsOf,
  buildRows,
  countsOf,
  dirOf,
  groupsOf,
  statusLabelKey,
  statusToneClass,
} from '@/features/workspace/statusModel';
import type { WorkspaceFileChange, WorkspaceStatus } from '@/lib/ipc/workspace';

function change(path: string, kind: string, index: string, worktree: string): WorkspaceFileChange {
  return {
    path,
    kind,
    indexStatus: index,
    worktreeStatus: worktree,
    isBinary: false,
    isLfs: false,
    isSubmodule: false,
    sizeBytes: 10,
  };
}

const STATUS: WorkspaceStatus = {
  branch: {
    oid: 'abc',
    head: 'main',
    detached: false,
    upstream: 'origin/main',
    ahead: 1,
    behind: 0,
  },
  operation: 'none',
  staged: [change('staged.txt', 'ordinary', 'A', '.')],
  unstaged: [change('dirty.txt', 'ordinary', '.', 'M')],
  untracked: [change('untracked.txt', 'untracked', '?', '?')],
  conflicted: [change('conflict.txt', 'unmerged', 'U', 'U')],
  ignored: [],
  ignoredCount: null,
};

describe('groupsOf / countsOf', () => {
  it('透传后端预拆分的分组', () => {
    const groups = groupsOf(STATUS);
    expect(groups.staged).toHaveLength(1);
    expect(groups.unstaged).toHaveLength(1);
    expect(groups.untracked).toHaveLength(1);
    expect(groups.conflicted).toHaveLength(1);
    expect(countsOf(groups)).toEqual({ staged: 1, unstaged: 1, untracked: 1, conflicted: 1 });
  });
});

describe('dirOf', () => {
  it('根级文件目录为空串，其余取到最后一个斜杠', () => {
    expect(dirOf('a.txt')).toBe('');
    expect(dirOf('src/main.ts')).toBe('src');
    expect(dirOf('a/b/c.txt')).toBe('a/b');
  });
});

describe('buildRows', () => {
  const entries = [change('a.txt', 'ordinary', '.', 'M'), change('src/b.ts', 'ordinary', '.', 'M')];

  it('扁平视图：一行一个文件', () => {
    const rows = buildRows(entries, 'flat', new Set());
    expect(rows).toHaveLength(2);
    expect(rows.every((row) => row.type === 'file')).toBe(true);
  });

  it('树视图：先目录行后文件行，折叠目录只留目录行', () => {
    const expanded = buildRows(entries, 'tree', new Set());
    expect(expanded.map((row) => row.type)).toEqual(['dir', 'file', 'dir', 'file']);

    const collapsed = buildRows(entries, 'tree', new Set(['src']));
    const types = collapsed.map((row) => row.type);
    expect(types).toContain('dir');
    expect(collapsed.some((row) => row.type === 'file' && row.entry.path === 'src/b.ts')).toBe(
      false,
    );
  });

  it('树视图目录顺序按首次出现（刷新不跳变）', () => {
    const reordered = [...entries].reverse();
    expect(
      buildRows(reordered, 'tree', new Set())
        .filter((row) => row.type === 'dir')
        .map((row) => row.dir),
    ).toEqual(['src', '']);
  });
});

describe('statusLabel / statusToneClass', () => {
  it('未跟踪与冲突有独立文案', () => {
    expect(statusLabelKey(change('x', 'untracked', '?', '?'))).toBe('workspace.status.untracked');
    expect(statusLabelKey(change('x', 'unmerged', 'U', 'U'))).toBe('workspace.status.conflict');
    expect(statusLabelKey(change('x', 'ordinary', '.', 'M'))).toBe('workspace.status.modified');
    expect(statusLabelKey(change('x', 'renamed-or-copied', 'R', '.'))).toBe(
      'workspace.status.renamed',
    );
  });

  it('新增为 success 色，删除为 danger 色（颜色之外还有文字）', () => {
    expect(statusToneClass(change('x', 'untracked', '?', '?'))).toBe('text-info');
    expect(statusToneClass(change('x', 'ordinary', '.', 'D'))).toBe('text-danger');
  });
});

describe('allPathsOf', () => {
  it('收集四个分组的全部路径（全选用）', () => {
    expect(allPathsOf(STATUS)).toEqual([
      'conflict.txt',
      'staged.txt',
      'dirty.txt',
      'untracked.txt',
    ]);
  });
});
