/**
 * 快捷键设置页（T5.9）：搜索、分类分组、冲突高亮、自定义、恢复默认。
 *
 * 自定义写 `shortcuts.overrides.<id>`（JSON 字符串的规范化键）；
 * 写入前用 detectConflicts 检查——同 when 上下文重复键**阻止保存**。
 * "导出 Markdown"复制到剪贴板（帮助页入口在本页顶部）。
 */
import { useMemo, useState } from 'react';

import { useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { detectConflicts, displayShortcut } from '@/lib/shortcutKeys';
import { SHORTCUT_OVERRIDE_PREFIX, useCommandRegistry } from '@/features/commands/commandRegistry';
import { useSettingsStore } from '@/stores/settingsStore';
import { Button } from '@/ui/components/button';
import { Input } from '@/ui/components/input';
import { SelectField } from '@/ui/components/select';
import { cn } from '@/lib/utils';

/** 分类过滤选项。 */
const CATEGORIES = ['all', 'navigate', 'git', 'editor', 'view', 'app'] as const;

export function ShortcutsSettingsPage() {
  const { t } = useTranslation('shell');
  const queryClient = useQueryClient();
  const commands = useCommandRegistry((state) => state.commands);
  const values = useSettingsStore((state) => state.values);
  const setJson = useSettingsStore((state) => state.setJson);
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('all');
  const [recording, setRecording] = useState<string | null>(null);
  const [blocked, setBlocked] = useState<string | null>(null);

  // 生效键（默认 + 覆盖）
  const effective = useMemo(
    () =>
      commands.map((command) => {
        const raw = values[`${SHORTCUT_OVERRIDE_PREFIX}${command.id}`];
        let normalized: string | null = command.defaultKey;
        if (raw !== undefined) {
          try {
            const parsed: unknown = JSON.parse(raw);
            normalized = typeof parsed === 'string' ? parsed : command.defaultKey;
          } catch {
            normalized = command.defaultKey;
          }
        }
        return { command, normalized };
      }),
    [commands, values],
  );

  const conflicts = useMemo(
    () =>
      detectConflicts(
        effective.map(({ command, normalized }) => ({
          id: command.id,
          normalizedKey: normalized,
          when: command.when,
        })),
      ),
    [effective],
  );
  const conflictIds = new Set(conflicts.flatMap((c) => [c.firstId, c.secondId]));

  const filtered = effective.filter(({ command }) => {
    if (category !== 'all' && command.category !== category) {
      return false;
    }
    const needle = query.trim().toLowerCase();
    if (needle === '') {
      return true;
    }
    return (
      t(command.titleKey).toLowerCase().includes(needle) ||
      command.id.toLowerCase().includes(needle)
    );
  });

  const startRecording = (id: string) => {
    setRecording(id);
    setBlocked(null);
    const handler = (event: KeyboardEvent): void => {
      event.preventDefault();
      event.stopPropagation();
      window.removeEventListener('keydown', handler, true);
      setRecording(null);
      if (event.key === 'Escape') {
        return;
      }
      const parts: string[] = [];
      if (event.ctrlKey || event.metaKey) {
        parts.push('Mod');
      }
      if (event.shiftKey) {
        parts.push('Shift');
      }
      if (event.altKey) {
        parts.push('Alt');
      }
      parts.push(event.key);
      const candidate = parts.join('+');
      // 冲突检测：候选键与其它命令冲突 → 阻止保存
      const candidateConflicts = detectConflicts(
        effective.map(({ command, normalized }) => ({
          id: command.id,
          normalizedKey: command.id === id ? candidate : normalized,
          when: command.when,
        })),
      ).filter((conflict) => conflict.firstId === id || conflict.secondId === id);
      if (candidateConflicts.length > 0) {
        setBlocked(
          t('settings.shortcuts.conflictBlocked', { other: candidateConflicts[0]?.secondId ?? '' }),
        );
        return;
      }
      void setJson(`${SHORTCUT_OVERRIDE_PREFIX}${id}`, candidate).then(() => {
        void queryClient.invalidateQueries({ queryKey: ['shortcuts'] });
      });
    };
    window.addEventListener('keydown', handler, true);
  };

  const reset = (id: string) => {
    void useSettingsStore.getState().setJson(`${SHORTCUT_OVERRIDE_PREFIX}${id}`, null);
  };

  const exportMarkdown = () => {
    const lines = [
      `| ${t('settings.shortcuts.keyColumn')} | ${t('settings.shortcuts.actionColumn')} |`,
      '| --- | --- |',
    ];
    for (const { command, normalized } of effective) {
      lines.push(
        `| ${normalized === null ? '—' : displayShortcut(normalized)} | ${t(command.titleKey)} |`,
      );
    }
    void navigator.clipboard.writeText(lines.join('\n')).then(() => {});
  };

  return (
    <div className="flex max-w-2xl flex-col gap-4">
      <h1 className="text-20 font-semibold tracking-tight">{t('pages.settingsShortcuts.title')}</h1>
      <div className="flex items-center gap-2">
        <Input
          srLabel={t('settings.shortcuts.search')}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t('settings.shortcuts.search')}
          className="w-52"
        />
        <SelectField
          label={t('settings.shortcuts.categoryFilter')}
          value={category}
          onValueChange={setCategory}
          options={CATEGORIES.map((name) => ({
            value: name,
            label:
              name === 'all'
                ? t('settings.shortcuts.allCategories')
                : t(`settings.shortcuts.category.${name}`),
          }))}
          className="w-40"
        />
        <Button size="sm" variant="secondary" onClick={exportMarkdown}>
          {t('settings.shortcuts.exportMarkdown')}
        </Button>
      </div>

      {blocked !== null ? <p className="text-danger text-12">{blocked}</p> : null}
      {recording !== null ? (
        <p className="text-warning text-12">{t('settings.shortcuts.recording')}</p>
      ) : null}

      <ul className="flex flex-col gap-1">
        {filtered.map(({ command, normalized }) => {
          const isConflict = conflictIds.has(command.id);
          return (
            <li
              key={command.id}
              className={cn(
                'border-line bg-surface flex items-center gap-2 rounded-md border px-2.5 py-1.5',
                isConflict && 'border-warning',
              )}
            >
              <span className="min-w-0 flex-1 truncate text-13">{t(command.titleKey)}</span>
              {isConflict ? (
                <span className="text-warning text-11">{t('settings.shortcuts.conflictMark')}</span>
              ) : null}
              {recording === command.id ? (
                <span className="text-warning text-12">…</span>
              ) : (
                <kbd
                  className={cn(
                    'border-line bg-surface-sunken text-fg-muted cursor-pointer rounded border px-1.5 py-0.5 font-mono text-11',
                  )}
                  onClick={() => startRecording(command.id)}
                  title={t('settings.shortcuts.clickToRecord')}
                >
                  {normalized === null ? '—' : displayShortcut(normalized)}
                </kbd>
              )}
              <Button
                size="sm"
                variant="ghost"
                onClick={() => reset(command.id)}
                disabled={values[`${SHORTCUT_OVERRIDE_PREFIX}${command.id}`] === undefined}
              >
                {t('settings.shortcuts.reset')}
              </Button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
