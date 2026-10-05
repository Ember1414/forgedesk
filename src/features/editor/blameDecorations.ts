/**
 * Blame 装饰器（T5.8）：把逐行归属渲染成 Monaco 左侧色条 + hover 摘要。
 *
 * # 渲染策略（与 Monaco 对齐，不另画 DOM）
 *
 * - 每行一个 decoration，`linesDecorationsClassName` 画 3px 色条——
 *   颜色按提交 oid 哈希映射到既有的 8 个 graph-lane token（不引入新色板）；
 * - `hoverMessage`（Markdown）显示作者 · 时间 · 标题，悬停即读；
 * - 未提交行（全零 SHA）用 warning 色 + 专属标记；
 * - 点击行（onMouseDown 时查 blame 映射）→ 回调打开提交详情。
 *
 * # 与编辑器的协同
 *
 * 行装饰跟随滚动与折叠（stickiness），只在 wordWrap 关闭时与行号严格对齐，
 * 因此启用 blame 时编辑器强制 `wordWrap: 'off'`。
 */
import type { editor } from 'monaco-editor/esm/vs/editor/editor.api';
import type { BlameLine } from '@/lib/ipc/blame';

/** 色条 class 前缀（CSS 定义在 src/features/editor/blame.css，色值取 graph-lane token）。 */
export const BLAME_CLASS_PREFIX = 'fd-blame-lane-';
export const BLAME_LANE_COUNT = 8;

/** 未提交行的 class。 */
export const BLAME_UNCOMMITTED_CLASS = 'fd-blame-uncommitted';

/** 按 oid 稳定分配 lane（同提交同色；8 色循环）。 */
export function laneIndexFor(oid: string): number {
  let hash = 0;
  for (let index = 0; index < 8; index += 1) {
    hash = (hash * 31 + (oid.charCodeAt(index) ?? 0)) >>> 0;
  }
  return hash % BLAME_LANE_COUNT;
}

/** hover 摘要（Monaco hoverMessage 是 Markdown 字符串）。 */
export function hoverMessageFor(line: BlameLine): string {
  const time = new Date(line.authorTime * 1000).toLocaleDateString();
  // 未提交标记：hover 是 Markdown 协议层的原始片段（无 t() 上下文），
  // 与 TRUNCATION_MARK 同类；正式文案在 hover 渲染端（T6.7 统一 i18n 时再收编）
  const badge = line.isUncommitted ? '_uncommitted_' : `**${line.shortOid}**`; // i18n-ignore
  return `${badge} · **${line.author}** · ${time}\n\n${line.summary}`;
}

/**
 * 应用 blame 装饰；返回 [`editor.IEditorDecorationsCollection`]（clear 由调用方调用）。
 */
export function applyBlameDecorations(
  editor: editor.IStandaloneCodeEditor,
  blame: readonly BlameLine[],
): editor.IEditorDecorationsCollection {
  const decorations: editor.IModelDeltaDecoration[] = blame.map((line) => ({
    range: {
      startLineNumber: line.lineNo,
      startColumn: 1,
      endLineNumber: line.lineNo,
      endColumn: 1,
    },
    options: {
      isWholeLine: true,
      linesDecorationsClassName: line.isUncommitted
        ? BLAME_UNCOMMITTED_CLASS
        : `${BLAME_CLASS_PREFIX}${laneIndexFor(line.oid)}`,
      hoverMessage: { value: hoverMessageFor(line) },
      stickiness: 1,
    },
  }));
  return editor.createDecorationsCollection(decorations);
}

/** 查询某行上的 blame 记录（点击跳转用）。 */
export function blameAtLine(blame: readonly BlameLine[], lineNumber: number): BlameLine | null {
  return blame.find((line) => line.lineNo === lineNumber) ?? null;
}
