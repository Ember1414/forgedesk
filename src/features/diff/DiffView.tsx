/**
 * Diff 查看器（T1.5）。
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
 */
import { useMemo, useRef, useState } from 'react';

import { useQuery } from '@tanstack/react-query';
import { ChevronDown, ChevronUp, Copy, Scissors, UnfoldVertical } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import {
  changedLineCount,
  CHAR_DIFF_MAX_CHANGED_LINES,
  flattenUnified,
  modifiedPairs,
  modifiedPairsUnified,
  pairSideBySide,
} from '@/features/diff/diffModel';
import type { PairRow, UnifiedRow } from '@/features/diff/diffModel';
import { useSettingsStore } from '@/stores/settingsStore';
import { normalizeError } from '@/lib/errors';
import { workspaceDiff, workspaceDiffPatch } from '@/lib/ipc/workspace';
import type { DiffHunk, DiffLine } from '@/lib/ipc/workspace';
import { Button } from '@/ui/components/button';
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
  readonly target: 'staged' | 'unstaged';
  readonly className?: string;
}

/** 显示序列的一行：hunk 头 / 内联行 / 并排行。 */
type DisplayRow =
  | { readonly kind: 'header'; readonly hunkIndex: number; readonly hunk: DiffHunk }
  | { readonly kind: 'u-line'; readonly row: UnifiedRow }
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
/** 行号列（并排视图半边一个；内联视图两个）。 */
function LineNo({ value }: { readonly value: number | null | undefined }) {
  return (
    <span className="w-12 shrink-0 select-none pr-1.5 text-right text-11 text-fg-subtle tabular-nums">
      {value ?? ''}
    </span>
  );
}

/** 单元格：符号 + 内容（并排视图的一半；空侧渲染为灰底占位）。 */
function Cell({
  line,
  charPair,
  side,
}: {
  readonly line: DiffLine | null;
  readonly charPair: readonly [string, string] | undefined;
  readonly side: 'old' | 'new';
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
      <LineNo value={side === 'old' ? line.oldNo : line.newNo} />
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

/** hunk 头行：范围 + 函数上下文 + 折叠开关 + 复制 hunk。 */
function HunkHeader({
  hunk,
  hunkIndex,
  isCollapsed,
  onToggle,
}: {
  readonly hunk: DiffHunk;
  readonly hunkIndex: number;
  readonly isCollapsed: boolean;
  readonly onToggle: (hunkIndex: number) => void;
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
export function DiffView({ repoId, path, target, className }: DiffViewProps) {
  const { t } = useTranslation('shell');
  const [contextLines, setContextLines] = useState(3);
  const [forceFull, setForceFull] = useState(false);
  const [collapsed, setCollapsed] = useState<ReadonlySet<number>>(new Set());
  const [scrollTarget, setScrollTarget] = useState(-1);
  const [jumpCursor, setJumpCursor] = useState(-1);

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
    queryKey: ['diff', repoId, target, path, contextLines, forceFull],
    queryFn: () => workspaceDiff(repoId, { target, paths: [path], contextLines, forceFull }),
  });

  const settings = useSettingsStore();
  const mode = settings.getJson<DiffViewMode>(DIFF_VIEW_MODE_KEY, 'unified');

  const file = query.data?.files.find((candidate) => candidate.path === path);
  const hunks = useMemo(() => file?.hunks ?? [], [file]);
  const charDiffEnabled = changedLineCount(hunks) <= CHAR_DIFF_MAX_CHANGED_LINES;

  const displayRows = useMemo<readonly DisplayRow[]>(() => {
    const rows: DisplayRow[] = [];
    hunks.forEach((hunk, hunkIndex) => {
      rows.push({ kind: 'header', hunkIndex, hunk });
      if (collapsed.has(hunkIndex)) {
        return;
      }
      if (mode === 'unified') {
        for (const row of flattenUnified([hunk])) {
          rows.push({ kind: 'u-line', row });
        }
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

  const toggleHunk = (hunkIndex: number) => {
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
    void workspaceDiffPatch(repoId, { target, paths: [path], forceFull: true }).then((bytes) => {
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
        <Button variant="secondary" size="sm" onClick={copyPatch}>
          {t('diff.copyPatch')}
        </Button>
      </div>
    );
  }

  return (
    <div className={cn('flex min-h-0 flex-col', className)} data-testid="diff-view">
      <div className="flex items-center gap-1 border-b border-border/60 px-2 py-1">
        <ToggleGroup
          label={t('diff.mode.label')}
          value={mode}
          onValueChange={(next) => {
            void settings.setJson(DIFF_VIEW_MODE_KEY, next);
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
                onToggle={toggleHunk}
              />
            )}
          />
        ) : null}
      </div>
    </div>
  );
}
/** 渲染显示序列的一行（模块级组件：显式接收全部依赖，避免闭包作用域纠缠）。 */
function DisplayRowView({
  row,
  pair,
  isCollapsed,
  onToggle,
}: {
  readonly row: DisplayRow;
  readonly pair: readonly [string, string] | undefined;
  readonly isCollapsed: boolean;
  readonly onToggle: (hunkIndex: number) => void;
}) {
  if (row.kind === 'header') {
    return (
      <HunkHeader
        hunk={row.hunk}
        hunkIndex={row.hunkIndex}
        isCollapsed={isCollapsed}
        onToggle={onToggle}
      />
    );
  }
  if (row.kind === 'u-line') {
    const line = row.row.line;
    const tone =
      line.kind === 'added' ? 'bg-success/12' : line.kind === 'removed' ? 'bg-danger/12' : '';
    const sign = line.kind === 'added' ? '+' : line.kind === 'removed' ? '-' : '';
    const isModified = pair !== undefined && (line.kind === 'added' || line.kind === 'removed');
    return (
      <div className={cn('flex h-6 items-center', tone)}>
        <LineNo value={line.oldNo} />
        <LineNo value={line.newNo} />
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
  return (
    <div className="flex h-6 items-stretch">
      <div className="flex min-w-0 flex-1">
        <Cell line={row.row.left} charPair={pair} side="old" />
      </div>
      <div className="w-px shrink-0 bg-border/60" aria-hidden="true" />
      <div className="flex min-w-0 flex-1">
        <Cell line={row.row.right} charPair={pair} side="new" />
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
