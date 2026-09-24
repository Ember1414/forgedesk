import { describe, expect, it } from 'vitest';

import type { DiffHunk, DiffLine, DiffLineKind } from '@/lib/ipc/workspace';
import {
  countSelected,
  emptySelection,
  isHunkFullySelected,
  isSelectable,
  selectableLines,
  selectRange,
  selectedHunkIndices,
  toLineSelections,
  toggleHunk,
  toggleLine,
} from '@/features/diff/selectionModel';

/** 用前缀字符构造 hunk：` ` 上下文、`+` 新增、`-` 删除、`\` 末尾无换行标记。 */
function hunk(prefixes: readonly (' ' | '+' | '-' | '\\')[]): DiffHunk {
  const lines: DiffLine[] = prefixes.map((prefix, index) => {
    const kind: DiffLineKind =
      prefix === '+'
        ? 'added'
        : prefix === '-'
          ? 'removed'
          : prefix === '\\'
            ? 'noNewline'
            : 'context';
    return {
      kind,
      content: `${prefix}line-${index}`,
      oldNo: kind === 'added' || kind === 'noNewline' ? null : index + 1,
      newNo: kind === 'removed' || kind === 'noNewline' ? null : index + 1,
    };
  });
  return {
    oldStart: 1,
    oldLines: lines.length,
    newStart: 1,
    newLines: lines.length,
    header: '',
    lines,
  };
}

const TWO_HUNKS: readonly DiffHunk[] = [
  hunk([' ', '-', '+', ' ']),
  // 第二个块里放一个 `\ No newline` 标记：它不参与选择，但会占据一个位置，
  // 因此后续行的下标会跟着后移（这正是最容易算错的地方）
  hunk([' ', '\\', '-', '+']),
];

describe('diff 选择模型', () => {
  it('只有新增与删除行能被选中', () => {
    expect(isSelectable('added')).toBe(true);
    expect(isSelectable('removed')).toBe(true);
    expect(isSelectable('context')).toBe(false);
    expect(isSelectable('noNewline')).toBe(false);
  });

  it('展平出的可选中行跳过上下文行与末尾无换行标记', () => {
    const lines = selectableLines(TWO_HUNKS);

    expect(lines).toEqual([
      { hunkIndex: 0, lineIndex: 1 },
      { hunkIndex: 0, lineIndex: 2 },
      { hunkIndex: 1, lineIndex: 2 },
      { hunkIndex: 1, lineIndex: 3 },
    ]);
  });

  it('切换一行后计数加一，再次切换后选择重新为空', () => {
    const once = toggleLine(emptySelection(), { hunkIndex: 0, lineIndex: 1 });
    expect(countSelected(once)).toBe(1);

    const twice = toggleLine(once, { hunkIndex: 0, lineIndex: 1 });
    expect(countSelected(twice)).toBe(0);
    expect(toLineSelections(twice)).toEqual([]);
  });

  it('范围选择包含两端，并且可以跨块', () => {
    const selection = selectRange(
      TWO_HUNKS,
      emptySelection(),
      { hunkIndex: 0, lineIndex: 2 },
      {
        hunkIndex: 1,
        lineIndex: 3,
      },
    );

    expect(countSelected(selection)).toBe(3);
    expect(toLineSelections(selection)).toEqual([
      { hunkIndex: 0, lines: [2] },
      { hunkIndex: 1, lines: [2, 3] },
    ]);
  });

  it('范围选择反向拖拽与正向等价', () => {
    const forward = selectRange(
      TWO_HUNKS,
      emptySelection(),
      { hunkIndex: 0, lineIndex: 1 },
      {
        hunkIndex: 1,
        lineIndex: 2,
      },
    );
    const backward = selectRange(
      TWO_HUNKS,
      emptySelection(),
      { hunkIndex: 1, lineIndex: 2 },
      {
        hunkIndex: 0,
        lineIndex: 1,
      },
    );

    expect(toLineSelections(forward)).toEqual(toLineSelections(backward));
  });

  it('整块切换在全选与清空之间往返', () => {
    const selected = toggleHunk(emptySelection(), TWO_HUNKS, 0);
    expect(isHunkFullySelected(selected, TWO_HUNKS, 0)).toBe(true);
    expect(countSelected(selected)).toBe(2);

    const cleared = toggleHunk(selected, TWO_HUNKS, 0);
    expect(isHunkFullySelected(cleared, TWO_HUNKS, 0)).toBe(false);
    expect(countSelected(cleared)).toBe(0);
  });

  it('半选状态下点击整块会补齐该块其余的行', () => {
    const half = toggleLine(emptySelection(), { hunkIndex: 0, lineIndex: 1 });
    expect(isHunkFullySelected(half, TWO_HUNKS, 0)).toBe(false);

    const full = toggleHunk(half, TWO_HUNKS, 0);
    expect(isHunkFullySelected(full, TWO_HUNKS, 0)).toBe(true);
    expect(countSelected(full)).toBe(2);
  });

  it('转成 IPC 形状时下标按升序稳定输出', () => {
    let selection = toggleLine(emptySelection(), { hunkIndex: 1, lineIndex: 3 });
    selection = toggleLine(selection, { hunkIndex: 0, lineIndex: 2 });
    selection = toggleLine(selection, { hunkIndex: 1, lineIndex: 2 });
    selection = toggleLine(selection, { hunkIndex: 0, lineIndex: 1 });

    expect(toLineSelections(selection)).toEqual([
      { hunkIndex: 0, lines: [1, 2] },
      { hunkIndex: 1, lines: [2, 3] },
    ]);
    expect(selectedHunkIndices(selection)).toEqual([0, 1]);
  });

  it('空选择的计数与转换结果都是空的', () => {
    const selection = emptySelection();

    expect(countSelected(selection)).toBe(0);
    expect(selectedHunkIndices(selection)).toEqual([]);
    expect(toLineSelections(selection)).toEqual([]);
  });
});
