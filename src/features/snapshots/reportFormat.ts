/**
 * 回滚报告的格式化辅助（快照页与操作历史页共用）。
 *
 * 为什么单独一个模块：T3.9 把这些逻辑写在快照页里，T3.10 的操作历史页要用同一套
 * 呈现（同一个回滚报告，两处展示）——复制一份必然会在某次改动后分叉，
 * 而"两处对同一次回滚给出不同说法"是最不该出现的 bug。
 */
import type { RestoreOutcomeKind } from '@/lib/ipc/snapshots';

/** 回滚结局对应的文字配色（**始终配图标**，不靠颜色单独表意）。 */
export function outcomeClass(outcome: RestoreOutcomeKind): string {
  switch (outcome) {
    case 'completed':
      return 'text-success';
    case 'rolledBack':
      return 'text-warning';
    default:
      return 'text-danger';
  }
}

/**
 * 前几条路径 + "等 N 项"。
 *
 * `more` 由调用方给（它知道往哪份 i18n 文案里填），这样这个模块不必引入 i18n，
 * 也不必知道语言。
 */
export function summarizePaths(
  paths: readonly string[],
  more: (count: number) => string,
  limit = 5,
): string {
  const head = paths.slice(0, limit).join(', ');
  return paths.length > limit ? head + more(paths.length - limit) : head;
}
