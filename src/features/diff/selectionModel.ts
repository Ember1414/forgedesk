/**
 * hunk / 行选择模型（T1.6）——纯函数。
 *
 * 为什么要单独一层：选择是"到底要暂存什么"的唯一真相，而它的边界最容易出错
 * （跨块的范围选择、全选与半选、折叠后的可见性、把上下文行也算进来）。
 * 写成纯函数后，固定样本单测就能覆盖这些边界，而不是靠点界面去试。
 *
 * # 下标口径
 *
 * `lineIndex` 是"行在该 hunk `lines` 数组里的位置"，与 `workspace_diff` 返回的
 * 结构一一对应（上下文行与 `\ No newline` 标记也参与计数）。后端裁剪器用的就是
 * 这个口径，因此界面不需要（也不能）自己算补丁。
 */
import type { DiffHunk, DiffLineKind, LineSelection } from '@/lib/ipc/workspace';

/** 一个可被选中的行：hunk 下标 + 行在 hunk 内的位置。 */
export interface SelectableLine {
  readonly hunkIndex: number;
  readonly lineIndex: number;
}

/** 选择状态：hunk 下标 → 该 hunk 内被选中的行位置。 */
export type Selection = ReadonlyMap<number, ReadonlySet<number>>;

/** 空选择。 */
export function emptySelection(): Selection {
  return new Map<number, ReadonlySet<number>>();
}

/**
 * 这一行是否具备"可暂存"的语义。
 *
 * 上下文行在补丁两侧都存在，选中它没有语义（后端会忽略）；
 * `\ No newline` 标记附着于它的上一行，由后端跟着上一行的去留一起处理。
 */
export function isSelectable(kind: DiffLineKind): boolean {
  return kind === 'added' || kind === 'removed';
}

/** 展平出文件里所有可选中的行（按补丁顺序，供范围选择使用）。 */
export function selectableLines(hunks: readonly DiffHunk[]): SelectableLine[] {
  const lines: SelectableLine[] = [];
  hunks.forEach((hunk, hunkIndex) => {
    hunk.lines.forEach((line, lineIndex) => {
      if (isSelectable(line.kind)) {
        lines.push({ hunkIndex, lineIndex });
      }
    });
  });
  return lines;
}

/** 选中 / 取消选中一行（返回新状态，不修改入参）。 */
export function toggleLine(selection: Selection, line: SelectableLine): Selection {
  const next = new Map(selection);
  const current = new Set(next.get(line.hunkIndex) ?? []);
  if (current.has(line.lineIndex)) {
    current.delete(line.lineIndex);
  } else {
    current.add(line.lineIndex);
  }
  if (current.size === 0) {
    next.delete(line.hunkIndex);
  } else {
    next.set(line.hunkIndex, current);
  }
  return next;
}

/** 范围选择（含两端，按展平后的顺序）：与 Shift 点击配合使用。 */
export function selectRange(
  hunks: readonly DiffHunk[],
  selection: Selection,
  from: SelectableLine,
  to: SelectableLine,
): Selection {
  const all = selectableLines(hunks);
  const start = all.findIndex((line) => sameLine(line, from));
  const end = all.findIndex((line) => sameLine(line, to));
  if (start < 0 || end < 0) {
    return selection;
  }

  const low = Math.min(start, end);
  const high = Math.max(start, end);
  const next = new Map(selection);
  for (let index = low; index <= high; index += 1) {
    const line = all[index];
    if (line === undefined) {
      continue;
    }
    const current = new Set(next.get(line.hunkIndex) ?? []);
    current.add(line.lineIndex);
    next.set(line.hunkIndex, current);
  }
  return next;
}

/** 该块是否所有变更行都被选中。 */
export function isHunkFullySelected(
  selection: Selection,
  hunks: readonly DiffHunk[],
  hunkIndex: number,
): boolean {
  const hunk = hunks[hunkIndex];
  const selected = selection.get(hunkIndex);
  if (hunk === undefined || selected === undefined) {
    return false;
  }
  return hunk.lines.every((line, index) => !isSelectable(line.kind) || selected.has(index));
}

/** 整块选中 / 取消选中：已全选则整块取消，否则整块选上。 */
export function toggleHunk(
  selection: Selection,
  hunks: readonly DiffHunk[],
  hunkIndex: number,
): Selection {
  const hunk = hunks[hunkIndex];
  if (hunk === undefined) {
    return selection;
  }
  const next = new Map(selection);
  if (isHunkFullySelected(selection, hunks, hunkIndex)) {
    next.delete(hunkIndex);
    return next;
  }

  const positions = new Set<number>();
  hunk.lines.forEach((line, index) => {
    if (isSelectable(line.kind)) {
      positions.add(index);
    }
  });
  if (positions.size === 0) {
    return selection;
  }
  next.set(hunkIndex, positions);
  return next;
}

/** 选中的行数。 */
export function countSelected(selection: Selection): number {
  let total = 0;
  for (const lines of selection.values()) {
    total += lines.size;
  }
  return total;
}

/** 是否没有任何选中行。 */
export function isEmptySelection(selection: Selection): boolean {
  return countSelected(selection) === 0;
}

/** 有选中行的 hunk 下标（升序）。 */
export function selectedHunkIndices(selection: Selection): number[] {
  return [...selection.entries()]
    .filter(([, lines]) => lines.size > 0)
    .map(([hunkIndex]) => hunkIndex)
    .sort((left, right) => left - right);
}

/**
 * 转成 IPC 形状。
 *
 * hunk 与行的下标都排序后再输出：稳定顺序让 IPC 载荷可比较（测试断言与日志排查
 * 都因此便宜），也避免"同样的选择产生两种载荷"。
 */
export function toLineSelections(selection: Selection): LineSelection[] {
  return [...selection.entries()]
    .filter(([, lines]) => lines.size > 0)
    .map(([hunkIndex, lines]) => ({
      hunkIndex,
      lines: [...lines].sort((left, right) => left - right),
    }))
    .sort((left, right) => left.hunkIndex - right.hunkIndex);
}

function sameLine(left: SelectableLine, right: SelectableLine): boolean {
  return left.hunkIndex === right.hunkIndex && left.lineIndex === right.lineIndex;
}
