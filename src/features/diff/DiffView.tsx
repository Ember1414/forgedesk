/**
 * Diff 查看器（T1.5）+ 行级 / 块级选择（T1.6）。
 *
 * # 关键取舍
 *
 * - **语法高亮暂缓到 M5**（编辑器里程碑）：Monaco diff 视图约 5MB、需要 Worker
 *   配置与按语言加载；shiki 需要异步词法器基础设施。两者都是"编辑功能"才摊得平
 *   的成本，只读查看器独自背负它们违反零成本约束。当前用"行级配色 + 修改行
 *   字符级高亮"覆盖评审场景，语言着色随 M5 的 Monaco 一起落地。
 * - **行高恒定 + 内容不换行**：定高是 VirtualList 虚拟化的前提；长行横向滚动。
 * - **折叠是纯界面状态**（上下文已在响应里），"更多上下文"才重新请求（×4 递增）。
 * - 模式偏好持久化在后端设置（`ui.diffViewMode`），与界面密度同一套机制。
 * - **查看器不做写操作**：选择由本组件收集，暂存 / 取消暂存 / 丢弃通过回调交给
 *   调用方（页面）去接线 IPC 与查询失效。这样同一个查看器仍能用在只读场景
 *   （例如未来的提交详情），而"谁能改仓库"这件事只在页面层决定一次。
 * - **`view` 参数必须与查询一致**：hunk 的划分取决于上下文行数，后端要用同一组
 *   参数重新生成补丁并对下标做越界校验（见 `services::staging` 的模块头）。
 *
 * # 可选择的边界
 *
 * - 只有新增 / 删除行可选（上下文行两侧都有，选中它没有语义）；
 * - 二进制文件与**被截断的大 diff** 不提供行级选择：前者的补丁不是文本，
 *   后者的 hunk 下标与后端的完整补丁对不上 —— 与其让用户点了再失败，
 *   不如把入口关掉并说明原因。
 */
import { useMemo, useRef, useState } from 'react';

import { useQuery } from '@tanstack/react-query';
import { ChevronDown, ChevronUp, Copy, Scissors, UnfoldVertical } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import {
  changedLineCount,
  CHAR_DIFF_MAX_CHANGED_LINES,
  modifiedPairs,
  modifiedPairsUnified,
  pairSideBySide,
} from '@/features/diff/diffModel';
import type { PairRow, UnifiedRow } from '@/features/diff/diffModel';
import {
  countSelected,
  emptySelection,
  isEmptySelection,
  isSelectable,
  selectRange,
  toLineSelections,
  toggleLine,
} from '@/features/diff/selectionModel';
import type { SelectableLine, Selection } from '@/features/diff/selectionModel';
import { useSettingsStore } from '@/stores/settingsStore';
import { normalizeError } from '@/lib/errors';
import { PERFORMANCE_CONTEXT_LINES, usePerformanceMode } from '@/lib/performanceMode';
import { workspaceDiff, workspaceDiffPatch } from '@/lib/ipc/workspace';
import type { DiffHunk, DiffLine, PatchViewSpec, StageScope } from '@/lib/ipc/workspace';
import { Button } from '@/ui/components/button';
import { DIFF_QUERY_KEY } from '@/lib/queryKeys';
import { ErrorState } from '@/ui/components/error-state';
import { IconButton } from '@/ui/components/icon-button';
import { Skeleton } from '@/ui/components/skeleton';
import { ToggleGroup } from '@/ui/components/toggle-group';
import { VirtualList } from '@/ui/components/virtual-list';
import { cn } from '@/lib/utils';
import { diffWordsWithSpace } from 'diff';

/** 行高（px）：虚拟化的数学依赖它，与行样式 leading-6 保持一致。 */
const ROW_HEIGHT = 24;

/** 单元内容的统一样式：等宽、不换行（横向滚动）。 */
const CONTENT_CLASS = 'font-mono text-12 leading-6 whitespace-pre';

const DIFF_VIEW_MODE_KEY = 'ui.diffViewMode';

export type DiffViewMode = 'unified' | 'side-by-side';

export interface DiffViewProps {
  readonly repoId: number;
  readonly path: string;
  /** 工作区比较侧；`source`（提交间比较）给出时可以省略。 */
  readonly target?: 'staged' | 'unstaged';
  readonly className?: string;
  /**
   * 提交间比较（T2.4）：给出 `from`/`to` 时按 `target: 'between'` 查询，
   * 覆盖 `target` 的语义。此模式下没有暂存/丢弃动作（那是对工作区的操作），
   * 行级选择按钮自然消失；行内"复制 hunk 补丁"仍然可用。
   */
  readonly source?: {
    readonly from: string;
    readonly to: string;
  };
  /**
   * 暂存选中的行 / 块（未暂存侧提供）。
   *
   * `view` 是生成补丁时用的查看参数，**必须原样转发给后端**：hunk 的划分取决于
   * 上下文行数，后端要用同一组参数重新生成补丁（见 `services::staging` 的模块头）。
   */
  readonly onStage?: (scope: StageScope, view: PatchViewSpec) => void;
  /** 取消暂存选中的行 / 块（已暂存侧提供）。 */
  readonly onUnstage?: (scope: StageScope, view: PatchViewSpec) => void;
  /** 丢弃选中的行 / 块（调用方必须先弹确认对话框）。 */
  readonly onDiscard?: (scope: StageScope, view: PatchViewSpec) => void;
  /** 有写操作正在进行：按钮进入 loading 且快捷键失效。 */
  readonly busy?: boolean;
}

/** 显示序列的一行：hunk 头 / 内联行 / 并排行。 */
type DisplayRow =
  | { readonly kind: 'header'; readonly hunkIndex: number; readonly hunk: DiffHunk }
  | { readonly kind: 'u-line'; readonly row: UnifiedRow; readonly lineIndex: number }
  | { readonly kind: 'pair'; readonly row: PairRow };

/** 重建一个 hunk 的补丁文本（复制用；与 git 输出逐字节可对齐）。 */
function hunkToPatch(hunk: DiffHunk): string {
  const header =
    '@@ -' +
    hunk.oldStart +
    ',' +
    hunk.oldLines +
    ' +' +
    hunk.newStart +
    ',' +
    hunk.newLines +
    ' @@' +
    (hunk.header === '' ? '' : ' ' + hunk.header);
  const prefix = (line: DiffLine): string => {
    if (line.kind === 'added') return '+' + line.content;
    if (line.kind === 'removed') return '-' + line.content;
    if (line.kind === 'noNewline') return '\\ No newline at end of file';
    return ' ' + line.content;
  };
  return header + '\n' + hunk.lines.map(prefix).join('\n');
}

/**
 * 字符级差异文本：对"修改对"的两侧分别渲染词级差异。
 * jsdiff 的 diffWordsWithSpace 按空白分词，对代码行的粒度正合适；
 * 视图侧（added/removed）只显示属于自己的部分并高亮真正变化的词。
 */
function CharDiffText({
  before,
  after,
  side,
}: {
  readonly before: string;
  readonly after: string;
  readonly side: 'before' | 'after';
}) {
  const parts = useMemo(() => diffWordsWithSpace(before, after), [before, after]);
  return (
    <span className={CONTENT_CLASS}>
      {parts.map((part, index) => {
        const show = side === 'before' ? !part.added : !part.removed;
        if (!show || part.value === '') {
          return null;
        }
        const highlighted =
          (side === 'before' && part.removed === true) || (side === 'after' && part.added === true);
        return (
          <span
            key={index}
            className={cn(highlighted && (side === 'before' ? 'bg-danger/30' : 'bg-success/30'))}
          >
            {part.value}
          </span>
        );
      })}
    </span>
  );
}

/** 行号列（并排视图半边一个；内联视图两个）。可选中的行号同时是选择按钮。 */
function LineNo({
  value,
  canPick = false,
  selected = false,
  onPick,
}: {
  readonly value: number | null | undefined;
  readonly canPick?: boolean;
  readonly selected?: boolean;
  readonly onPick?: (additive: boolean) => void;
}) {
  const { t } = useTranslation('shell');
  const label = value ?? '';

  if (!canPick) {
    return (
      <span className="w-12 shrink-0 select-none pr-1.5 text-right text-11 text-fg-subtle tabular-nums">
        {label}
      </span>
    );
  }

  return (
    <button
      type="button"
      aria-pressed={selected}
      aria-label={t('diff.selection.pickLineWithNumber', { line: label })}
      onClick={(event) => onPick?.(event.shiftKey)}
      className={cn(
        'fd-transition w-12 shrink-0 select-none pr-1.5 text-right text-11 tabular-nums',
        selected ? 'bg-brand-subtle font-semibold text-brand' : 'text-fg-subtle hover:bg-subtle/60',
      )}
    >
      {label}
    </button>
  );
}

/** 单元格：符号 + 内容（并排视图的一半；空侧渲染为灰底占位）。 */
function Cell({
  line,
  charPair,
  side,
  canPick,
  selected,
  onPick,
}: {
  readonly line: DiffLine | null;
  readonly charPair: readonly [string, string] | undefined;
  readonly side: 'old' | 'new';
  readonly canPick: boolean;
  readonly selected: boolean;
  readonly onPick: (line: DiffLine, additive: boolean) => void;
}) {
  if (line === null) {
    return <div className="flex flex-1 bg-subtle/30" aria-hidden="true" />;
  }
  const tone =
    line.kind === 'added' ? 'bg-success/12' : line.kind === 'removed' ? 'bg-danger/12' : '';
  const sign = line.kind === 'added' ? '+' : line.kind === 'removed' ? '-' : '';
  const isModifiedHalf =
    charPair !== undefined && (line.kind === 'added' || line.kind === 'removed');
  return (
    <div className={cn('flex min-w-0 flex-1 items-center', tone)}>
      <LineNo
        value={side === 'old' ? line.oldNo : line.newNo}
        canPick={
          canPick &&
          isSelectable(line.kind) &&
          (side === 'old' ? line.oldNo != null : line.newNo != null)
        }
        selected={selected}
        onPick={(additive) => onPick(line, additive)}
      />
      <span className="w-4 shrink-0 select-none text-center text-11 text-fg-subtle">{sign}</span>
      <span className="min-w-0 flex-1 overflow-hidden pr-2">
        {isModifiedHalf && charPair !== undefined ? (
          <CharDiffText
            before={charPair[0]}
            after={charPair[1]}
            side={line.kind === 'removed' ? 'before' : 'after'}
          />
        ) : (
          <span className={CONTENT_CLASS}>{line.content}</span>
        )}
      </span>
    </div>
  );
}

/** hunk 头行：范围 + 函数上下文 + 折叠开关 + 复制 hunk + （可选）整块操作。 */
function HunkHeader({
  hunk,
  hunkIndex,
  isCollapsed,
  onToggle,
  actionLabel,
  onAction,
}: {
  readonly hunk: DiffHunk;
  readonly hunkIndex: number;
  readonly isCollapsed: boolean;
  readonly onToggle: (hunkIndex: number) => void;
  readonly actionLabel?: string;
  readonly onAction?: (hunkIndex: number) => void;
}) {
  const { t } = useTranslation('shell');
  return (
    <div className="flex h-6 items-center gap-2 bg-subtle/50 px-2 font-mono text-11 text-fg-subtle">
      <button
        type="button"
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
        aria-expanded={!isCollapsed}
        onClick={() => onToggle(hunkIndex)}
      >
        <span
          className={cn(
            'inline-block w-3 shrink-0 text-center transition-transform',
            isCollapsed ? '' : 'rotate-90',
          )}
          aria-hidden="true"
        >
          ▶
        </span>
        <span className="whitespace-pre">
          {'@@ -' +
            hunk.oldStart +
            ',' +
            hunk.oldLines +
            ' +' +
            hunk.newStart +
            ',' +
            hunk.newLines +
            ' @@'}
        </span>
        {hunk.header !== '' && <span className="truncate text-fg-subtle/80">{hunk.header}</span>}
      </button>
      {actionLabel !== undefined && onAction !== undefined ? (
        <button
          type="button"
          className="fd-transition shrink-0 rounded-sm px-1.5 py-0.5 text-11 text-brand hover:bg-brand-subtle"
          onClick={() => onAction(hunkIndex)}
        >
          {actionLabel}
        </button>
      ) : null}
      <IconButton
        variant="ghost"
        size="sm"
        label={t('diff.copyHunk')}
        tooltip={t('diff.copyHunk')}
        className="h-4 w-4"
        onClick={() => {
          void navigator.clipboard.writeText(hunkToPatch(hunk));
        }}
      >
        <Scissors className="size-3" />
      </IconButton>
    </div>
  );
}

export function DiffView({
  repoId,
  path,
  target,
  source,
  className,
  onStage,
  onUnstage,
  onDiscard,
  busy = false,
}: DiffViewProps) {
  const { t } = useTranslation('shell');
  const [contextLines, setContextLines] = useState(3);
  const [forceFull, setForceFull] = useState(false);
  // 性能模式（T2.9）：大仓库里把上下文压到 1 行。用户点"更多上下文"是
  // 显式要细节——那次点击之后以用户为准，模式不再往回收。
  const [contextBoosted, setContextBoosted] = useState(false);
  const perfMode = usePerformanceMode(repoId);
  const effectiveContextLines =
    perfMode && !contextBoosted ? PERFORMANCE_CONTEXT_LINES : contextLines;
  const [collapsed, setCollapsed] = useState<ReadonlySet<number>>(new Set());
  const [scrollTarget, setScrollTarget] = useState(-1);
  const [jumpCursor, setJumpCursor] = useState(-1);
  const [selection, setSelection] = useState<Selection>(() => emptySelection());
  /** Shift 范围选择的锚点（上一次点击的行）。 */
  const anchor = useRef<SelectableLine | null>(null);

  // VirtualList 需要像素高度：用 ResizeObserver 测容器（Sheet 高度会随窗口变化）。
  // 必须用**回调 ref** 而不是挂载 effect：容器只在查询成功后渲染，
  // 挂载时它还不存在——effect 依赖 [] 只跑一次，观察者就永远不会被装上，
  // 面板会一直是空白（单测抓到的真实 bug）。
  const observerRef = useRef<ResizeObserver | null>(null);
  const [listHeight, setListHeight] = useState(0);
  const attachListContainer = (element: HTMLDivElement | null) => {
    observerRef.current?.disconnect();
    observerRef.current = null;
    if (element === null) {
      return;
    }
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry !== undefined) {
        setListHeight(Math.floor(entry.contentRect.height));
      }
    });
    observer.observe(element);
    observerRef.current = observer;
  };

  const query = useQuery({
    // 键的形状由 `@/lib/queryKeys` 统一提供：`repo:changed` 的失效逻辑按同一批键
    // 去找查询，键在这里写错（少一段、换个字面量）就会让自动刷新静默失效。
    // between 模式把 from/to 编进 target 位（同一文件、不同提交区间是不同的缓存条目）
    queryKey: [
      DIFF_QUERY_KEY,
      repoId,
      source === undefined ? target : `between:${source.from}:${source.to}`,
      path,
      effectiveContextLines,
      forceFull,
    ],
    queryFn: () =>
      source === undefined
        ? workspaceDiff(repoId, {
            target: target ?? 'unstaged',
            paths: [path],
            contextLines: effectiveContextLines,
            forceFull,
          })
        : workspaceDiff(repoId, {
            target: 'between',
            from: source.from,
            to: source.to,
            paths: [path],
            contextLines: effectiveContextLines,
            forceFull,
          }),
  });

  // DiffView 是重组件：必须订阅具体值而不是整个 store，否则任何设置写入
  // （与 diff 无关的也一样）都会让整个 diff 重渲染一遍
  const mode = useSettingsStore((state) =>
    state.getJson<DiffViewMode>(DIFF_VIEW_MODE_KEY, 'unified'),
  );

  const file = query.data?.files.find((candidate) => candidate.path === path);
  const hunks = useMemo(() => file?.hunks ?? [], [file]);
  // 性能模式下字符级高亮整档关闭：万级行的 diff 上它是最贵的渲染项
  const charDiffEnabled = !perfMode && changedLineCount(hunks) <= CHAR_DIFF_MAX_CHANGED_LINES;

  /** 行对象 → 它在所属 hunk 里的位置（选择与后端用同一口径，见 selectionModel）。 */
  const lineIndexOf = useMemo(() => {
    const map = new Map<DiffLine, number>();
    for (const hunk of hunks) {
      hunk.lines.forEach((line, index) => {
        map.set(line, index);
      });
    }
    return map;
  }, [hunks]);

  /** 该文件是否允许行级选择（二进制与截断的补丁做不到）。 */
  const canPick =
    file !== undefined &&
    !file.binary &&
    !file.truncated &&
    (onStage !== undefined || onUnstage !== undefined);

  const displayRows = useMemo<readonly DisplayRow[]>(() => {
    const rows: DisplayRow[] = [];
    hunks.forEach((hunk, hunkIndex) => {
      rows.push({ kind: 'header', hunkIndex, hunk });
      if (collapsed.has(hunkIndex)) {
        return;
      }
      if (mode === 'unified') {
        hunk.lines.forEach((line, lineIndex) => {
          rows.push({ kind: 'u-line', row: { kind: 'line', line, hunkIndex }, lineIndex });
        });
      } else {
        for (const row of pairSideBySide([hunk])) {
          rows.push({ kind: 'pair', row });
        }
      }
    });
    return rows;
  }, [hunks, collapsed, mode]);

  // 字符级"修改对"：按显示序列定位（折叠会移动行号，必须在显示序列上算）
  const pairs = useMemo(() => {
    if (!charDiffEnabled) {
      return new Map<number, readonly [string, string]>();
    }
    const bodyRows: DisplayRow[] = displayRows.filter((row) => row.kind !== 'header');
    const found =
      mode === 'unified'
        ? modifiedPairsUnified(
            bodyRows.map((row) => (row as Extract<DisplayRow, { kind: 'u-line' }>).row),
          )
        : modifiedPairs(bodyRows.map((row) => (row as Extract<DisplayRow, { kind: 'pair' }>).row));
    const map = new Map<number, readonly [string, string]>();
    let bodyCursor = 0;
    for (let index = 0; index < displayRows.length; index += 1) {
      const displayRow = displayRows[index];
      if (displayRow === undefined || displayRow.kind === 'header') {
        continue;
      }
      const foundPair = found.get(bodyCursor);
      if (foundPair !== undefined) {
        map.set(index, foundPair);
      }
      bodyCursor += 1;
    }
    return map;
  }, [displayRows, charDiffEnabled, mode]);

  /**
   * 某个"行对象"是否被选中。
   *
   * 并排视图里一行显示的是 (左, 右) 两个 DiffLine，所以这里以**行对象**为入口，
   * 而不是以位置为入口 —— 位置在视图之间会变，行对象不会。
   */
  const isSelectedLine = (hunkIndex: number, line: DiffLine): boolean => {
    const lineIndex = lineIndexOf.get(line);
    return lineIndex !== undefined && (selection.get(hunkIndex)?.has(lineIndex) ?? false);
  };

  const pickLine = (line: DiffLine, hunkIndex: number, additive: boolean) => {
    const lineIndex = lineIndexOf.get(line);
    if (lineIndex === undefined) {
      return;
    }
    const position: SelectableLine = { hunkIndex, lineIndex };
    setSelection((previous) => {
      if (additive && anchor.current !== null) {
        return selectRange(hunks, previous, anchor.current, position);
      }
      return toggleLine(previous, position);
    });
    anchor.current = position;
  };

  const selectedCount = countSelected(selection);
  const canStage = target === 'unstaged' && onStage !== undefined;
  const canUnstage = target === 'staged' && onUnstage !== undefined;
  const canDiscard = target === 'unstaged' && onDiscard !== undefined;

  /**
   * 生成补丁的查看参数。
   *
   * 只带 `contextLines`：其余两项（忽略空白、重命名检测）与查询用的是后端默认值，
   * 省略即等价。**"更多上下文"改过的值必须带走** —— 否则后端会用 -U3 重新生成补丁，
   * hunk 的划分与界面看到的不是同一份，下标随之错位。
   */
  const viewSpec: PatchViewSpec = { contextLines: effectiveContextLines };

  const scopeFromSelection = (): StageScope => ({
    kind: 'lines',
    path,
    selections: toLineSelections(selection),
  });

  const applySelection = () => {
    if (isEmptySelection(selection) || busy) {
      return;
    }
    if (canStage) {
      onStage?.(scopeFromSelection(), viewSpec);
    } else if (canUnstage) {
      onUnstage?.(scopeFromSelection(), viewSpec);
    }
  };

  const discardSelection = () => {
    if (isEmptySelection(selection) || busy) {
      return;
    }
    onDiscard?.(scopeFromSelection(), viewSpec);
  };

  /** 整块操作：粒度是 hunk（与"选中行"区分开，用户不必先选行再点）。 */
  const applyHunk = (hunkIndex: number) => {
    if (busy) {
      return;
    }
    const scope: StageScope = { kind: 'hunks', path, hunkIndices: [hunkIndex] };
    if (canStage) {
      onStage?.(scope, viewSpec);
    } else if (canUnstage) {
      onUnstage?.(scope, viewSpec);
    }
  };

  const toggleHunkCollapsed = (hunkIndex: number) => {
    setCollapsed((previous) => {
      const next = new Set(previous);
      if (next.has(hunkIndex)) {
        next.delete(hunkIndex);
      } else {
        next.add(hunkIndex);
      }
      return next;
    });
  };

  const copyPatch = () => {
    // between 模式（提交详情的文件 diff）同样可以复制整个文件的补丁；
    // forceFull：用户要的就是完整补丁，截断会让复制 silently 缺一段
    const spec =
      source === undefined
        ? { target: target ?? 'unstaged', paths: [path], forceFull: true }
        : {
            target: 'between' as const,
            from: source.from,
            to: source.to,
            paths: [path],
            forceFull: true,
          };
    void workspaceDiffPatch(repoId, spec).then((bytes) => {
      void navigator.clipboard.writeText(new TextDecoder().decode(new Uint8Array(bytes)));
    });
  };

  const jumpToHunk = (direction: 1 | -1) => {
    const headerPositions = displayRows
      .map((row, index) => (row.kind === 'header' ? index : -1))
      .filter((index) => index >= 0);
    if (headerPositions.length === 0) {
      return;
    }
    const cursor = direction === 1 ? jumpCursor + 1 : jumpCursor - 1;
    const bounded = (cursor + headerPositions.length) % headerPositions.length;
    setJumpCursor(bounded);
    const target = headerPositions[bounded];
    if (target !== undefined) {
      setScrollTarget(target);
    }
  };

  /**
   * 快捷键：s 暂存选中 / u 取消暂存选中 / d 丢弃选中。
   *
   * 只在容器内生效（容器可聚焦），并且跳过输入控件里的按键 ——
   * 把快捷键挂在 window 上会与页面其他输入抢键，而那类冲突极难排查。
   */
  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) {
      return;
    }
    const element = event.target as HTMLElement | null;
    if (
      element !== null &&
      (element.tagName === 'INPUT' || element.tagName === 'TEXTAREA' || element.isContentEditable)
    ) {
      return;
    }
    if (isEmptySelection(selection) || busy) {
      return;
    }

    const key = event.key.toLowerCase();
    if (key === 's' && canStage) {
      event.preventDefault();
      applySelection();
    } else if (key === 'u' && canUnstage) {
      event.preventDefault();
      applySelection();
    } else if (key === 'd' && canDiscard) {
      event.preventDefault();
      discardSelection();
    }
  };

  if (query.isPending) {
    return (
      <div className={cn('flex flex-col gap-2 p-3', className)} role="status">
        <Skeleton className="h-5 w-2/3" />
        <Skeleton className="h-5 w-full" />
        <Skeleton className="h-5 w-5/6" />
        <span className="sr-only">{t('diff.loading')}</span>
      </div>
    );
  }

  if (query.isError) {
    const normalized = normalizeError(query.error);
    return (
      <ErrorState
        title={t('diff.error')}
        hint={t('diff.errorHint')}
        retryLabel={t('diff.retry')}
        {...(normalized.detail ? { details: normalized.detail } : {})}
        onRetry={() => {
          void query.refetch();
        }}
      />
    );
  }

  if (file === undefined) {
    return <div className={cn('p-3 text-12 text-fg-subtle', className)}>{t('diff.noFile')}</div>;
  }

  if (file.binary) {
    return (
      <div className={cn('flex flex-col items-start gap-1.5 p-3', className)}>
        <p className="text-13">{t('diff.binary.title')}</p>
        <p className="text-12 text-fg-subtle">{t('diff.binary.hint')}</p>
        <div className="flex items-center gap-2">
          <Button variant="secondary" size="sm" onClick={copyPatch}>
            {t('diff.copyPatch')}
          </Button>
          {/* 二进制没有可裁剪的文本补丁，只能整体暂存 / 取消暂存 */}
          {canStage ? (
            <Button
              size="sm"
              loading={busy}
              onClick={() => onStage?.({ kind: 'files', paths: [path] }, viewSpec)}
            >
              {t('diff.binary.stageWhole')}
            </Button>
          ) : null}
          {canUnstage ? (
            <Button
              size="sm"
              loading={busy}
              onClick={() => onUnstage?.({ kind: 'files', paths: [path] }, viewSpec)}
            >
              {t('diff.binary.unstageWhole')}
            </Button>
          ) : null}
        </div>
      </div>
    );
  }

  const hunkActionLabel = canStage
    ? t('diff.stageHunk')
    : canUnstage
      ? t('diff.unstageHunk')
      : undefined;

  return (
    <div
      className={cn('flex min-h-0 flex-col outline-none', className)}
      data-testid="diff-view"
      tabIndex={0}
      onKeyDown={handleKeyDown}
    >
      <div className="flex items-center gap-1 border-b border-border/60 px-2 py-1">
        <ToggleGroup
          label={t('diff.mode.label')}
          value={mode}
          onValueChange={(next) => {
            void useSettingsStore.getState().setJson(DIFF_VIEW_MODE_KEY, next);
          }}
          options={[
            { value: 'unified', label: t('diff.mode.unified') },
            { value: 'side-by-side', label: t('diff.mode.sideBySide') },
          ]}
        />
        <div className="flex-1" />
        {file.truncated && (
          <Button variant="secondary" size="sm" onClick={() => setForceFull(true)}>
            {t('diff.loadFull')}
          </Button>
        )}
        <IconButton
          variant="ghost"
          size="sm"
          label={t('diff.prevHunk')}
          tooltip={t('diff.prevHunk')}
          onClick={() => {
            jumpToHunk(-1);
          }}
        >
          <ChevronUp className="size-3.5" />
        </IconButton>
        <IconButton
          variant="ghost"
          size="sm"
          label={t('diff.nextHunk')}
          tooltip={t('diff.nextHunk')}
          onClick={() => {
            jumpToHunk(1);
          }}
        >
          <ChevronDown className="size-3.5" />
        </IconButton>
        <IconButton
          variant="ghost"
          size="sm"
          label={t('diff.moreContext')}
          tooltip={t('diff.moreContext')}
          onClick={() => {
            setContextBoosted(true);
            setContextLines((n) => Math.min(n * 4, 200));
          }}
        >
          <UnfoldVertical className="size-3.5" />
        </IconButton>
        <IconButton
          variant="ghost"
          size="sm"
          label={t('diff.copyPatch')}
          tooltip={t('diff.copyPatch')}
          onClick={copyPatch}
        >
          <Copy className="size-3.5" />
        </IconButton>
      </div>

      {file.truncated && (
        <p className="bg-warning/10 px-3 py-1 text-12 text-warning" role="status">
          {t('diff.truncatedBanner')}
        </p>
      )}

      {file.truncated && (
        <p className="px-3 py-1 text-12 text-fg-subtle">{t('diff.selection.truncatedHint')}</p>
      )}

      {canPick && selectedCount === 0 ? (
        <p className="px-3 py-1 text-11 text-fg-subtle">{t('diff.selection.hint')}</p>
      ) : null}

      {/* VirtualList 需要像素高度：容器高度用 ResizeObserver 测量 */}
      <div ref={attachListContainer} className="min-h-0 flex-1 overflow-hidden">
        {listHeight > 0 ? (
          <VirtualList
            items={displayRows}
            itemHeight={ROW_HEIGHT}
            height={listHeight}
            scrollTargetIndex={scrollTarget}
            getKey={displayRowKey}
            label={t('diff.listLabel')}
            className="h-full"
            renderItem={(row, index) => (
              <DisplayRowView
                row={row}
                pair={pairs.get(index)}
                isCollapsed={row.kind === 'header' ? collapsed.has(row.hunkIndex) : false}
                onToggle={toggleHunkCollapsed}
                canPick={canPick}
                isSelectedLine={isSelectedLine}
                onPickLine={pickLine}
                hunkActionLabel={hunkActionLabel}
                onHunkAction={applyHunk}
              />
            )}
          />
        ) : null}
      </div>

      {selectedCount > 0 ? (
        <div
          role="toolbar"
          aria-label={t('diff.selection.toolbar')}
          className="flex flex-wrap items-center gap-2 border-t border-border/60 bg-surface px-2 py-1.5"
        >
          <span className="text-12 text-fg-muted">
            {t('diff.selection.count', { count: selectedCount })}
          </span>
          <span className="hidden text-11 text-fg-subtle sm:inline">
            {t('diff.selection.shortcuts')}
          </span>
          <div className="flex-1" />
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              setSelection(emptySelection());
              anchor.current = null;
            }}
          >
            {t('diff.selection.clear')}
          </Button>
          {canStage || canUnstage ? (
            <Button size="sm" loading={busy} onClick={applySelection}>
              {canStage ? t('diff.selection.stage') : t('diff.selection.unstage')}
            </Button>
          ) : null}
          {canDiscard ? (
            <Button size="sm" variant="danger" loading={busy} onClick={discardSelection}>
              {t('diff.selection.discard')}
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** 渲染显示序列的一行（模块级组件：显式接收全部依赖，避免闭包作用域纠缠）。 */
function DisplayRowView({
  row,
  pair,
  isCollapsed,
  onToggle,
  canPick,
  isSelectedLine,
  onPickLine,
  hunkActionLabel,
  onHunkAction,
}: {
  readonly row: DisplayRow;
  readonly pair: readonly [string, string] | undefined;
  readonly isCollapsed: boolean;
  readonly onToggle: (hunkIndex: number) => void;
  readonly canPick: boolean;
  readonly isSelectedLine: (hunkIndex: number, line: DiffLine) => boolean;
  readonly onPickLine: (line: DiffLine, hunkIndex: number, additive: boolean) => void;
  readonly hunkActionLabel: string | undefined;
  readonly onHunkAction: (hunkIndex: number) => void;
}) {
  if (row.kind === 'header') {
    return (
      <HunkHeader
        hunk={row.hunk}
        hunkIndex={row.hunkIndex}
        isCollapsed={isCollapsed}
        onToggle={onToggle}
        {...(hunkActionLabel === undefined ? {} : { actionLabel: hunkActionLabel })}
        onAction={onHunkAction}
      />
    );
  }
  if (row.kind === 'u-line') {
    const line = row.row.line;
    const tone =
      line.kind === 'added' ? 'bg-success/12' : line.kind === 'removed' ? 'bg-danger/12' : '';
    const sign = line.kind === 'added' ? '+' : line.kind === 'removed' ? '-' : '';
    const isModified = pair !== undefined && (line.kind === 'added' || line.kind === 'removed');
    const selected = isSelectedLine(row.row.hunkIndex, line);
    // 只有"该侧真的有行号"的那一列才可点：删除行没有 newNo、新增行没有 oldNo，
    // 否则会出现一个标签为空的可点击行号（既不可读，也无法被测试定位）。
    const pickable = canPick && isSelectable(line.kind);
    return (
      <div className={cn('flex h-6 items-center', tone)}>
        <LineNo
          value={line.oldNo}
          canPick={pickable && line.oldNo != null}
          selected={selected}
          onPick={(additive) => onPickLine(line, row.row.hunkIndex, additive)}
        />
        <LineNo
          value={line.newNo}
          canPick={pickable && line.newNo != null}
          selected={selected}
          onPick={(additive) => onPickLine(line, row.row.hunkIndex, additive)}
        />
        <span className="w-4 shrink-0 select-none text-center text-11 text-fg-subtle">{sign}</span>
        <span className="min-w-0 flex-1 overflow-hidden pr-2">
          {isModified && pair !== undefined ? (
            <CharDiffText
              before={pair[0]}
              after={pair[1]}
              side={line.kind === 'removed' ? 'before' : 'after'}
            />
          ) : (
            <span className={CONTENT_CLASS}>{line.content}</span>
          )}
        </span>
      </div>
    );
  }
  const left = row.row.left;
  const right = row.row.right;
  return (
    <div className="flex h-6 items-stretch">
      <div className="flex min-w-0 flex-1">
        <Cell
          line={left}
          charPair={pair}
          side="old"
          canPick={canPick}
          selected={left !== null && isSelectedLine(row.row.hunkIndex, left)}
          onPick={(line, additive) => onPickLine(line, row.row.hunkIndex, additive)}
        />
      </div>
      <div className="w-px shrink-0 bg-border/60" aria-hidden="true" />
      <div className="flex min-w-0 flex-1">
        <Cell
          line={right}
          charPair={pair}
          side="new"
          canPick={canPick}
          selected={right !== null && isSelectedLine(row.row.hunkIndex, right)}
          onPick={(line, additive) => onPickLine(line, row.row.hunkIndex, additive)}
        />
      </div>
    </div>
  );
}

function displayRowKey(row: DisplayRow): string {
  if (row.kind === 'header') {
    return 'header-' + row.hunkIndex;
  }
  if (row.kind === 'u-line') {
    const line = row.row.line;
    return (
      'u-' +
      row.row.hunkIndex +
      '-' +
      (line.oldNo ?? 'x') +
      '-' +
      (line.newNo ?? 'x') +
      '-' +
      line.kind +
      '-' +
      line.content.length
    );
  }
  const left = row.row.left;
  const right = row.row.right;
  return (
    'p-' +
    row.row.hunkIndex +
    '-' +
    (left?.oldNo ?? 'x') +
    '-' +
    (right?.newNo ?? 'x') +
    '-' +
    String(left?.kind ?? 'null') +
    String(right?.kind ?? 'null')
  );
}
