import { describe, expect, it } from 'vitest';

import type { RebasePreview, RebaseRangeCommit, ReorderAction } from '@/lib/ipc';
import {
  buildPreviewRows,
  computeIssues,
  initialEntries,
  isExecutable,
  mergeTargetFor,
  moveEntry,
  rangeFromSelection,
  setAction,
  toRequestEntries,
  type PlanEntry,
} from '@/features/rebase/planState';

const RANGE: readonly RebaseRangeCommit[] = [
  { oid: 'c1', subject: 'one', parents: ['base'], author: 'Fixture Author', authorTime: 1 },
  { oid: 'c2', subject: 'two', parents: ['c1'], author: 'Fixture Author', authorTime: 2 },
  { oid: 'c3', subject: 'three', parents: ['c2'], author: 'Fixture Author', authorTime: 3 },
];

function entry(oid: string, action: ReorderAction, extra: Partial<PlanEntry> = {}): PlanEntry {
  return {
    oid,
    subject: `subject-${oid}`,
    parents: ['p'],
    author: 'Fixture Author',
    authorTime: 1,
    action,
    ...extra,
  };
}

function previewOf(overrides: Partial<RebasePreview> = {}): RebasePreview {
  return {
    surviving: [],
    dropped: [],
    reworded: [],
    squashed: [],
    affectedCount: 0,
    touchesPushed: false,
    todoText: '',
    ...overrides,
  };
}

describe('initialEntries', () => {
  it('区间清单转成全 pick 的步骤（顺序保持从旧到新）', () => {
    const entries = initialEntries(RANGE);
    expect(entries.map((item) => item.oid)).toEqual(['c1', 'c2', 'c3']);
    expect(entries.every((item) => item.action === 'pick')).toBe(true);
  });

  it('父链是拷贝（面板修改不会污染历史页数据）', () => {
    const entries = initialEntries(RANGE);
    expect(entries[0]?.parents).toEqual(['base']);
  });
});

describe('moveEntry', () => {
  it('向下移动与向上移动都保持其余顺序', () => {
    const entries = initialEntries(RANGE);
    expect(moveEntry(entries, 0, 2).map((item) => item.oid)).toEqual(['c2', 'c3', 'c1']);
    expect(moveEntry(entries, 2, 0).map((item) => item.oid)).toEqual(['c3', 'c1', 'c2']);
  });

  it('越界与原地移动返回原引用（React 可跳过渲染）', () => {
    const entries = initialEntries(RANGE);
    expect(moveEntry(entries, 0, 0)).toBe(entries);
    expect(moveEntry(entries, -1, 1)).toBe(entries);
    expect(moveEntry(entries, 0, 3)).toBe(entries);
  });
});

describe('setAction', () => {
  it('reword 携带新信息草案', () => {
    const entries = setAction(initialEntries(RANGE), 1, 'reword', 'new two');
    expect(entries[1]).toEqual({
      oid: 'c2',
      subject: 'two',
      parents: ['c1'],
      author: 'Fixture Author',
      authorTime: 2,
      action: 'reword',
      newMessage: 'new two',
    });
  });

  it('切回 pick 时清掉旧草案（避免"幽灵"文案）', () => {
    const withMessage = setAction(initialEntries(RANGE), 1, 'reword', 'draft');
    const cleared = setAction(withMessage, 1, 'pick');
    expect(cleared[1]?.newMessage).toBeUndefined();
  });

  it('越界索引原样返回', () => {
    const entries = initialEntries(RANGE);
    expect(setAction(entries, 9, 'drop')).toBe(entries);
  });
});

describe('computeIssues', () => {
  it('全 pick 合法', () => {
    expect(computeIssues(initialEntries(RANGE))).toEqual([]);
    expect(isExecutable(initialEntries(RANGE))).toBe(true);
  });

  it('第一条不能是 squash / fixup', () => {
    for (const action of ['squash', 'fixup'] as const) {
      const entries = [entry('c1', action), entry('c2', 'pick')];
      expect(computeIssues(entries)).toEqual([{ index: 0, rule: 'squashAsFirst' }]);
    }
  });

  it('drop + squash 开头也算"前无存活"（比后端更早说人话）', () => {
    const entries = [entry('c1', 'drop'), entry('c2', 'squash')];
    expect(computeIssues(entries)).toEqual([{ index: 1, rule: 'squashAsFirst' }]);
  });

  it('全部 drop 被拒（等于丢整个区间）', () => {
    const entries = [entry('c1', 'drop'), entry('c2', 'drop')];
    const issues = computeIssues(entries);
    expect(issues.some((issue) => issue.rule === 'allDropped')).toBe(true);
    expect(isExecutable(entries)).toBe(false);
  });

  it('merge 提交不能 squash；开启 flatten 后放行', () => {
    const entries = [entry('c1', 'pick'), entry('m', 'squash', { parents: ['c1', 'side'] })];
    expect(computeIssues(entries)).toEqual([{ index: 1, rule: 'squashOnMerge' }]);
    expect(computeIssues(entries, { allowFlattenMerges: true })).toEqual([]);
  });
});

describe('mergeTargetFor', () => {
  it('跳过 drop，向前找最近的存活条目', () => {
    const entries = [entry('c1', 'pick'), entry('c2', 'drop'), entry('c3', 'squash')];
    expect(mergeTargetFor(entries, 2)?.oid).toBe('c1');
  });

  it('前面没有存活条目返回 null', () => {
    const entries = [entry('c1', 'drop'), entry('c2', 'squash')];
    expect(mergeTargetFor(entries, 1)).toBeNull();
  });
});

describe('toRequestEntries', () => {
  it('只有 reword / squash 携带信息草案', () => {
    const entries = [
      entry('c1', 'pick', { newMessage: 'ignored' }),
      entry('c2', 'reword', { newMessage: 'kept' }),
      entry('c3', 'squash', { newMessage: 'merged' }),
    ];
    expect(toRequestEntries(entries)).toEqual([
      { oid: 'c1', action: 'pick' },
      { oid: 'c2', action: 'reword', newMessage: 'kept' },
      { oid: 'c3', action: 'squash', newMessage: 'merged' },
    ]);
  });
});

describe('buildPreviewRows', () => {
  it('squash / fixup 归组到前一条存活行，丢弃行排在最后', () => {
    const entries = [
      entry('c1', 'pick'),
      entry('c2', 'squash', { newMessage: 'one\n\ntwo' }),
      entry('c3', 'drop'),
      entry('c4', 'reword', { newMessage: 'four rw' }),
    ];
    const preview = previewOf({
      surviving: [
        { oid: 'c1', subject: 'one\n\ntwo' },
        { oid: 'c4', subject: 'four rw' },
      ],
      dropped: ['c3'],
      reworded: ['c4'],
      squashed: ['c2 -> c1'],
      affectedCount: 4,
    });

    const rows = buildPreviewRows(entries, preview);
    expect(rows.map((row) => [row.kind, row.oid])).toEqual([
      ['survivor', 'c1'],
      ['survivor', 'c4'],
      ['dropped', 'c3'],
    ]);
    expect(rows[0]?.mergedOids).toEqual(['c2']);
    expect(rows[1]?.reworded).toBe(true);
    expect(rows[2]?.subject).toBe('subject-c3');
  });

  it('fixup 归组但不影响主体信息', () => {
    const entries = [entry('c1', 'pick'), entry('c2', 'fixup')];
    const preview = previewOf({
      surviving: [{ oid: 'c1', subject: 'one' }],
      squashed: ['c2 -> (fixup)'],
      affectedCount: 2,
    });
    const rows = buildPreviewRows(entries, preview);
    expect(rows[0]?.mergedOids).toEqual(['c2']);
    expect(rows[0]?.subject).toBe('one');
  });
});

describe('rangeFromSelection', () => {
  const ROWS = [
    { oid: 'c4', parents: ['c3'] },
    { oid: 'c3', parents: ['c2'] },
    { oid: 'c2', parents: ['c1'] },
    { oid: 'c1', parents: ['base'] },
    { oid: 'base', parents: [] },
  ];

  it('连续选中段：head 取最新、base 取最旧选中提交的父', () => {
    expect(rangeFromSelection(ROWS, ['c2', 'c3'])).toEqual({ base: 'c1', head: 'c3' });
  });

  it('不连续选中：取跨越区间（head 最新、base 最旧选中的父）', () => {
    expect(rangeFromSelection(ROWS, ['c1', 'c3'])).toEqual({ base: 'base', head: 'c3' });
  });

  it('空选中或根提交（无父）返回 null', () => {
    expect(rangeFromSelection(ROWS, [])).toBeNull();
    expect(rangeFromSelection(ROWS, ['base'])).toBeNull();
  });

  it('选中集合里的 oid 不在行序中时忽略（历史页换过滤条件后的陈旧选中）', () => {
    expect(rangeFromSelection(ROWS, ['c2', 'ghost'])).toEqual({ base: 'c1', head: 'c2' });
  });
});
