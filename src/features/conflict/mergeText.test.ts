//! mergeText 纯函数的测试：结果文本组装与残留标记检查是**保存正确性**，
//! 这里在不挂载组件的情况下穷举行为。
import { describe, expect, it } from 'vitest';

import {
  assembleResult,
  conflictMarkerLines,
  findConflictMarkers,
  INITIAL_BLOCK_STATE,
  wordDiffLine,
} from '@/features/conflict/mergeText';
import type { MergeBlock } from '@/lib/ipc';

const conflict: MergeBlock = {
  type: 'conflict',
  base: ['base'],
  ours: ['ours line'],
  theirs: ['theirs line'],
};

const context: MergeBlock = { type: 'context', lines: ['ctx'] };

describe('assembleResult', () => {
  it('未解决块保留 git 冲突标记原文', () => {
    const text = assembleResult([conflict], [INITIAL_BLOCK_STATE]);
    expect(text.split('\n')).toEqual([
      '<<<<<<< ours',
      'ours line',
      '=======',
      'theirs line',
      '>>>>>>> theirs',
    ]);
  });

  it('采用本方 / 对方 / 两种保留顺序各自正确', () => {
    expect(assembleResult([conflict], [{ resolution: 'ours' }])).toBe('ours line');
    expect(assembleResult([conflict], [{ resolution: 'theirs' }])).toBe('theirs line');
    expect(assembleResult([conflict], [{ resolution: 'bothOursFirst' }])).toBe(
      'ours line\ntheirs line',
    );
    expect(assembleResult([conflict], [{ resolution: 'bothTheirsFirst' }])).toBe(
      'theirs line\nours line',
    );
  });

  it('自定义文本原样使用', () => {
    expect(assembleResult([conflict], [{ resolution: 'custom', customText: 'my\nway' }])).toBe(
      'my\nway',
    );
  });

  it('空段贡献零行（删除一侧后不留下凭空空行）', () => {
    // ours 删除了内容：采纳本方后该块为空，前后上下文直接相邻
    const emptyOurs: MergeBlock = {
      type: 'conflict',
      base: ['gone'],
      ours: [],
      theirs: ['kept'],
    };
    const text = assembleResult(
      [context, emptyOurs, { type: 'context', lines: ['after'] }],
      [undefined, { resolution: 'ours' }, undefined],
    );
    expect(text.split('\n')).toEqual(['ctx', 'after']);
  });

  it('上下文与已解决段直接拼接', () => {
    const text = assembleResult(
      [context, { type: 'resolved', lines: ['auto'], source: 'ours' }, context],
      [undefined, undefined, undefined],
    );
    expect(text.split('\n')).toEqual(['ctx', 'auto', 'ctx']);
  });
});

describe('findConflictMarkers', () => {
  it('检出行首的 git 冲突标记', () => {
    const markers = findConflictMarkers('ok\n<<<<<<< ours\nbody\n=======\nbody2\n>>>>>>> theirs\n');
    expect(markers).toHaveLength(3);
  });

  it('忽略正文里恰好包含等号但不是标记的行', () => {
    expect(findConflictMarkers('====== 文档标题 ======\ncontent')).toHaveLength(0);
    // 整行等号会被检出（保守：警告不阻断，误报成本远低于漏报）
    expect(findConflictMarkers('=======')).toHaveLength(1);
  });
});

describe('conflictMarkerLines', () => {
  it('给出与 git 一致的五段结构', () => {
    expect(conflictMarkerLines(conflict)).toEqual([
      '<<<<<<< ours',
      'ours line',
      '=======',
      'theirs line',
      '>>>>>>> theirs',
    ]);
  });
});

describe('wordDiffLine', () => {
  it('公共词不高亮、落单词高亮', () => {
    const diff = wordDiffLine('const value = 1;', 'const result = 1;');
    expect(diff).not.toBeNull();
    if (diff === null) return;
    const changed = (spans: readonly { text: string; changed: boolean }[]) =>
      spans.filter((span) => span.changed).map((span) => span.text.trim());
    // 拼回原文（顺序保持）
    const joined = (spans: readonly { text: string; changed: boolean }[]) =>
      spans.map((span) => span.text).join('');
    expect(joined(diff.ours)).toBe('const value = 1;');
    expect(joined(diff.theirs)).toBe('const result = 1;');
    expect(changed(diff.ours)).toEqual(['value']);
    expect(changed(diff.theirs)).toEqual(['result']);
  });

  it('两行完全相同则没有高亮', () => {
    const diff = wordDiffLine('same line', 'same line');
    expect(diff).not.toBeNull();
    if (diff === null) return;
    expect(diff.ours.every((span) => !span.changed)).toBe(true);
    expect(diff.theirs.every((span) => !span.changed)).toBe(true);
  });

  it('超过 40 词的行返回 null（性能闸）', () => {
    const long = Array.from({ length: 41 }, (_, i) => `w${i}`).join(' ');
    expect(wordDiffLine(long, 'x')).toBeNull();
  });
});
