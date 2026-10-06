/**
 * 快捷键管理器（T5.9）：注册命令、全局 keydown 分发、when 上下文维护。
 *
 * 渲染为 null（面板打开时渲染 CommandPalette）；挂在 AppShell（单实例）。
 * 自定义键从 settingsStore 读取（`shortcuts.overrides.<id>`）；
 * 冲突检测的"阻止保存"在设置页（写入口）执行。
 */
import { useEffect, useMemo } from 'react';

import { CommandPalette } from '@/features/commands/CommandPalette';
import { useCommandDefinitions } from '@/features/commands/commandDefs';
import { useCommandRegistry } from '@/features/commands/commandRegistry';
import { matchesEvent, normalizeShortcut } from '@/lib/shortcutKeys';
import { useSettingsStore } from '@/stores/settingsStore';
import { useUiStore } from '@/stores/uiStore';

export interface ShortcutManagerProps {
  /** 当前是否在编辑器页（editorActive when 标签的来源）。 */
  readonly editorActive: boolean;
}

export function ShortcutManagerWithPalette({ editorActive }: ShortcutManagerProps) {
  const definitions = useCommandDefinitions();
  const register = useCommandRegistry((state) => state.register);
  const setContextTags = useCommandRegistry((state) => state.setContextTags);
  const paletteOpen = useCommandRegistry((state) => state.paletteOpen);
  const currentRepoId = useUiStore((state) => state.currentRepoId);
  const values = useSettingsStore((state) => state.values);

  useEffect(() => {
    register(definitions);
  }, [definitions, register]);

  useEffect(() => {
    const tags = new Set<string>();
    if (currentRepoId !== null) {
      tags.add('repoOpen');
    }
    if (editorActive) {
      tags.add('editorActive');
    }
    setContextTags(tags);
  }, [currentRepoId, editorActive, setContextTags]);

  // 自定义键覆盖（键 → 规范化后的生效键）
  const overrides = useMemo(
    () =>
      Object.fromEntries(
        Object.entries(values)
          .filter(([key]) => key.startsWith('shortcuts.overrides.'))
          .map(([key, value]) => [key.slice('shortcuts.overrides.'.length), value]),
      ),
    [values],
  );

  // 全局 keydown 分发（capture：先于页面内快捷键；输入框聚焦时跳过）
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (
        paletteOpen ||
        (event.target instanceof HTMLElement &&
          (event.target.tagName === 'INPUT' ||
            event.target.tagName === 'TEXTAREA' ||
            event.target.isContentEditable))
      ) {
        return;
      }
      const effective = new Map<string, string>();
      for (const command of definitions) {
        const raw = overrides[command.id];
        const fallback = command.defaultKey;
        effective.set(
          command.id,
          raw !== undefined ? normalizeShortcut(raw) : fallback === null ? '' : normalizeShortcut(fallback),
        );
      }
      for (const [id, normalizedKey] of effective) {
        if (normalizedKey === null || normalizedKey === '' || !matchesEvent(normalizedKey, event)) {
          continue;
        }
        const command = useCommandRegistry.getState().commands.find((c) => c.id === id);
        if (!command) {
          continue;
        }
        const tags = useCommandRegistry.getState().contextTags;
        if (!command.when.every((tag) => tags.has(tag))) {
          continue;
        }
        event.preventDefault();
        event.stopPropagation();
        command.run();
        return;
      }
    };
    window.addEventListener('keydown', onKeyDown, { capture: true });
    return () => {
      window.removeEventListener('keydown', onKeyDown, { capture: true });
    };
  }, [definitions, overrides, paletteOpen]);

  if (!paletteOpen) {
    return null;
  }

  return <CommandPalette overrides={overrides} contextTags={useCommandRegistry.getState().contextTags} />;
}
