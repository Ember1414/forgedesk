/**
 * 命令面板（T5.9）：Mod+Shift+P 打开，模糊搜索、键盘全可用。
 *
 * - 搜索命中 title（i18n 文案）与 id；按"最近使用优先 + 字母序"排序；
 * - when 不满足的命令灰显并说明原因（i18n: commands.when.<tag>）；
 * - ↑↓ 选择、Enter 执行并关闭、Esc 关闭；
 * - 执行后记入"最近使用"。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useTranslation } from 'react-i18next';

import {
  readRecentCommands,
  recordRecentCommand,
  SHORTCUT_OVERRIDE_PREFIX,
  useCommandRegistry,
} from '@/features/commands/commandRegistry';
import { displayShortcut } from '@/lib/shortcutKeys';
import { Dialog, DialogContent, DialogTitle } from '@/ui/components/dialog';
import { cn } from '@/lib/utils';

export interface CommandPaletteProps {
  readonly overrides: Readonly<Record<string, string>>;
  /** when 求值上下文（AppShell 维护）。 */
  readonly contextTags: ReadonlySet<string>;
}

function overrideKeyOf(values: Readonly<Record<string, string>>, id: string): string | null {
  const raw = values[`${SHORTCUT_OVERRIDE_PREFIX}${id}`];
  if (raw === undefined) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    return typeof parsed === 'string' ? parsed : null;
  } catch {
    return null;
  }
}

export function CommandPalette({ overrides, contextTags }: CommandPaletteProps) {
  const { t } = useTranslation('shell');
  const commands = useCommandRegistry((state) => state.commands);
  const setPaletteOpen = useCommandRegistry((state) => state.setPaletteOpen);
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const [queryEpoch, setQueryEpoch] = useState(0);
  const listRef = useRef<HTMLUListElement>(null);

  const flat = useMemo(() => {
    return commands.map((command) => {
      const key = overrideKeyOf(overrides, command.id) ?? command.defaultKey;
      const available = command.when.every((tag) => contextTags.has(tag));
      return { command, key, available };
    });
  }, [commands, contextTags, overrides]);

  const results = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const matched = flat.filter(
      ({ command }) =>
        needle === '' ||
        t(command.titleKey).toLowerCase().includes(needle) ||
        command.id.toLowerCase().includes(needle),
    );
    // 排序：可用在前 → 最近使用优先 → title 字母序
    const recentSet = new Set(readRecentCommands());
    return [...matched].sort((a, b) => {
      const availDiff = Number(b.available) - Number(a.available);
      if (availDiff !== 0) {
        return availDiff;
      }
      const recentDiff = Number(recentSet.has(b.command.id)) - Number(recentSet.has(a.command.id));
      if (recentDiff !== 0) {
        return recentDiff;
      }
      return t(a.command.titleKey).localeCompare(t(b.command.titleKey));
    });
  }, [flat, query, t]);

  useEffect(() => {
    const active = results[activeIndex];
    listRef.current?.children[activeIndex]?.scrollIntoView({ block: 'nearest' });
    void active;
  }, [activeIndex, results]);

  const close = useCallback(() => {
    setPaletteOpen(false);
  }, [setPaletteOpen]);

  const execute = useCallback(
    (index: number) => {
      const hit = results[index];
      if (!hit || !hit.available) {
        return;
      }
      recordRecentCommand(hit.command.id);
      close();
      // 执行放到关闭之后（命令可能导航，避免对话框残留）
      setTimeout(() => hit.command.run(), 0);
    },
    [close, results],
  );

  return (
    <Dialog open onOpenChange={(open) => !open && close()}>
      <DialogContent closeLabel={t('commands.palette.close')} className="max-w-xl p-0">
        <DialogTitle className="sr-only">{t('commands.palette.title')}</DialogTitle>
        <div className="border-line border-b p-2">
          <input
            autoFocus
            key={queryEpoch}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'ArrowDown') {
                event.preventDefault();
                setActiveIndex((index) => Math.min(index + 1, results.length - 1));
              } else if (event.key === 'ArrowUp') {
                event.preventDefault();
                setActiveIndex((index) => Math.max(index - 1, 0));
              } else if (event.key === 'Backspace' && query === '') {
                // 查询清空后再退格：重置输入与选择（useMemo 依赖 query）
                setQueryEpoch((epoch) => epoch + 1);
                setQuery('');
                setActiveIndex(0);
              } else if (event.key === 'Enter') {
                event.preventDefault();
                execute(activeIndex);
              } else if (event.key === 'Escape') {
                close();
              }
            }}
            placeholder={t('commands.palette.placeholder')}
            className="bg-surface w-full rounded-md px-2 py-1.5 text-13 outline-none"
            aria-label={t('commands.palette.placeholder')}
          />
        </div>
        <ul ref={listRef} className="max-h-80 overflow-auto p-1" role="listbox">
          {results.length === 0 ? (
            <li className="text-fg-subtle p-2 text-13">{t('commands.palette.empty')}</li>
          ) : (
            results.map(({ command, key, available }, index) => (
              <li
                key={command.id}
                role="option"
                aria-selected={index === activeIndex}
                aria-disabled={!available}
                className={cn(
                  'fd-transition flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-13',
                  index === activeIndex && 'bg-surface-sunken',
                  !available && 'opacity-50',
                )}
                onClick={() => execute(index)}
                onMouseEnter={() => setActiveIndex(index)}
              >
                <span className="min-w-0 flex-1 truncate">
                  {t(command.titleKey)}
                  {!available ? (
                    <span className="text-fg-subtle ms-1 text-11">
                      ({command.when.map((tag) => t(`commands.when.${tag}`)).join('、')})
                    </span>
                  ) : null}
                </span>
                {key !== null ? (
                  <kbd className="border-line bg-surface text-fg-muted rounded border px-1.5 py-0.5 font-mono text-11">
                    {displayShortcut(key)}
                  </kbd>
                ) : null}
              </li>
            ))
          )}
        </ul>
      </DialogContent>
    </Dialog>
  );
}
