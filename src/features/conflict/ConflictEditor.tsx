//! 三栏冲突编辑器（T3.2）——**卡片流**设计。
//!
//! # 交互设计（与任务书参考布局的差异，见验收审批说明）
//!
//! 任务书的参考布局是"左 ours / 中 result / 右 theirs 的行对齐三栏"。
//! 本实现刻意改为**逐块卡片流**：每个冲突块一张卡片（卡内左右两栏对照
//! ours / theirs + 块操作按钮 + 该块的结果编辑区），非冲突上下文在
//! "全文"模式下以只读折叠段穿插。理由：
//!
//! 1. 行对齐三栏的同步滚动与列宽对齐是这类编辑器的复杂度大源头，
//!    而且正是 GitKraken / Fork 的既有形态（红线 R3 要求不复刻）；
//! 2. 卡片流天然支持"仅显示冲突块"与逐块独立操作，块与块之间没有
//!    行号耦合，20 文件 200 块的场景下每张卡可以独立重渲；
//! 3. "结果可直接编辑"落在**每个冲突块的结果区**上（编辑即标记
//!    自定义），语义与任务书一致，只是粒度从全文缩到块。
//!
//! # 数据流
//!
//! `git_conflict_file_detail` 给出块序列 → 前端维护每块的解决状态
//! （`BlockState`，纯内存）→ `assembleResult` 拼出结果文本 →
//! `git_conflict_apply_resolution` 写回（EOL/BOM 由后端重建）。
//! 二进制与删除类冲突走 `git_conflict_take_side` / `git_conflict_remove_file`。

import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  ArrowDown,
  ArrowUp,
  FileWarning,
  Redo2,
  Trash2,
  Undo2,
  ZoomIn,
  ZoomOut,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { EmptyState } from '@/ui/components/empty-state';
import { ErrorState } from '@/ui/components/error-state';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/ui/components/alert-dialog';
import { Button } from '@/ui/components/button';
import { Checkbox } from '@/ui/components/checkbox';
import { ToggleGroup } from '@/ui/components/toggle-group';
import { useAppError } from '@/lib/errors';
import {
  gitConflictApplyResolution,
  gitConflictContinue,
  gitConflictFileDetail,
  gitConflictMarkResolved,
  gitConflictRemoveFile,
  gitConflictTakeSide,
} from '@/lib/ipc';
import type { ConflictFileDetail, MergeBlock, TakeSide } from '@/lib/ipc';
import { conflictKey } from '@/lib/queryKeys';
import {
  assembleResult,
  conflictMarkerLines,
  findConflictMarkers,
  INITIAL_BLOCK_STATE,
  wordDiffLine,
} from '@/features/conflict/mergeText';
import type { BlockResolution, BlockState } from '@/features/conflict/mergeText';

/** 冲突块的行数上限：超过它关闭行内词级高亮（任务书第 4 条）。 */
const WORD_DIFF_MAX_LINES = 500;

/** 字号档位（编辑器内容用，用户显式调节，不属于设计 token 语义）。 */
const FONT_SIZES = [12, 14, 16] as const;

/** 一行带词级高亮的文本。 */
function HighlightLine({
  spans,
  lineHeight,
}: {
  readonly spans: readonly { text: string; changed: boolean }[];
  readonly lineHeight: number;
}) {
  return (
    <span style={{ lineHeight: `${lineHeight}px` }}>
      {spans.map((span, index) =>
        span.changed ? (
          <span key={index} className="rounded-xs bg-warning/25" data-changed>
            {span.text}
          </span>
        ) : (
          <span key={index}>{span.text}</span>
        ),
      )}
    </span>
  );
}

/** 代码行序列（可选行内高亮）。 */
function CodeLines({
  lines,
  otherLines,
  side,
  diffOn,
  fontPx,
}: {
  readonly lines: readonly string[];
  readonly otherLines: readonly string[];
  readonly side: 'ours' | 'theirs';
  readonly diffOn: boolean;
  readonly fontPx: number;
}) {
  return (
    <pre
      className="min-w-0 whitespace-pre-wrap break-words p-2 font-mono"
      style={{ fontSize: fontPx }}
    >
      {lines.map((line, index) => {
        const pair =
          diffOn && otherLines[index] !== undefined ? wordDiffLine(line, otherLines[index]) : null;
        const spans = pair === null ? null : side === 'ours' ? pair.ours : pair.theirs;
        return (
          <div key={index}>
            {spans === null ? (
              <span style={{ lineHeight: `${fontPx + 6}px` }}>{line}</span>
            ) : (
              <HighlightLine spans={spans} lineHeight={fontPx + 6} />
            )}
            {line.length === 0 ? '\u00A0' : null}
          </div>
        );
      })}
    </pre>
  );
}

/** 单个冲突块卡片。
 *
 * memo：200 块场景下每次块操作只应重渲**发生变化的那一张**——
 * 其余卡的 props（块数据、状态、稳定的回调）都不变，跳过重渲
 * 是"操作延迟 < 100ms"验收的主要手段。 */
const ConflictCard = memo(function ConflictCard({
  block,
  index,
  blockIndex,
  total,
  state,
  setBlockState,
  showBase,
  fontPx,
  selected,
}: {
  readonly block: Extract<MergeBlock, { type: 'conflict' }>;
  /** 显示序号（第几处冲突，1 起）。 */
  readonly index: number;
  /** 该块在 blocks 全序里的下标（解决状态数组的定位键）。 */
  readonly blockIndex: number;
  readonly total: number;
  readonly state: BlockState | undefined;
  readonly setBlockState: (index: number, state: BlockState) => void;
  readonly showBase: boolean;
  readonly fontPx: number;
  /** 是否为键盘流的当前选中块（视觉高亮，j/k 移动）。 */
  readonly selected: boolean;
}) {
  const { t } = useTranslation('shell');
  const resolution = state?.resolution ?? 'unresolved';
  const resolved = resolution !== 'unresolved';
  const diffOn = block.ours.length + block.theirs.length <= WORD_DIFF_MAX_LINES;
  const customText = state?.customText ?? conflictMarkerLines(block).join('\n');
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const resolvedLabel =
    resolution === 'unresolved' ? null : t(`pages.repoConflict.editor.resolution.${resolution}`);

  return (
    <article
      className={
        selected
          ? 'flex flex-col gap-2 rounded-md border border-brand p-3'
          : 'flex flex-col gap-2 rounded-md border border-line p-3'
      }
      role="group"
      // 读屏信息（T3.3 任务书第 5 条）："第 N 块冲突，本地 M 行，远端 K 行"
      aria-label={t('pages.repoConflict.editor.blockAria', {
        current: index + 1,
        total,
        oursLines: block.ours.length,
        theirsLines: block.theirs.length,
      })}
      data-conflict-card=""
      data-selected={selected ? 'true' : undefined}
      data-resolved={resolved ? 'true' : 'false'}
    >
      <header className="flex flex-wrap items-center gap-2">
        <FileWarning aria-hidden className="size-4 shrink-0 text-warning" />
        <span className="text-sm font-medium">
          {t('pages.repoConflict.editor.conflictN', { current: index + 1, total })}
        </span>
        {resolvedLabel === null ? null : (
          <span className="rounded border border-line px-1.5 py-0.5 text-xs text-fg-muted">
            {resolvedLabel}
          </span>
        )}
      </header>

      <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
        <section
          aria-label={t('pages.repoConflict.editor.sideOurs')}
          tabIndex={0}
          className="rounded border border-line bg-surface-raised focus:outline focus:outline-1 focus:outline-brand/40"
        >
          <header className="border-b border-line px-2 py-1 text-xs font-medium text-fg-muted">
            {t('pages.repoConflict.editor.sideOurs')}
          </header>
          <CodeLines
            lines={block.ours}
            otherLines={block.theirs}
            side="ours"
            diffOn={diffOn}
            fontPx={fontPx}
          />
        </section>
        <section
          aria-label={t('pages.repoConflict.editor.sideTheirs')}
          tabIndex={0}
          className="rounded border border-line bg-surface-raised focus:outline focus:outline-1 focus:outline-brand/40"
        >
          <header className="border-b border-line px-2 py-1 text-xs font-medium text-fg-muted">
            {t('pages.repoConflict.editor.sideTheirs')}
          </header>
          <CodeLines
            lines={block.theirs}
            otherLines={block.ours}
            side="theirs"
            diffOn={diffOn}
            fontPx={fontPx}
          />
        </section>
      </div>

      {showBase && block.base.length > 0 ? (
        <section
          aria-label={t('pages.repoConflict.editor.sideBase')}
          className="rounded border border-line"
        >
          <header className="border-b border-line px-2 py-1 text-xs text-fg-muted">
            {t('pages.repoConflict.editor.sideBase')}
          </header>
          <pre
            className="whitespace-pre-wrap break-words p-2 font-mono"
            style={{ fontSize: fontPx }}
          >
            {block.base.join('\n')}
          </pre>
        </section>
      ) : null}

      <label className="flex flex-col gap-1">
        <span className="text-xs text-fg-muted">{t('pages.repoConflict.editor.resultLabel')}</span>
        <textarea
          ref={textareaRef}
          className="min-h-16 rounded border border-line bg-surface p-2 font-mono focus:border-line-strong focus:outline-none"
          style={{ fontSize: fontPx }}
          value={customText}
          readOnly={resolution !== 'custom'}
          onChange={(event) =>
            setBlockState(blockIndex, { resolution: 'custom', customText: event.target.value })
          }
          data-testid={`conflict-result-${index}`}
        />
      </label>

      <footer className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="secondary"
          onClick={() => setBlockState(blockIndex, { resolution: 'ours' })}
          data-testid={`conflict-adopt-ours-${index}`}
        >
          {t('pages.repoConflict.editor.adoptOurs')}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => setBlockState(blockIndex, { resolution: 'theirs' })}
          data-testid={`conflict-adopt-theirs-${index}`}
        >
          {t('pages.repoConflict.editor.adoptTheirs')}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => setBlockState(blockIndex, { resolution: 'bothOursFirst' })}
          data-testid={`conflict-both-of-${index}`}
        >
          {t('pages.repoConflict.editor.bothOursFirst')}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => setBlockState(blockIndex, { resolution: 'bothTheirsFirst' })}
          data-testid={`conflict-both-tf-${index}`}
        >
          {t('pages.repoConflict.editor.bothTheirsFirst')}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => {
            setBlockState(blockIndex, { resolution: 'custom', customText });
            textareaRef.current?.focus();
          }}
          data-testid={`conflict-manual-${index}`}
        >
          {t('pages.repoConflict.editor.manualEdit')}
        </Button>
      </footer>
    </article>
  );
});

/** 编辑器主体（detail 加载成功后的内容区）。 */
function EditorBody({
  detail,
  repoId,
  onResolved,
}: {
  readonly detail: ConflictFileDetail;
  readonly repoId: number;
  readonly onResolved: () => void;
}) {
  const { t } = useTranslation('shell');
  const appError = useAppError();
  const queryClient = useQueryClient();

  const conflicts = useMemo(
    () =>
      detail.blocks.filter(
        (block): block is Extract<MergeBlock, { type: 'conflict' }> => block.type === 'conflict',
      ),
    [detail.blocks],
  );

  const [states, setStates] = useState<(BlockState | undefined)[]>(() =>
    detail.blocks.map(() => undefined),
  );
  const [undoStack, setUndoStack] = useState<(BlockState | undefined)[][]>([]);
  const [redoStack, setRedoStack] = useState<(BlockState | undefined)[][]>([]);
  const [showBase, setShowBase] = useState(false);
  const [view, setView] = useState<'conflicts' | 'all'>('conflicts');
  const [fontIndex, setFontIndex] = useState(1);
  const [markersOpen, setMarkersOpen] = useState(false);
  const [pendingContent, setPendingContent] = useState<string | null>(null);
  const [pendingContinue, setPendingContinue] = useState(false);
  // 键盘流：当前选中的冲突块（显示序号，0 起）；批量 / 文件级操作的确认框
  const [selectedConflict, setSelectedConflict] = useState<number | null>(null);
  const [batchOpen, setBatchOpen] = useState<'ours' | 'theirs' | null>(null);
  const [fileSideOpen, setFileSideOpen] = useState<TakeSide | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // detail 变化时在**渲染期间**重置全部编辑状态（React 官方的
  // "adjust state during rendering" 模式，替代 effect 重置——后者会
  // 触发级联渲染，react-hooks/set-state-in-effect 会拦）
  const [prevDetail, setPrevDetail] = useState(detail);
  if (prevDetail !== detail) {
    setPrevDetail(detail);
    setStates(detail.blocks.map(() => undefined));
    setUndoStack([]);
    setRedoStack([]);
  }

  const updateStates = useCallback(
    (next: (previous: (BlockState | undefined)[]) => (BlockState | undefined)[]) => {
      setStates((previous) => {
        setUndoStack((stack) => [...stack.slice(-49), previous]);
        setRedoStack([]);
        return next(previous);
      });
    },
    [],
  );

  const setBlockState = useCallback(
    (index: number, blockState: BlockState) => {
      updateStates((previous) => {
        const copy = [...previous];
        copy[index] = blockState;
        return copy;
      });
    },
    [updateStates],
  );

  const undo = useCallback(() => {
    setUndoStack((stack) => {
      const last = stack[stack.length - 1];
      if (last === undefined) {
        return stack;
      }
      setStates(last);
      setRedoStack((redo) => [...redo, last]);
      return stack.slice(0, -1);
    });
  }, []);

  const redo = useCallback(() => {
    setRedoStack((stack) => {
      const last = stack[stack.length - 1];
      if (last === undefined) {
        return stack;
      }
      setStates(last);
      setUndoStack((undo) => [...undo, last]);
      return stack.slice(0, -1);
    });
  }, []);

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: conflictKey(repoId) });
  };

  const continueMutation = useMutation({
    mutationFn: () => gitConflictContinue(repoId),
    onSuccess: invalidate,
    onError: appError.show,
  });

  const takeSideMutation = useMutation({
    mutationFn: (side: TakeSide) => gitConflictTakeSide(repoId, detail.path, side),
    onSuccess: () => {
      invalidate();
      onResolved();
    },
    onError: appError.show,
  });

  const applyMutation = useMutation({
    mutationFn: (content: string) =>
      gitConflictApplyResolution(repoId, detail.path, {
        content,
        eol: detail.eol,
        bom: detail.bom,
        trailingNewline: detail.trailingNewline,
      }),
    onSuccess: () => {
      invalidate();
      onResolved();
    },
    onError: appError.show,
  });

  const markOnlyMutation = useMutation({
    mutationFn: () => gitConflictMarkResolved(repoId, [detail.path]),
    onSuccess: () => {
      invalidate();
      onResolved();
    },
    onError: appError.show,
  });

  // 按块索引定位到每个 conflict 块在 detail.blocks 里的下标
  const conflictIndexes = useMemo(
    () =>
      detail.blocks
        .map((block, index) => (block.type === 'conflict' ? index : -1))
        .filter((index) => index >= 0),
    [detail.blocks],
  );

  const unresolvedIndexes = useMemo(
    () =>
      conflictIndexes.filter(
        (index) => (states[index] ?? INITIAL_BLOCK_STATE).resolution === 'unresolved',
      ),
    [conflictIndexes, states],
  );

  // 上一处 / 下一处**未解决**：都在未解决清单里循环；有跳转动作说明用户
  // 在顺序处理，因此定位后自动把当前块留在原地（不自动采用）
  const scrollToConflict = useCallback(
    (offset: number) => {
      if (unresolvedIndexes.length === 0) {
        return;
      }
      const cards = listRef.current?.querySelectorAll('[data-conflict-card]');
      if (cards === undefined) {
        return;
      }
      // 找到视口上方最近的未解决卡（简化：按未解决清单顺序 + offset）
      const target =
        unresolvedIndexes.length === 1
          ? 0
          : (offset + unresolvedIndexes.length) % unresolvedIndexes.length;
      const index = unresolvedIndexes[target];
      if (index === undefined) {
        return;
      }
      cards[index]?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    },
    [unresolvedIndexes],
  );

  const resultText = useMemo(() => assembleResult(detail.blocks, states), [detail.blocks, states]);
  const markers = useMemo(() => findConflictMarkers(resultText), [resultText]);

  const unresolvedCount = unresolvedIndexes.length;

  /** 保存（T3.3：`thenContinue` 为真时保存成功后自动 continue——Ctrl+Enter 的
   *  "标记并继续"；还有未解决块时只保存，continue 由服务层的前置校验兜底）。 */
  const requestSave = useCallback(
    (thenContinue: boolean) => {
      if (markers.length > 0) {
        setPendingContent(resultText);
        setPendingContinue(thenContinue);
        setMarkersOpen(true);
        return;
      }
      applyMutation.mutate(resultText, {
        onSuccess: () => {
          if (thenContinue && unresolvedCount === 0) {
            continueMutation.mutate();
          }
        },
      });
    },
    [applyMutation, continueMutation, markers.length, resultText, unresolvedCount],
  );

  // ---------------------------------------------------------------- 键盘流（T3.3）

  /** 移动选中块：在全部冲突块里循环（j/k 与 ↑/↓ 共用）。 */
  const moveSelected = useCallback(
    (offset: number) => {
      if (conflicts.length === 0) {
        return;
      }
      setSelectedConflict((current) => {
        const base = current === null ? (offset > 0 ? -1 : 0) : current;
        return (base + offset + conflicts.length) % conflicts.length;
      });
    },
    [conflicts.length],
  );

  /** 键盘采用：对选中块应用解决方式，然后把选中点推进到下一个冲突块
   *  （连续解决的节奏：o → 自动到下一块 → o……需要精挑时用 j/k 回退）。 */
  const adoptSelected = useCallback(
    (resolution: BlockResolution) => {
      if (selectedConflict === null) {
        return;
      }
      const blockIndex = conflictIndexes[selectedConflict];
      if (blockIndex === undefined) {
        return;
      }
      setBlockState(blockIndex, { resolution });
      if (conflicts.length > 0) {
        setSelectedConflict((current) =>
          current === null ? null : (current + 1) % conflicts.length,
        );
      }
    },
    [conflictIndexes, conflicts.length, selectedConflict, setBlockState],
  );

  // 选中块滚动到可视区（键盘移动时跟随）
  useEffect(() => {
    if (selectedConflict === null) {
      return;
    }
    const cards = listRef.current?.querySelectorAll('[data-conflict-card]');
    cards?.[selectedConflict]?.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
  }, [selectedConflict]);

  // 全局键盘监听（T3.3 任务书第 1 条）。挂 window 而不是容器：用户焦点可能在
  // 侧栏或工具条上，快捷键仍应可用；输入框内的普通输入不受影响——单键动作
  // （j/k/o/t/b/n/p）在 input/textarea 聚焦时被跳过，Ctrl+S / Ctrl+Enter
  // 是组合键，任何位置都生效。
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const target = event.target;
      const inTextField =
        target instanceof HTMLElement &&
        (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);
      if ((event.ctrlKey || event.metaKey) && (event.key === 's' || event.key === 'S')) {
        event.preventDefault();
        requestSave(false);
        return;
      }
      if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
        event.preventDefault();
        requestSave(true);
        return;
      }
      if (inTextField || event.ctrlKey || event.metaKey || event.altKey) {
        return;
      }
      switch (event.key) {
        case 'j':
        case 'ArrowDown':
          event.preventDefault();
          moveSelected(1);
          break;
        case 'k':
        case 'ArrowUp':
          event.preventDefault();
          moveSelected(-1);
          break;
        case 'o':
          adoptSelected('ours');
          break;
        case 't':
          adoptSelected('theirs');
          break;
        case 'b':
          adoptSelected('bothOursFirst');
          break;
        case 'n':
          event.preventDefault();
          scrollToConflict(1);
          break;
        case 'p':
          event.preventDefault();
          scrollToConflict(-1);
          break;
        default:
          break;
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [adoptSelected, moveSelected, requestSave, scrollToConflict]);

  /** 批量采用（T3.3 任务书第 2 条）：确认框里给出块数与行数后才会执行。 */
  const batchApply = useCallback(
    (resolution: 'ours' | 'theirs') => {
      updateStates((previous) => {
        const copy = [...previous];
        detail.blocks.forEach((block, index) => {
          if (block.type === 'conflict') {
            copy[index] = { resolution };
          }
        });
        return copy;
      });
      setBatchOpen(null);
    },
    [detail.blocks, updateStates],
  );

  const fontPx = FONT_SIZES[fontIndex] ?? FONT_SIZES[1];
  const diffFontPx = FONT_SIZES[fontIndex] ?? FONT_SIZES[1];

  return (
    <div className="flex min-h-0 flex-1 flex-col" data-testid="conflict-editor">
      {/* 工具条 */}
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-4 py-2">
        <Checkbox
          checked={showBase}
          onCheckedChange={(checked) => setShowBase(checked === true)}
          label={t('pages.repoConflict.editor.showBase')}
          data-testid="editor-show-base"
        />
        <ToggleGroup
          label={t('pages.repoConflict.editor.viewLabel')}
          value={view}
          onValueChange={(value) => setView(value as 'conflicts' | 'all')}
          options={[
            { value: 'conflicts', label: t('pages.repoConflict.editor.conflictsOnly') },
            { value: 'all', label: t('pages.repoConflict.editor.allContent') },
          ]}
        />
        <Button
          size="sm"
          variant="ghost"
          onClick={() => scrollToConflict(-1)}
          data-testid="editor-prev-conflict"
        >
          <ArrowUp aria-hidden className="size-4" />
          {t('pages.repoConflict.editor.prevConflict')}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => scrollToConflict(1)}
          data-testid="editor-next-conflict"
        >
          <ArrowDown aria-hidden className="size-4" />
          {t('pages.repoConflict.editor.nextConflict')}
        </Button>
        {/* 批量操作（T3.3 任务书第 2 条）：确认框给出影响量后才执行 */}
        <Button
          size="sm"
          variant="ghost"
          disabled={conflicts.length === 0}
          onClick={() => setBatchOpen('ours')}
          data-testid="editor-batch-ours"
        >
          {t('pages.repoConflict.editor.batchOurs')}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={conflicts.length === 0}
          onClick={() => setBatchOpen('theirs')}
          data-testid="editor-batch-theirs"
        >
          {t('pages.repoConflict.editor.batchTheirs')}
        </Button>
        <span className="ml-auto flex items-center gap-1">
          <Button
            size="sm"
            variant="ghost"
            onClick={undo}
            disabled={undoStack.length === 0}
            data-testid="editor-undo"
          >
            <Undo2 aria-hidden className="size-4" />
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={redo}
            disabled={redoStack.length === 0}
            data-testid="editor-redo"
          >
            <Redo2 aria-hidden className="size-4" />
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setFontIndex((index) => Math.max(0, index - 1))}
            disabled={fontIndex === 0}
          >
            <ZoomOut aria-hidden className="size-4" />
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setFontIndex((index) => Math.min(FONT_SIZES.length - 1, index + 1))}
            disabled={fontIndex === FONT_SIZES.length - 1}
          >
            <ZoomIn aria-hidden className="size-4" />
          </Button>
        </span>
      </div>

      {/* 块列表 */}
      <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto p-4" data-testid="editor-blocks">
        {detail.blocks.map((block, index) => {
          if (block.type === 'conflict') {
            const conflictIndex = conflictIndexes.indexOf(index);
            return (
              <div key={index} className="mb-4">
                <ConflictCard
                  block={block}
                  index={conflictIndex}
                  blockIndex={index}
                  total={conflicts.length}
                  state={states[index]}
                  setBlockState={setBlockState}
                  showBase={showBase}
                  fontPx={diffFontPx}
                  selected={selectedConflict === conflictIndex}
                />
              </div>
            );
          }
          if (view === 'conflicts') {
            return null;
          }
          return (
            <section
              key={index}
              className="mb-4 rounded border border-line"
              data-auto-resolved={block.type === 'resolved' ? 'true' : undefined}
            >
              <header className="border-b border-line px-2 py-1 text-xs text-fg-muted">
                {block.type === 'resolved'
                  ? t(`pages.repoConflict.editor.autoResolved.${block.source}`)
                  : t('pages.repoConflict.editor.context')}
              </header>
              <pre
                className="whitespace-pre-wrap break-words p-2 font-mono"
                style={{ fontSize: fontPx }}
              >
                {block.lines.join('\n')}
                {block.lines.length === 0 ? '\u00A0' : null}
              </pre>
            </section>
          );
        })}
      </div>

      {/* 操作条 */}
      <div className="flex flex-wrap items-center gap-2 border-t border-line px-4 py-3">
        <span className="text-sm text-fg-muted" data-testid="editor-conflict-progress">
          {conflicts.length === 0
            ? t('pages.repoConflict.allResolved')
            : t('pages.repoConflict.editor.conflictProgress', {
                resolved: conflicts.length - unresolvedCount,
                total: conflicts.length,
              })}
        </span>
        <div className="ml-auto flex items-center gap-2">
          {/* 文件级快捷操作（T3.3 任务书第 3 条）：覆盖工作区文件，必须确认 */}
          <Button
            variant="secondary"
            disabled={takeSideMutation.isPending}
            onClick={() => setFileSideOpen('ours')}
            data-testid="editor-file-ours"
          >
            {t('pages.repoConflict.editor.fileOurs')}
          </Button>
          <Button
            variant="secondary"
            disabled={takeSideMutation.isPending}
            onClick={() => setFileSideOpen('theirs')}
            data-testid="editor-file-theirs"
          >
            {t('pages.repoConflict.editor.fileTheirs')}
          </Button>
          <Button
            variant="secondary"
            onClick={() => markOnlyMutation.mutate()}
            disabled={markOnlyMutation.isPending}
            data-testid="editor-mark-only"
          >
            {t('pages.repoConflict.editor.markOnly')}
          </Button>
          <Button
            onClick={() => requestSave(false)}
            disabled={applyMutation.isPending}
            title={t('pages.repoConflict.editor.saveHint')}
            data-testid="editor-save-resolve"
          >
            {t('pages.repoConflict.editor.saveAndResolve')}
          </Button>
        </div>
      </div>

      {/* 残留标记警告（不阻断） */}
      <AlertDialog open={markersOpen} onOpenChange={setMarkersOpen}>
        <AlertDialogContent impact={t('pages.repoConflict.editor.markersImpact')}>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('pages.repoConflict.editor.markersTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('pages.repoConflict.editor.markersBody', { count: markers.length })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('pages.repoConflict.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setMarkersOpen(false);
                if (pendingContent !== null) {
                  applyMutation.mutate(pendingContent, {
                    onSuccess: () => {
                      if (pendingContinue) {
                        continueMutation.mutate();
                      }
                    },
                  });
                }
                setPendingContinue(false);
              }}
              data-testid="editor-markers-keep"
            >
              {t('pages.repoConflict.editor.keepAnyway')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* 批量采用确认（T3.3 任务书第 2 条）：说明会覆盖的块数与行数；取消不产生任何修改 */}
      <AlertDialog
        open={batchOpen !== null}
        onOpenChange={(open) => setBatchOpen(open ? batchOpen : null)}
      >
        <AlertDialogContent
          impact={t('pages.repoConflict.editor.batchImpact', {
            blocks: conflicts.length,
            lines:
              batchOpen === 'ours'
                ? conflicts.reduce((sum, block) => sum + block.ours.length, 0)
                : conflicts.reduce((sum, block) => sum + block.theirs.length, 0),
          })}
          tone="danger"
        >
          <AlertDialogHeader>
            <AlertDialogTitle>
              {batchOpen === 'ours'
                ? t('pages.repoConflict.editor.batchOursTitle')
                : t('pages.repoConflict.editor.batchTheirsTitle')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('pages.repoConflict.editor.batchBody', {
                blocks: conflicts.length,
                side:
                  batchOpen === 'ours'
                    ? t('pages.repoConflict.editor.sideOurs')
                    : t('pages.repoConflict.editor.sideTheirs'),
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-testid="editor-batch-cancel">
              {t('pages.repoConflict.cancel')}
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() => batchApply(batchOpen ?? 'ours')}
              data-testid="editor-batch-confirm"
            >
              {t('pages.repoConflict.editor.batchConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* 整个文件采用一方（T3.3 任务书第 3 条）：覆盖工作区文件与编辑状态 */}
      <AlertDialog
        open={fileSideOpen !== null}
        onOpenChange={(open) => setFileSideOpen(open ? fileSideOpen : null)}
      >
        <AlertDialogContent impact={t('pages.repoConflict.editor.fileImpact')} tone="danger">
          <AlertDialogHeader>
            <AlertDialogTitle>
              {fileSideOpen === 'theirs'
                ? t('pages.repoConflict.editor.fileTheirsTitle')
                : t('pages.repoConflict.editor.fileOursTitle')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('pages.repoConflict.editor.fileBody')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-testid="editor-file-cancel">
              {t('pages.repoConflict.cancel')}
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setFileSideOpen(null);
                if (fileSideOpen !== null) {
                  takeSideMutation.mutate(fileSideOpen);
                }
              }}
              data-testid="editor-file-confirm"
            >
              {fileSideOpen === 'theirs'
                ? t('pages.repoConflict.editor.fileTheirs')
                : t('pages.repoConflict.editor.fileOurs')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** 二进制冲突面板：无法显示差异，走"采用一方"路径。 */
function BinaryPanel({
  detail,
  repoId,
  onResolved,
}: {
  readonly detail: ConflictFileDetail;
  readonly repoId: number;
  readonly onResolved: () => void;
}) {
  const { t } = useTranslation('shell');
  const appError = useAppError();
  const queryClient = useQueryClient();
  const [deleteOpen, setDeleteOpen] = useState(false);

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: conflictKey(repoId) });
  };
  const takeSideMutation = useMutation({
    mutationFn: (side: TakeSide) => gitConflictTakeSide(repoId, detail.path, side),
    onSuccess: () => {
      invalidate();
      onResolved();
    },
    onError: appError.show,
  });
  const removeMutation = useMutation({
    mutationFn: () => gitConflictRemoveFile(repoId, detail.path),
    onSuccess: () => {
      invalidate();
      onResolved();
    },
    onError: appError.show,
  });

  return (
    <div
      className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8"
      data-testid="conflict-binary-panel"
    >
      <FileWarning aria-hidden className="size-8 text-warning" />
      <p className="text-center text-sm text-fg-muted">
        {t('pages.repoConflict.editor.binaryNote')}
      </p>
      <div className="flex flex-wrap items-center justify-center gap-2">
        <Button
          variant="secondary"
          disabled={takeSideMutation.isPending}
          onClick={() => takeSideMutation.mutate('ours')}
          data-testid="binary-take-ours"
        >
          {t('pages.repoConflict.editor.adoptOurs')}
        </Button>
        <Button
          variant="secondary"
          disabled={takeSideMutation.isPending}
          onClick={() => takeSideMutation.mutate('theirs')}
          data-testid="binary-take-theirs"
        >
          {t('pages.repoConflict.editor.adoptTheirs')}
        </Button>
        <Button
          variant="secondary"
          disabled={removeMutation.isPending}
          onClick={() => setDeleteOpen(true)}
          data-testid="binary-delete"
        >
          <Trash2 aria-hidden className="size-4" />
          {t('pages.repoConflict.editor.deleteFile')}
        </Button>
      </div>
      <AlertDialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <AlertDialogContent
          impact={t('pages.repoConflict.editor.deleteImpact', { path: detail.path })}
          tone="danger"
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t('pages.repoConflict.editor.deleteTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('pages.repoConflict.editor.deleteBody')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('pages.repoConflict.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => removeMutation.mutate()}
              data-testid="conflict-delete-confirm"
            >
              {t('pages.repoConflict.editor.deleteFile')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** 删除类冲突面板（文件不在工作区：DeletedByUs / DeletedByThem）。 */
function DeletedPanel({
  detail,
  repoId,
  onResolved,
}: {
  readonly detail: ConflictFileDetail;
  readonly repoId: number;
  readonly onResolved: () => void;
}) {
  const { t } = useTranslation('shell');
  const appError = useAppError();
  const queryClient = useQueryClient();

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: conflictKey(repoId) });
  };
  const takeSideMutation = useMutation({
    mutationFn: (side: TakeSide) => gitConflictTakeSide(repoId, detail.path, side),
    onSuccess: () => {
      invalidate();
      onResolved();
    },
    onError: appError.show,
  });
  const removeMutation = useMutation({
    mutationFn: () => gitConflictRemoveFile(repoId, detail.path),
    onSuccess: () => {
      invalidate();
      onResolved();
    },
    onError: appError.show,
  });
  const [deleteOpen, setDeleteOpen] = useState(false);

  const keepButton =
    detail.kind === 'deletedByUs'
      ? // 我们删了、对方改了：恢复对方的修改
        {
          side: 'theirs' as TakeSide,
          label: t('pages.repoConflict.editor.keepTheirs'),
          testid: 'deleted-keep-theirs',
        }
      : // 对方删了、我们改了：保留我们的修改
        {
          side: 'ours' as TakeSide,
          label: t('pages.repoConflict.editor.keepOurs'),
          testid: 'deleted-keep-ours',
        };

  return (
    <div
      className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8"
      data-testid="conflict-deleted-panel"
    >
      <FileWarning aria-hidden className="size-8 text-warning" />
      <p className="text-center text-sm text-fg-muted">
        {t(`pages.repoConflict.kind.${detail.kind}`)}
      </p>
      <div className="flex flex-wrap items-center justify-center gap-2">
        <Button
          variant="secondary"
          disabled={takeSideMutation.isPending}
          onClick={() => takeSideMutation.mutate(keepButton.side)}
          data-testid={keepButton.testid}
        >
          {keepButton.label}
        </Button>
        <Button
          variant="secondary"
          disabled={removeMutation.isPending}
          onClick={() => setDeleteOpen(true)}
          data-testid="deleted-remove"
        >
          <Trash2 aria-hidden className="size-4" />
          {t('pages.repoConflict.editor.deleteFile')}
        </Button>
      </div>
      <AlertDialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <AlertDialogContent
          impact={t('pages.repoConflict.editor.deleteImpact', { path: detail.path })}
          tone="danger"
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t('pages.repoConflict.editor.deleteTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('pages.repoConflict.editor.deleteBody')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('pages.repoConflict.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => removeMutation.mutate()}
              data-testid="conflict-delete-confirm"
            >
              {t('pages.repoConflict.editor.deleteFile')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** 冲突编辑器（三态：删除类 / 二进制 / 文本）。 */
export function ConflictEditor({
  repoId,
  path,
  onResolved,
}: {
  readonly repoId: number;
  readonly path: string;
  /** 文件被解决（从冲突清单消失）后回调。 */
  readonly onResolved: (path: string) => void;
}) {
  const { t } = useTranslation('shell');
  const detailQuery = useQuery({
    queryKey: ['conflict-detail', repoId, path],
    queryFn: () => gitConflictFileDetail(repoId, path),
    enabled: path.length > 0,
  });

  if (detailQuery.isPending) {
    return (
      <div
        className="flex flex-1 items-center justify-center p-8"
        data-testid="conflict-editor-loading"
      >
        <span className="text-sm text-fg-muted">{t('common:state.loading')}</span>
      </div>
    );
  }
  if (detailQuery.isError || detailQuery.data === undefined) {
    return (
      <div className="p-4">
        <ErrorState
          title={t('pages.repoConflict.title')}
          hint={t('pages.repoConflict.editor.loadFailed')}
          retryLabel={t('common:actions.retry')}
          onRetry={() => void detailQuery.refetch()}
        />
      </div>
    );
  }

  const detail = detailQuery.data;
  if (!detail.worktreeExists) {
    return (
      <DeletedPanel detail={detail} repoId={repoId} onResolved={() => onResolved(detail.path)} />
    );
  }
  const hasText =
    (detail.ours === null || detail.ours.content !== null) &&
    (detail.theirs === null || detail.theirs.content !== null) &&
    (detail.base === null || detail.base.content !== null);
  if (detail.kind === 'binary' || !hasText || detail.blocks.length === 0) {
    return (
      <BinaryPanel detail={detail} repoId={repoId} onResolved={() => onResolved(detail.path)} />
    );
  }

  return <EditorBody detail={detail} repoId={repoId} onResolved={() => onResolved(detail.path)} />;
}

/** 编辑器打开前的占位（未选中文件）。 */
export function EditorPlaceholder() {
  const { t } = useTranslation('shell');
  return (
    <div
      className="flex min-h-0 flex-1 items-center justify-center p-8"
      data-testid="editor-placeholder"
    >
      <EmptyState
        title={t('pages.repoConflict.editor.selectFile')}
        description={t('pages.repoConflict.editor.selectFileHint')}
      />
    </div>
  );
}
