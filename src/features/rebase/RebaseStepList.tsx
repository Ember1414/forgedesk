/**
 * Rebase 步骤清单：拖拽排序 + 动作下拉 + 键盘替代方案（Alt+↑/↓）。
 *
 * 拖拽用原生 HTML5 DnD——不引第三方拖拽库（供应链与体积）；落点指示由
 * dragOver 时的鼠标半区决定（该行上半 → 插到它前面，下半 → 插到它后面），
 * 落点渲染成一条 2px 的插入线。
 *
 * 键盘是**一等路径**而不是降级：桌面 Git 工具的高频操作在键盘上完成，
 * Alt+↑/↓ 与拖拽共用同一个 `moveEntry`（`planState.ts`），两条路径不会
 * 出现"拖拽对了但键盘错位"的偏差；移动后焦点跟随被移动的行。
 *
 * 非法动作在菜单里即时禁用并给出人话原因（规则短名来自后端 `PlanError`，
 * 文案走 i18n）；后端仍会兜底校验——UI 只是更早说清楚。
 */
import { useRef, useState } from 'react';

import { GripVertical } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { absoluteTime } from '@/features/history/commitMeta';
import type { ReorderAction } from '@/lib/ipc';
import { cn } from '@/lib/utils';
import { Button } from '@/ui/components/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/ui/components/dropdown-menu';
import { Input } from '@/ui/components/input';

import {
  computeIssues,
  isMergeEntry,
  moveEntry,
  setAction,
  type EntryIssue,
  type PlanEntry,
} from './planState';

/** 菜单顺序：最常用在前，危险动作（drop）在最后。 */
const ACTIONS: readonly ReorderAction[] = ['pick', 'reword', 'edit', 'squash', 'fixup', 'drop'];

export interface RebaseStepListProps {
  readonly entries: readonly PlanEntry[];
  readonly issues: readonly EntryIssue[];
  /** 执行中（或加载中）时禁止编辑。 */
  readonly disabled: boolean;
  readonly onChange: (entries: readonly PlanEntry[]) => void;
}

/**
 * 若把某行换成指定动作会触发的问题（返回规则短名；合法返回 null）。
 *
 * 用「试换 + 全局校验」而不是手写一遍规则：规则只有一处（`computeIssues`），
 * 菜单的禁用与底部的整体校验永远一致。
 */
function blockedRule(
  entries: readonly PlanEntry[],
  index: number,
  action: ReorderAction,
): string | null {
  const probe = setAction(entries, index, action);
  const hit = computeIssues(probe).find((issue) => issue.index === index);
  return hit?.rule ?? null;
}

/** 可拖拽的步骤清单。 */
export function RebaseStepList({ entries, issues, disabled, onChange }: RebaseStepListProps) {
  const { t } = useTranslation('shell');
  const [drag, setDrag] = useState<{ from: number; over: number; after: boolean } | null>(null);
  const itemRefs = useRef<(HTMLLIElement | null)[]>([]);

  const applyMove = (from: number, to: number) => {
    if (from === to) {
      return;
    }
    onChange(moveEntry(entries, from, to));
    // 键盘路径的焦点跟随：渲染后再聚焦新位置（拖拽路径没有焦点可跟，无害）
    window.setTimeout(() => itemRefs.current[to]?.focus(), 0);
  };

  const finishDrop = () => {
    if (drag === null) {
      return;
    }
    // 落点是"插入槽位"（0..n）：拖动行从数组中移除后，其后的槽位整体左移一格
    const rawSlot = drag.after ? drag.over + 1 : drag.over;
    const destination = drag.from < rawSlot ? rawSlot - 1 : rawSlot;
    applyMove(drag.from, destination);
    setDrag(null);
  };

  const issueByIndex = new Map(issues.map((issue) => [issue.index, issue.rule]));

  return (
    <ul className="flex flex-col" data-testid="rebase-step-list">
      {entries.map((entry, index) => {
        const issue = issueByIndex.get(index) ?? null;
        const dropBefore = drag !== null && drag.over === index && !drag.after;
        const dropAfter = drag !== null && drag.over === index && drag.after;
        const editingMessage = entry.action === 'reword' || entry.action === 'squash';
        return (
          <li
            key={entry.oid}
            ref={(node) => {
              itemRefs.current[index] = node;
            }}
            tabIndex={0}
            aria-label={entry.subject}
            className={cn(
              'relative outline-none',
              'focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-focus-ring',
              drag?.from === index && 'opacity-50',
            )}
            draggable={!disabled}
            onKeyDown={(event) => {
              if (!event.altKey || disabled) {
                return;
              }
              if (event.key === 'ArrowUp' && index > 0) {
                event.preventDefault();
                applyMove(index, index - 1);
              }
              if (event.key === 'ArrowDown' && index < entries.length - 1) {
                event.preventDefault();
                applyMove(index, index + 1);
              }
            }}
            onDragStart={(event) => {
              event.dataTransfer.effectAllowed = 'move';
              setDrag({ from: index, over: index, after: false });
            }}
            onDragOver={(event) => {
              if (disabled) {
                return;
              }
              event.preventDefault();
              const rect = event.currentTarget.getBoundingClientRect();
              const after = event.clientY > rect.top + rect.height / 2;
              setDrag((current) => ({
                from: current?.from ?? index,
                over: index,
                after,
              }));
            }}
            onDrop={(event) => {
              event.preventDefault();
              finishDrop();
            }}
            onDragEnd={() => {
              setDrag(null);
            }}
          >
            {dropBefore ? (
              <span
                aria-hidden
                className="absolute inset-x-0 top-0 z-10 h-0.5 bg-brand"
                data-testid={`rebase-drop-before-${index}`}
              />
            ) : null}
            <div className="flex items-center gap-2 border-b border-line px-2 py-1.5">
              <GripVertical aria-hidden className="size-3.5 shrink-0 text-fg-subtle" />
              <span className="w-16 shrink-0 font-mono text-12 text-fg-subtle" title={entry.oid}>
                {entry.oid.slice(0, 7)}
              </span>
              <span className="min-w-0 flex-1 truncate text-13" title={entry.subject}>
                {entry.subject}
              </span>
              {isMergeEntry(entry) ? (
                <span className="shrink-0 rounded-sm bg-surface-sunken px-1 text-11 text-fg-muted">
                  {t('history.rebase.mergeBadge')}
                </span>
              ) : null}
              <span className="shrink-0 text-12 text-fg-muted">
                {entry.author} · {absoluteTime(entry.authorTime) ?? ''}
              </span>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={disabled}
                    data-testid={`rebase-action-${entry.oid}`}
                  >
                    {t(`history.rebase.action.${entry.action}`)}
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuLabel>{t('history.rebase.actionLabel')}</DropdownMenuLabel>
                  <DropdownMenuSeparator />
                  {ACTIONS.map((action) => {
                    const blocked = blockedRule(entries, index, action);
                    return (
                      <DropdownMenuItem
                        key={action}
                        disabled={blocked !== null}
                        onSelect={() => {
                          onChange(setAction(entries, index, action));
                        }}
                        data-testid={`rebase-action-option-${action}`}
                      >
                        <span className="flex flex-col">
                          <span>{t(`history.rebase.action.${action}`)}</span>
                          {blocked === null ? null : (
                            <span className="text-11 text-fg-subtle">
                              {t(`history.rebase.reason.${blocked}`)}
                            </span>
                          )}
                        </span>
                      </DropdownMenuItem>
                    );
                  })}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
            {editingMessage ? (
              <div className="px-2 pb-2 pl-8">
                <Input
                  label={t('history.rebase.newMessage')}
                  placeholder={entry.subject}
                  value={entry.newMessage ?? ''}
                  disabled={disabled}
                  onChange={(event) => {
                    onChange(setAction(entries, index, entry.action, event.target.value));
                  }}
                  data-testid={`rebase-message-${entry.oid}`}
                />
              </div>
            ) : null}
            {issue === null ? null : (
              <p
                className="px-2 pb-1.5 pl-8 text-12 text-danger"
                data-testid={`rebase-issue-${entry.oid}`}
              >
                {t(`history.rebase.reason.${issue}`)}
              </p>
            )}
            {dropAfter ? (
              <span
                aria-hidden
                className="absolute inset-x-0 bottom-0 z-10 h-0.5 bg-brand"
                data-testid={`rebase-drop-after-${index}`}
              />
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}
