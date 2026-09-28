//! 提交历史的列表模式（T2.2）。
//!
//! # 为什么要有它（AGENTS.md 的无障碍等价路径）
//!
//! 图模式把分支拓扑画在 Canvas 上，对读屏软件是不可见的；`GraphCanvas` 虽然
//! 额外渲染了一份 `role="listbox"` 的文本列，但那是"图的可访问替身"，
//! 交互模型仍是图的。列表模式给的是一条**独立、完整、以表格语义为主**的路径：
//! `role="grid"` + 每行 `aria-posinset/aria-setsize` + `aria-selected`，
//! 键盘 ↑/↓ 移动焦点行、Enter 打开详情、Ctrl+A 全选——不依赖任何 Canvas 能力。
//!
//! # 为什么不直接复用 `ui/components/virtual-list.tsx`
//!
//! `VirtualList` 把 `role="list"` / `role="listitem"` 写死在实现里，而这里需要的是
//! `grid` / `row` / `gridcell` 语义（列头、按列导航、`aria-selected` 行选择）。
//! 该组件不在本任务的可修改范围内，改它的角色会波及工作区面板等既有调用方，
//! 因此这里沿用它的**窗口化思路**（等高行 + 绝对定位 + overscan）自行实现，
//! 视觉语言则与 `ui/components/table.tsx` 保持一致（同样的行高、边框、选中底色）。
//!
//! # 为什么用 `aria-activedescendant` 而不是给每行 tabIndex
//!
//! 与 `GraphCanvas` 的文本列同一套做法：焦点始终留在网格容器上，
//! "当前行"用 `aria-activedescendant` 指向。这样 ↑/↓ 导航不会让 DOM 焦点
//! 在成百上千个行间跳来跳去（那会让读屏软件反复播报"进入/离开"），
//! 也避免了 roving tabindex 在虚拟化下"目标行还没渲染出来"的时序问题。
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent } from 'react';

import { useTranslation } from 'react-i18next';

import { cn } from '@/lib/utils';

import type { RowText } from '@/features/history/commitMeta';
import { laneBgClass, refChipClass } from '@/features/history/graphTheme';
import {
  NO_MODIFIERS,
  useGraphSelectionStore,
  useSelectedOidSet,
} from '@/features/history/graphSelectionStore';
import type { SelectionModifiers } from '@/features/history/graphSelectionStore';

/** 行高（px）；与图模式的 `BASE_ROW_HEIGHT` 接近，保证两种模式切换时节奏一致。 */
const ROW_HEIGHT = 32;
/** 可视区外额外渲染的行数（滚动时减少白屏）。 */
const OVERSCAN_ROWS = 6;
/**
 * 测量不到容器高度时的兜底可视高度（px）。
 *
 * jsdom 里 `clientHeight` 恒为 0、`ResizeObserver` 是空桩，若不兜底就会
 * "一行都不渲染"，列表模式的单测将无从断言。生产环境首帧测量生效后即被覆盖。
 */
const FALLBACK_VIEWPORT_HEIGHT = 600;

/** 六列的网格模板：色点 / 短 oid / 摘要 / 作者 / 时间 / 引用。列头与数据行共用。 */
const GRID_COLUMNS =
  'grid grid-cols-[1rem_5rem_minmax(0,1fr)_7rem_6rem_9rem] items-center gap-2 px-2';

export interface GraphListModeProps {
  /** 与图模式同序同长的行文案（由 `HistoryPage` 注入 i18n 后算好）。 */
  readonly texts: readonly RowText[];
  /** 搜索命中的行（T2.3：淡琥珀底色标记；空集 = 无高亮）。 */
  readonly matchOids?: ReadonlySet<string> | undefined;
  /** 跳转目标（T2.3 的"上一处 / 下一处"）：把它设为焦点行并滚进可视区。 */
  readonly focusOid?: string | null | undefined;
  readonly className?: string;
}

/** 从鼠标事件翻译出 store 认识的修饰键（store 不认识 DOM 事件）。 */
function modifiersFrom(event: {
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly shiftKey: boolean;
}): SelectionModifiers {
  return { additive: event.ctrlKey || event.metaKey, range: event.shiftKey };
}

/** 行 id（`aria-activedescendant` 与测试都用它定位）。 */
function rowId(index: number): string {
  return `fd-history-list-row-${index}`;
}

export function GraphListMode({ texts, matchOids, focusOid, className }: GraphListModeProps) {
  const { t } = useTranslation('shell');

  const select = useGraphSelectionStore((state) => state.select);
  const selectMany = useGraphSelectionStore((state) => state.selectMany);
  const selected = useSelectedOidSet();

  const total = texts.length;
  /** 行序的 oid 列表：区间选与全选都靠它把"点击顺序"归一化成"行顺序"。 */
  const order = useMemo(() => texts.map((text) => text.oid), [texts]);

  const [rawActiveIndex, setRawActiveIndex] = useState(0);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);

  const gridRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // 派生而非 effect：数据变短后 activeIndex 自动夹回合法范围（react-hooks/set-state-in-effect）
  const activeIndex = total === 0 ? 0 : Math.min(rawActiveIndex, total - 1);

  // 测量可视区高度（窗口化需要它算"能放下几行"）
  useEffect(() => {
    const element = scrollRef.current;
    if (element === null) {
      return;
    }
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const height = entry.contentRect.height;
        if (Number.isFinite(height)) {
          setViewportHeight(height);
        }
      }
    });
    observer.observe(element);
    setViewportHeight(element.clientHeight);
    return () => {
      observer.disconnect();
    };
  }, []);

  const effectiveHeight = viewportHeight > 0 ? viewportHeight : FALLBACK_VIEWPORT_HEIGHT;

  // 焦点行变化时把它滚进可视区（否则 ↑/↓ 会"走进"未渲染的区域）
  useEffect(() => {
    const element = scrollRef.current;
    if (element === null || total === 0) {
      return;
    }
    const top = activeIndex * ROW_HEIGHT;
    const bottom = top + ROW_HEIGHT;
    if (top < element.scrollTop) {
      element.scrollTop = top;
    } else if (bottom > element.scrollTop + effectiveHeight) {
      element.scrollTop = bottom - effectiveHeight;
    }
  }, [activeIndex, effectiveHeight, total]);

  // 跳转目标（T2.3）：焦点行切到目标 oid，随后的"滚进可视区"既有 effect
  // 跟着生效。用"props 变化时调整状态"的渲染期模式（有守卫），而不是
  // effect 里 setState（react-hooks 直接禁掉）。
  const [focusedOidApplied, setFocusedOidApplied] = useState<string | null>(null);
  if (focusOid !== null && focusOid !== undefined && focusOid !== focusedOidApplied && total > 0) {
    const index = texts.findIndex((text) => text.oid === focusOid);
    if (index >= 0) {
      setFocusedOidApplied(focusOid);
      setRawActiveIndex(index);
    }
  }

  const firstVisible = Math.floor(scrollTop / ROW_HEIGHT);
  const start = Math.max(0, firstVisible - OVERSCAN_ROWS);
  const visibleCount = Math.ceil(effectiveHeight / ROW_HEIGHT) + OVERSCAN_ROWS * 2;
  const end = Math.min(total, start + visibleCount);
  const visible = texts.slice(start, end);

  const handleKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      if (total === 0) {
        return;
      }
      // Ctrl/Cmd+A 全选（网格语义里的标准快捷键；必须 preventDefault 否则浏览器全选页面）
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') {
        event.preventDefault();
        selectMany(order, order);
        return;
      }
      switch (event.key) {
        case 'ArrowDown':
          event.preventDefault();
          setRawActiveIndex((index) => Math.min(total - 1, index + 1));
          break;
        case 'ArrowUp':
          event.preventDefault();
          setRawActiveIndex((index) => Math.max(0, index - 1));
          break;
        case 'Home':
          event.preventDefault();
          setRawActiveIndex(0);
          break;
        case 'End':
          event.preventDefault();
          setRawActiveIndex(total - 1);
          break;
        case 'Enter':
        case ' ': {
          event.preventDefault();
          const text = texts[activeIndex];
          if (text !== undefined) {
            // select 内部会把 detailOid 一并设为该提交（见 graphSelectionStore），
            // 因此"Enter 打开详情"与"Enter 选中"是同一个动作，不必再调 setDetailOid。
            select(text.oid, NO_MODIFIERS, order);
          }
          break;
        }
        default:
          break;
      }
    },
    [activeIndex, order, select, selectMany, texts, total],
  );

  const handleRowClick = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>, index: number, text: RowText) => {
      setRawActiveIndex(index);
      select(text.oid, modifiersFrom(event), order);
      gridRef.current?.focus();
    },
    [order, select],
  );

  const columns = [
    t('history.list.column.oid'),
    t('history.list.column.subject'),
    t('history.list.column.author'),
    t('history.list.column.time'),
    t('history.list.column.refs'),
  ];

  return (
    <div
      ref={gridRef}
      role="grid"
      tabIndex={0}
      aria-label={t('history.list.gridLabel')}
      aria-colcount={6}
      aria-multiselectable
      aria-activedescendant={total === 0 ? undefined : rowId(activeIndex)}
      onKeyDown={handleKeyDown}
      className={cn(
        'flex h-full min-h-0 flex-col overflow-hidden rounded-md border border-line bg-surface',
        'focus-visible:outline focus-visible:outline-2 focus-visible:outline-brand',
        className,
      )}
      data-testid="graph-list"
    >
      {/* 列头（不随内容滚动） */}
      <div role="rowgroup" className="shrink-0 border-b border-line text-12 text-fg-subtle">
        <div role="row" className={cn(GRID_COLUMNS, 'h-8')}>
          {/* 首列是分支色点，纯装饰（颜色在图模式里与泳道对应），故列头无名。 */}
          <div role="columnheader" />
          {columns.map((label) => (
            <div key={label} role="columnheader" className="truncate font-medium">
              {label}
            </div>
          ))}
        </div>
      </div>

      {/* 数据区（虚拟化滚动） */}
      <div
        ref={scrollRef}
        role="rowgroup"
        className="min-h-0 flex-1 overflow-y-auto"
        onScroll={(event) => {
          setScrollTop(event.currentTarget.scrollTop);
        }}
      >
        {total === 0 ? (
          <div className="px-2 py-8 text-center text-13 text-fg-muted">
            {t('history.list.empty')}
          </div>
        ) : (
          <div className="relative" style={{ height: total * ROW_HEIGHT }}>
            {visible.map((text, offset) => {
              const index = start + offset;
              const isSelected = selected.has(text.oid);
              const isActive = index === activeIndex;
              const isMatch = matchOids !== undefined && matchOids.has(text.oid);
              return (
                <div
                  key={text.oid}
                  id={rowId(index)}
                  role="row"
                  aria-selected={isSelected}
                  aria-posinset={index + 1}
                  aria-setsize={total}
                  data-oid={text.oid}
                  data-active={isActive ? '' : undefined}
                  onClick={(event) => {
                    handleRowClick(event, index, text);
                  }}
                  className={cn(
                    'fd-transition absolute inset-x-0 grid grid-cols-[1rem_5rem_minmax(0,1fr)_7rem_6rem_9rem] items-center gap-2 px-2 text-13',
                    isSelected ? 'bg-brand-subtle' : 'hover:bg-surface-sunken',
                    isMatch && !isSelected ? 'bg-warning/15' : undefined,
                    isActive ? 'outline outline-1 -outline-offset-1 outline-brand' : undefined,
                    text.hidden ? 'text-fg-muted' : undefined,
                  )}
                  style={{ top: index * ROW_HEIGHT, height: ROW_HEIGHT }}
                >
                  <div role="gridcell" className="flex justify-center">
                    <span
                      aria-hidden="true"
                      className={cn('size-2.5 rounded-full', laneBgClass(text.colorIndex))}
                    />
                  </div>
                  <div role="gridcell" className="truncate font-mono text-12 text-fg-subtle">
                    {text.shortOid}
                  </div>
                  <div role="gridcell" className="flex min-w-0 items-center gap-1.5">
                    <span className="truncate" title={text.subject}>
                      {text.subject}
                    </span>
                    {text.collapsed > 0 ? (
                      <span
                        className="shrink-0 rounded-full bg-surface-sunken px-1.5 text-10 text-fg-subtle"
                        title={t('history.detail.collapsed', { count: text.collapsed })}
                      >
                        {t('history.collapsed', { count: text.collapsed })}
                      </span>
                    ) : null}
                  </div>
                  <div role="gridcell" className="truncate text-12 text-fg-muted">
                    {text.author}
                  </div>
                  <div role="gridcell" className="truncate text-12 text-fg-muted">
                    {text.time}
                  </div>
                  <div role="gridcell" className="flex min-w-0 flex-wrap items-center gap-1">
                    {text.refs.map((ref) => (
                      <span
                        key={`${ref.kind}:${ref.label}`}
                        className={cn(
                          'max-w-full truncate rounded-full border px-1.5 text-10',
                          refChipClass(ref.kind),
                        )}
                        title={ref.label}
                      >
                        {ref.label}
                      </span>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
