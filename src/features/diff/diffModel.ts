/**
 * diff 行模型（T1.5）——把后端的 hunk 列表变成两种视图的行序列。
 *
 * 为什么是纯函数：配对（并排）与展平（内联）的逻辑是"diff 查看器正确性"的核心，
 * 用固定样本单测比挂载组件便宜且能覆盖边界（不对称增删、空 hunk）。
 */
import type { DiffHunk, DiffLine } from '@/lib/ipc/workspace';

/** 内联视图的一行。 */
export interface UnifiedRow {
  readonly kind: 'line';
  readonly line: DiffLine;
  /** 所属 hunk 在文件里的序号（折叠/跳转用）。 */
  readonly hunkIndex: number;
}

/** 并排视图的一行：左右各是新旧一侧，缺失侧为 null。 */
export interface PairRow {
  readonly kind: 'pair';
  readonly left: DiffLine | null;
  readonly right: DiffLine | null;
  readonly hunkIndex: number;
}

/** 两种视图共用的行（判别联合而不是 any）。 */
export type DiffRow = UnifiedRow | PairRow;

/** 字符级高亮的开关阈值：变更行数超过它就只染色整行。 */
export const CHAR_DIFF_MAX_CHANGED_LINES = 2000;

/** 文件的全部 hunk 里新增+删除的行数（决定字符级高亮是否负担得起）。 */
export function changedLineCount(hunks: readonly DiffHunk[]): number {
  let count = 0;
  for (const hunk of hunks) {
    for (const line of hunk.lines) {
      if (line.kind === 'added' || line.kind === 'removed') {
        count += 1;
      }
    }
  }
  return count;
}

/** 内联视图：hunk 顺序展开为行序列。 */
export function flattenUnified(hunks: readonly DiffHunk[]): UnifiedRow[] {
  const rows: UnifiedRow[] = [];
  hunks.forEach((hunk, hunkIndex) => {
    for (const line of hunk.lines) {
      rows.push({ kind: 'line', line, hunkIndex });
    }
  });
  return rows;
}

/**
 * 并排视图：把 hunk 展开成 (左, 右) 行对。
 *
 * 配对规则与主流 diff 工具一致：上下文行左右成对；连续的删除/添加运行
 * 按顺序 1:1 拉齐，短的一侧补 null（独占一整行，渲染为空白占位）。
 * 行高恒定是虚拟化的前提。`\ No newline` 标记行独占左侧（跟随其内容行）。
 */
export function pairSideBySide(hunks: readonly DiffHunk[]): PairRow[] {
  const rows: PairRow[] = [];
  hunks.forEach((hunk, hunkIndex) => {
    let index = 0;
    while (index < hunk.lines.length) {
      const line = hunk.lines[index];
      if (line === undefined) {
        break; // 不可达（index < length），仅为收窄类型
      }
      if (line.kind === 'context') {
        rows.push({ kind: 'pair', left: line, right: line, hunkIndex });
        index += 1;
        continue;
      }
      if (line.kind === 'noNewline') {
        rows.push({ kind: 'pair', left: line, right: null, hunkIndex });
        index += 1;
        continue;
      }
      // 收集同一侧的连续运行（removed 或 added）
      const side = line.kind;
      const run: DiffLine[] = [];
      while (index < hunk.lines.length) {
        const next = hunk.lines[index];
        if (next === undefined || next.kind !== side) {
          break;
        }
        run.push(next);
        index += 1;
      }
      // 对面的运行
      const other = side === 'removed' ? 'added' : 'removed';
      const counterpart: DiffLine[] = [];
      while (index < hunk.lines.length) {
        const next = hunk.lines[index];
        if (next === undefined || next.kind !== other) {
          break;
        }
        counterpart.push(next);
        index += 1;
      }
      const leftIsOld = side === 'removed';
      const length = Math.max(run.length, counterpart.length);
      for (let pairIndex = 0; pairIndex < length; pairIndex += 1) {
        const first = run[pairIndex] ?? null;
        const second = counterpart[pairIndex] ?? null;
        rows.push({
          kind: 'pair',
          left: leftIsOld ? first : second,
          right: leftIsOld ? second : first,
          hunkIndex,
        });
      }
    }
  });
  return rows;
}

/**
 * 找出"修改对"：并排视图里左删右加的行对序号 → 两侧内容。
 *
 * 字符级高亮只对真正的"修改"做（左删右加），纯增/纯删没有对照可高亮。
 */
export function modifiedPairs(rows: readonly PairRow[]): Map<number, readonly [string, string]> {
  const pairs = new Map<number, readonly [string, string]>();
  for (let index = 0; index < rows.length; index += 1) {
    const row = rows[index];
    if (row === undefined) {
      continue;
    }
    if (row.left !== null && row.right !== null) {
      if (row.left.kind === 'removed' && row.right.kind === 'added') {
        pairs.set(index, [row.left.content, row.right.content]);
      }
    }
  }
  return pairs;
}

/** 内联视图里的"修改对"：removed 后紧跟 added 的相邻行对序号 → 两侧内容。 */
export function modifiedPairsUnified(
  rows: readonly UnifiedRow[],
): Map<number, readonly [string, string]> {
  const pairs = new Map<number, readonly [string, string]>();
  for (let index = 0; index + 1 < rows.length; index += 1) {
    const left = rows[index];
    const right = rows[index + 1];
    if (left === undefined || right === undefined) {
      continue;
    }
    if (left.line.kind === 'removed' && right.line.kind === 'added') {
      pairs.set(index, [left.line.content, right.line.content]);
      index += 1; // 一对只配一次
    }
  }
  return pairs;
}

/** hunk 折叠占位里显示的上下文行数。 */
export function contextLineCount(hunk: DiffHunk): number {
  return hunk.lines.filter((line) => line.kind === 'context').length;
}
