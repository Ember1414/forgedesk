/**
 * 提交图的交互层（T2.2）。
 *
 * # 职责边界
 *
 * `GraphCanvas` 是**表现层**：它只画、只做命中检测，所有文案与动作都从 props 注入，
 * 因此它既不认识 i18n，也不认识"选中意味着什么"。本组件是它唯一的容器，负责三件事：
 *   1. hover 摘要卡片（贴在节点右侧，而不是跟着指针跑）；
 *   2. 右键菜单（复制 / 比较基准 / 一批要到 T2.8 才接通的写操作）；
 *   3. 选中语义（单击、Ctrl/Cmd 多选、Shift 区间选）落到 `graphSelectionStore`。
 *
 * "hover 时高亮同分支链路"不在这里画：`GraphCanvas` 自己订阅 store 的 `hoverOid`，
 * 用 `buildLaneAdjacency` + `walkLaneChain` 的结果在**动态层**描高亮。走 store 而
 * 不是 props，是因为 hover 是每秒几十次的高频状态——走 React 会让整棵子树重渲染，
 * 而动态层的重绘本来就只需要一次 rAF。
 *
 * # 为什么右键的"空目标"判定读 store，而不是读 hover 卡片的 state
 *
 * 鼠标右键前面必有一次 `pointermove`，两者是**独立的事件派发**，React 已经落地了
 * 那次 setState，所以卡片 state 通常是新的。但键盘的菜单键（以及辅助技术的
 * "打开上下文菜单"）不会先移动指针，此时 `GraphCanvas.handleContextMenu` 才补做
 * 命中检测并上报——那次 setState 与 Radix 的开启逻辑在**同一次**派发里，
 * 批处理还没落地，读 state 拿到的是旧值。store 的写入是同步的，所以这里读 store。
 *
 * 抑制菜单的手段是在 wrapper 的 `onContextMenu` 里 `preventDefault()`：
 * `ContextMenuTrigger asChild` 走 Radix 的 Slot，而 Slot 的 `mergeProps`
 * **先调子元素自己的 handler、再调 Trigger 传下来的**；Trigger 内部又用
 * `composeEventHandlers`（默认 `checkForDefaultPrevented: true`）包着开启逻辑，
 * 于是 `preventDefault()` 恰好能否决它。这是 Radix 公开的组合语义，不是取巧。
 *
 * # 为什么 hover 卡片没有复用 `ui/components/tooltip.tsx`
 *
 * Radix Tooltip 只接受**真实元素**作触发器；我们的触发目标是 canvas 上的一个坐标，
 * 需要虚拟锚点，而 `tooltip.tsx` 没有导出 `Anchor`，该文件也不在本任务的修改范围内。
 * 因此这里渲染一个与 `TooltipContent` 同样式的浮层，并标 `aria-hidden`：
 * 同样的信息在文本列与列表模式里都可读，卡片只是给鼠标用户的加速，
 * 不能成为任何信息的唯一来源（这也是 tooltip 组件自己的使用约定）。
 */
import { useCallback, useMemo, useState } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';

import {
  Copy,
  GitBranch,
  GitCompare,
  GitMerge,
  History,
  MessageSquare,
  Pencil,
  PenLine,
  RotateCcw,
  Tag,
  Trash2,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import type { ReorderAction } from '@/lib/ipc';
import { cn } from '@/lib/utils';

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from '@/ui/components/context-menu';

import { absoluteTime, fullMessage, parseRefs, shortOid } from '@/features/history/commitMeta';
import type { RowText } from '@/features/history/commitMeta';
import type { RefLabel } from '@/features/history/graphGeometry';
import { GraphCanvas } from '@/features/history/GraphCanvas';
import type { HoverTarget } from '@/features/history/GraphCanvas';
import { useGraphSelectionStore } from '@/features/history/graphSelectionStore';
import type { SelectionModifiers } from '@/features/history/graphSelectionStore';
import { refChipClass } from '@/features/history/graphTheme';
import type { GraphModel } from '@/features/history/useGraphQuery';

/** hover 卡片的宽度上限。用 `min()` 与父容器宽度取小：靠右的节点不会被截断。 */
const HOVER_CARD_MAX_WIDTH = 'min(288px, 100%)';

/** 菜单项图标的统一尺寸与配色（与 `context-menu.tsx` 的勾选图标一致）。 */
const MENU_ICON_CLASS = 'size-3.5 shrink-0 text-fg-subtle';

/** 空 ref 列表（模块级常量：卡片每次 hover 都要一个数组，不该每次新建）。 */
const NO_REFS: readonly RefLabel[] = [];

export interface GraphOverlayProps {
  readonly model: GraphModel;
  /** 与 `model.rows` 同序同长的 DOM 文案（由 `HistoryPage` 注入 i18n 后算好）。 */
  readonly texts: readonly RowText[];
  /** 搜索命中的提交（T2.3：琥珀点线环；透传给画布动态层）。 */
  readonly matchOids?: ReadonlySet<string> | undefined;
  /** 滚动跳转请求（T2.3 的"上一处 / 下一处"；透传给画布）。 */
  readonly scrollToRow?: { readonly row: number; readonly token: number } | null | undefined;
  /** 滚到底部附近时请求下一页。 */
  readonly onNeedMore: () => void;
  readonly className?: string;
}

/**
 * hover 摘要卡片的内容。
 *
 * 单独一个模块级组件而不是内联 JSX：`react-hooks/static-components` 禁止在渲染里
 * 定义组件，而在父组件的 JSX 里摊开三十行卡片会让右键菜单那部分难以阅读。
 */
function HoverCardBody({
  subject,
  author,
  time,
  oid,
  refs,
}: {
  readonly subject: string;
  readonly author: string;
  readonly time: string;
  readonly oid: string;
  readonly refs: readonly RefLabel[];
}) {
  return (
    <>
      <p className="line-clamp-2 text-13 leading-snug font-medium">{subject}</p>
      <p className="mt-1 flex min-w-0 items-baseline gap-1.5 text-12 text-fg-muted">
        <span className="truncate">{author}</span>
        <span aria-hidden="true" className="shrink-0 text-fg-subtle">
          ·
        </span>
        <span className="shrink-0">{time}</span>
      </p>
      {refs.length > 0 ? (
        <p className="mt-1.5 flex flex-wrap gap-1">
          {refs.map((entry) => (
            <span
              key={`${entry.kind}:${entry.label}`}
              className={cn('shrink-0 rounded-xs border px-1 text-12', refChipClass(entry.kind))}
            >
              {entry.label}
            </span>
          ))}
        </p>
      ) : null}
      <p className="mt-1.5 font-mono text-12 text-fg-subtle">{oid}</p>
    </>
  );
}

export function GraphOverlay({
  model,
  texts,
  matchOids,
  scrollToRow,
  onNeedMore,
  className,
}: GraphOverlayProps) {
  const { t } = useTranslation('shell');
  const { show } = useAppError();

  const select = useGraphSelectionStore((state) => state.select);
  const selectMany = useGraphSelectionStore((state) => state.selectMany);
  const setCompareBase = useGraphSelectionStore((state) => state.setCompareBase);
  const compareBaseOid = useGraphSelectionStore((state) => state.compareBaseOid);
  const selectedOids = useGraphSelectionStore((state) => state.selectedOids);
  const selectedCount = selectedOids.length;
  const requestRebase = useGraphSelectionStore((state) => state.requestRebase);

  const [hoverTarget, setHoverTarget] = useState<HoverTarget | null>(null);

  /**
   * 行序的 oid 列表。
   *
   * store 需要它来把"点击顺序"归一化成"行顺序"，以及算 Shift 区间；
   * 每次 `model.rows` 变化才重算，滚动与 hover 都不会碰它。
   */
  const order = useMemo(() => model.rows.map((row) => row.oid), [model.rows]);

  const handleActivate = useCallback(
    (oid: string, modifiers: SelectionModifiers) => {
      select(oid, modifiers, order);
    },
    [order, select],
  );

  const handleSelectAll = useCallback(() => {
    selectMany(order, order);
  }, [order, selectMany]);

  const formatCollapsed = useCallback((count: number) => t('history.collapsed', { count }), [t]);

  const rowLabel = useCallback(
    (text: RowText) =>
      t('history.rowLabel', {
        subject: text.subject,
        author: text.author,
        time: text.time,
        oid: text.shortOid,
      }),
    [t],
  );

  // ---------------------------------------------------------------- 右键目标

  const menuOid = hoverTarget?.oid ?? null;
  const menuCommit = menuOid === null ? undefined : model.commitByOid.get(menuOid);

  /**
   * 空白处右键不出菜单。
   *
   * 读 store 而不是 `hoverTarget`：见文件头"为什么右键的空目标判定读 store"。
   */
  const handleContextMenu = useCallback((event: ReactMouseEvent<HTMLDivElement>) => {
    if (useGraphSelectionStore.getState().hoverOid === null) {
      event.preventDefault();
    }
  }, []);

  const handleCopyOid = useCallback(() => {
    if (menuOid === null) {
      return;
    }
    // 与仓库既有写法一致：剪贴板可能因为窗口失焦而 reject，交给全局错误提示。
    void navigator.clipboard.writeText(menuOid).catch(show);
  }, [menuOid, show]);

  const handleCopyMessage = useCallback(() => {
    if (menuCommit === undefined) {
      return;
    }
    void navigator.clipboard.writeText(fullMessage(menuCommit)).catch(show);
  }, [menuCommit, show]);

  const handleToggleCompareBase = useCallback(() => {
    if (menuOid === null) {
      return;
    }
    // 再次点击同一个提交就取消：比较基准是"一次性设定"的东西，
    // 留着旧的会让 T2.4 的差异视图悄悄比错对象。
    setCompareBase(compareBaseOid === menuOid ? null : menuOid);
  }, [compareBaseOid, menuOid, setCompareBase]);

  /**
   * 本次"整理提交"的操作对象：右键的那条在选中集里 → 用全部选中（框选一批
   * 再右键是主路径）；不在 → 只针对它自己（用户在未选中的行上右键）。
   */
  const menuSelection = useMemo<readonly string[]>(() => {
    if (menuOid === null) {
      return [];
    }
    return selectedOids.includes(menuOid) ? selectedOids : [menuOid];
  }, [menuOid, selectedOids]);

  const handleOrganize = useCallback(() => {
    if (menuSelection.length === 0) {
      return;
    }
    requestRebase({ oids: menuSelection });
  }, [menuSelection, requestRebase]);

  const handleRebaseAction = useCallback(
    (action: ReorderAction) => {
      if (menuOid === null) {
        return;
      }
      requestRebase({ oids: [menuOid], preset: { oid: menuOid, action } });
    },
    [menuOid, requestRebase],
  );

  // ---------------------------------------------------------------- 渲染

  const canvasLabel = t('history.canvasLabel', {
    rows: model.rowCount,
    lanes: model.laneCount,
  });

  const cardCommit = hoverTarget === null ? undefined : model.commitByOid.get(hoverTarget.oid);
  const cardTime = cardCommit === undefined ? null : absoluteTime(cardCommit.author.time);
  /**
   * ref 芯片优先用预算好的行文案（`buildRowTexts` 已经解析过一遍），
   * 只有分页边界上行文案还没到时才现场解析——解析是纯词法判断，很便宜，
   * 但重复做没有意义。
   */
  const cardRefs =
    hoverTarget?.text?.refs ?? (cardCommit === undefined ? NO_REFS : parseRefs(cardCommit.refs));

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          data-testid="graph-overlay"
          className={cn('relative flex min-h-0 min-w-0 flex-1', className)}
          onContextMenu={handleContextMenu}
        >
          <GraphCanvas
            className="h-full"
            model={model}
            texts={texts}
            listLabel={t('history.listLabel')}
            canvasLabel={canvasLabel}
            minimapLabel={t('history.minimapLabel')}
            formatCollapsed={formatCollapsed}
            rowLabel={rowLabel}
            onActivate={handleActivate}
            onSelectAll={handleSelectAll}
            onHoverTarget={setHoverTarget}
            onNeedMore={onNeedMore}
            matchOids={matchOids}
            scrollToRow={scrollToRow ?? null}
          />

          {hoverTarget === null ? null : (
            /**
             * 外层用 `left` + `right: 0` 而不是给卡片自己写 `max-width: calc(...)`：
             * 后者在节点靠右时会算出负数，而负的 `max-width` 是非法值，
             * 浏览器会整条丢弃声明，卡片反而溢出容器。用"从 x 铺到右边缘"的
             * 定位容器，宽度天然非负，卡片再按 `min(288px, 100%)` 收缩即可。
             */
            <div
              aria-hidden="true"
              data-testid="graph-hover-card"
              className="pointer-events-none absolute inset-y-0 overflow-hidden"
              style={{ left: hoverTarget.x, right: 0 }}
            >
              <div
                className={cn(
                  'absolute w-max -translate-y-1/2 rounded-md border border-line',
                  'bg-surface-raised px-2 py-1 text-12 text-fg shadow-md',
                )}
                style={{ top: hoverTarget.y, maxWidth: HOVER_CARD_MAX_WIDTH }}
              >
                <HoverCardBody
                  subject={cardCommit?.subject ?? hoverTarget.text?.subject ?? t('history.pending')}
                  author={
                    hoverTarget.text?.author ??
                    (cardCommit === undefined ? '' : cardCommit.author.name)
                  }
                  time={cardTime ?? hoverTarget.text?.time ?? t('history.pending')}
                  oid={shortOid(hoverTarget.oid)}
                  refs={cardRefs}
                />
              </div>
            </div>
          )}
        </div>
      </ContextMenuTrigger>

      <ContextMenuContent data-testid="graph-context-menu">
        <ContextMenuLabel className="flex min-w-0 items-baseline gap-1.5">
          <span className="font-mono">{menuOid === null ? '' : shortOid(menuOid)}</span>
          <span className="truncate font-normal text-fg-muted">
            {menuCommit?.subject ?? t('history.pending')}
          </span>
        </ContextMenuLabel>
        {selectedCount > 1 ? (
          <ContextMenuLabel className="font-normal">
            {t('history.menu.selectedCount', { count: selectedCount })}
          </ContextMenuLabel>
        ) : null}
        <ContextMenuSeparator />

        <ContextMenuItem data-testid="graph-menu-copy-oid" onSelect={handleCopyOid}>
          <Copy aria-hidden="true" className={MENU_ICON_CLASS} />
          {t('history.menu.copyOid')}
        </ContextMenuItem>
        <ContextMenuItem data-testid="graph-menu-copy-message" onSelect={handleCopyMessage}>
          <MessageSquare aria-hidden="true" className={MENU_ICON_CLASS} />
          {t('history.menu.copyMessage')}
        </ContextMenuItem>
        <ContextMenuItem data-testid="graph-menu-compare-base" onSelect={handleToggleCompareBase}>
          <GitCompare aria-hidden="true" className={MENU_ICON_CLASS} />
          {compareBaseOid !== null && compareBaseOid === menuOid
            ? t('history.menu.compareBaseClear')
            : t('history.menu.compareBaseSet')}
        </ContextMenuItem>

        <ContextMenuSeparator />

        {/**
         * 整理提交（T3.6）：框选一组 → 打开拖拽面板；单个提交 → 三个直达动作。
         * 单提交动作直接以 preset 打开面板（用户仍能看到预览与确认），不在
         * 菜单里就地执行——破坏性操作必须经过"预览 → 快照 → 执行"（R7）。
         */}
        <ContextMenuItem
          data-testid="graph-menu-organize"
          disabled={menuSelection.length === 0}
          onSelect={handleOrganize}
        >
          <History aria-hidden="true" className={MENU_ICON_CLASS} />
          {t('history.menu.organizeCommits')}
        </ContextMenuItem>
        {menuSelection.length === 1 ? (
          <>
            <ContextMenuItem
              data-testid="graph-menu-reword-commit"
              onSelect={() => {
                handleRebaseAction('reword');
              }}
            >
              <PenLine aria-hidden="true" className={MENU_ICON_CLASS} />
              {t('history.menu.rewordCommit')}
            </ContextMenuItem>
            <ContextMenuItem
              data-testid="graph-menu-edit-commit"
              onSelect={() => {
                handleRebaseAction('edit');
              }}
            >
              <Pencil aria-hidden="true" className={MENU_ICON_CLASS} />
              {t('history.menu.editCommit')}
            </ContextMenuItem>
            <ContextMenuItem
              destructive
              data-testid="graph-menu-drop-commit"
              onSelect={() => {
                handleRebaseAction('drop');
              }}
            >
              <Trash2 aria-hidden="true" className={MENU_ICON_CLASS} />
              {t('history.menu.dropCommit')}
            </ContextMenuItem>
          </>
        ) : null}

        <ContextMenuSeparator />

        {/**
         * 以下五项都是**写操作**，要到 T2.8（分支与标签管理 / 提交动作）才有对应的
         * IPC 命令。这里保留禁用态而不是先不画：用户能看见"这个功能是有的、
         * 只是还没接通"，比菜单里凭空少几项更容易理解，也让 T2.8 的接线点
         * 在代码里是显式的（把 `disabled` 去掉、补上 onSelect 即可）。
         */}
        <ContextMenuItem disabled data-testid="graph-menu-create-tag">
          <Tag aria-hidden="true" className={MENU_ICON_CLASS} />
          {t('history.menu.createTag')}
        </ContextMenuItem>
        <ContextMenuItem disabled data-testid="graph-menu-create-branch">
          <GitBranch aria-hidden="true" className={MENU_ICON_CLASS} />
          {t('history.menu.createBranch')}
        </ContextMenuItem>
        <ContextMenuItem disabled data-testid="graph-menu-cherry-pick">
          <GitMerge aria-hidden="true" className={MENU_ICON_CLASS} />
          {t('history.menu.cherryPick')}
        </ContextMenuItem>
        <ContextMenuItem disabled data-testid="graph-menu-revert">
          <RotateCcw aria-hidden="true" className={MENU_ICON_CLASS} />
          {t('history.menu.revert')}
        </ContextMenuItem>
        <ContextMenuItem disabled destructive data-testid="graph-menu-reset">
          <RotateCcw aria-hidden="true" className={MENU_ICON_CLASS} />
          {t('history.menu.reset')}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
