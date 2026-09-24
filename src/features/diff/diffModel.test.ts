import { describe, expect, it } from 'vitest';

import {
  changedLineCount,
  contextLineCount,
  flattenUnified,
  modifiedPairs,
  modifiedPairsUnified,
  pairSideBySide,
} from '@/features/diff/diffModel';
import type { DiffHunk, DiffLine } from '@/lib/ipc/workspace';

function line(kind: DiffLine['kind'], content: string, oldNo?: number, newNo?: number): DiffLine {
  return { kind, content, oldNo: oldNo ?? null, newNo: newNo ?? null };
}

const HUNK: DiffHunk = {
  oldStart: 1,
  oldLines: 4,
  newStart: 1,
  newLines: 4,
  header: 'fn main() {',
  lines: [
    line('context', 'a', 1, 1),
    line('removed', 'old 1', 2),
    line('removed', 'old 2', 3),
    line('added', 'new 1', undefined, 2),
    line('added', 'new 2', undefined, 3),
    line('context', 'b', 4, 4),
  ],
};

describe('flattenUnified（内联）', () => {
  it('按 hunk 顺序展开全部行并保留所属序号', () => {
    const rows = flattenUnified([HUNK]);
    expect(rows).toHaveLength(6);
    expect(rows[0]?.line.content).toBe('a');
    expect(rows[2]?.line.content).toBe('old 2');
    expect(rows[5]?.line.content).toBe('b');
    expect(rows.every((row) => row.hunkIndex === 0)).toBe(true);
  });
});

describe('pairSideBySide（并排）', () => {
  it('上下文行左右成对；等长增删运行 1:1 拉齐（2+2 行合成 2 个对行）', () => {
    const rows = pairSideBySide([HUNK]);
    expect(rows).toHaveLength(4);
    expect(rows[0]).toMatchObject({ left: { content: 'a' }, right: { content: 'a' } });
    expect(rows[1]).toMatchObject({ left: { content: 'old 1' }, right: { content: 'new 1' } });
    expect(rows[2]).toMatchObject({ left: { content: 'old 2' }, right: { content: 'new 2' } });
    expect(rows[3]).toMatchObject({ left: { content: 'b' }, right: { content: 'b' } });
  });

  it('不等长的运行用 null 补齐短侧', () => {
    const hunk: DiffHunk = {
      oldStart: 1,
      oldLines: 3,
      newStart: 1,
      newLines: 2,
      header: '',
      lines: [line('removed', 'a', 1), line('removed', 'b', 2), line('added', 'x', undefined, 1)],
    };
    const rows = pairSideBySide([hunk]);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({ left: { content: 'a' }, right: { content: 'x' } });
    expect(rows[1]).toMatchObject({ left: { content: 'b' }, right: null });
  });

  it('纯新增文件（无旧侧）每行右满左空', () => {
    const hunk: DiffHunk = {
      oldStart: 0,
      oldLines: 0,
      newStart: 1,
      newLines: 2,
      header: '',
      lines: [line('added', 'x', undefined, 1), line('added', 'y', undefined, 2)],
    };
    const rows = pairSideBySide([hunk]);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({ left: null, right: { content: 'x' } });
  });
});

describe('字符级高亮的门槛与配对', () => {
  it('变更行数按 added+removed 计', () => {
    expect(changedLineCount([HUNK])).toBe(4);
  });

  it('并排视图里左删右加是修改对', () => {
    const pairs = modifiedPairs(pairSideBySide([HUNK]));
    expect(pairs.get(1)).toEqual(['old 1', 'new 1']);
    expect(pairs.has(0)).toBe(false);
  });

  it('内联视图里相邻的删除-新增是修改对且一对只配一次', () => {
    const pairs = modifiedPairsUnified(flattenUnified([HUNK]));
    // 相邻规则：最后一行删除（old 2）与第一行新增（new 1）成对
    expect(pairs.get(2)).toEqual(['old 2', 'new 1']);
    expect(pairs.has(1)).toBe(false); // old 1 的相邻行也是删除，不成对
  });
});

describe('contextLineCount', () => {
  it('只数上下文行', () => {
    expect(contextLineCount(HUNK)).toBe(2);
  });
});
