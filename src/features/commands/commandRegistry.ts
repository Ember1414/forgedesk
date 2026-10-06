/**
 * 命令注册表（T5.9）：命令面板与快捷键的单一数据源。
 *
 * # 结构
 *
 * - 命令定义在 `commandDefs.tsx`（React hook 组装 run 闭包——需要导航与
 *   仓库上下文）；本模块持有**运行时注册表**（Zustand，模块级单例）。
 * - 自定义快捷键存 settings 表（`shortcuts.overrides.<id>` → 规范化键），
 *   冲突检测在保存时由 [`detectConflicts`] 完成（调用方阻止保存）。
 * - `when` 求值上下文由 [`setCommandContext`] 维护（AppShell 更新）。
 * - 最近使用记 localStorage（`forgedesk.commands.recent`，上限 10）。
 */
import { create } from 'zustand';

import { normalizeShortcut } from '@/lib/shortcutKeys';

/** 一个已注册的命令（run 由注册方闭包提供）。 */
export interface RegisteredCommand {
  readonly id: string;
  /** i18n key（shell 命名空间下）。 */
  readonly titleKey: string;
  /** 分类（面板分组与设置页）。 */
  readonly category: 'navigate' | 'git' | 'editor' | 'view' | 'app';
  /** 规范化默认键；null = 无快捷键。 */
  readonly defaultKey: string | null;
  /** when 标签集合（空 = 全局）。 */
  readonly when: readonly string[];
  /** 执行。 */
  run(): void;
}

export interface CommandRegistryState {
  readonly commands: readonly RegisteredCommand[];
  /** 命令面板是否打开。 */
  readonly paletteOpen: boolean;
  /** 运行时 when 上下文（如 repoOpen）。 */
  readonly contextTags: ReadonlySet<string>;
  register(commands: readonly RegisteredCommand[]): void;
  setPaletteOpen(open: boolean): void;
  setContextTags(tags: ReadonlySet<string>): void;
}

export const initialCommandRegistryState = {
  commands: [] as readonly RegisteredCommand[],
  paletteOpen: false,
  contextTags: new Set<string>() as ReadonlySet<string>,
};

export const useCommandRegistry = create<CommandRegistryState>()((set) => ({
  ...initialCommandRegistryState,

  register: (commands) => {
    set((state) => {
      // 合并语义：按 id 去重（新的覆盖旧的），保持注册顺序稳定
      const merged = new Map(state.commands.map((command) => [command.id, command]));
      for (const command of commands) {
        merged.set(command.id, command);
      }
      return { commands: [...merged.values()] };
    });
  },

  setPaletteOpen: (open) => {
    set({ paletteOpen: open });
  },

  setContextTags: (tags) => {
    set({ contextTags: tags });
  },
}));

/** 记录最近使用的命令 id。 */
const RECENT_KEY = 'forgedesk.commands.recent';
const RECENT_LIMIT = 10;

export function recordRecentCommand(id: string): void {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    const parsed: unknown = raw === null ? [] : JSON.parse(raw);
    const list = Array.isArray(parsed) ? parsed.filter((v): v is string => typeof v === 'string') : [];
    const next = [id, ...list.filter((entry) => entry !== id)].slice(0, RECENT_LIMIT);
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // 最近使用是便利功能，失败即放弃
  }
}

/** 最近使用的命令 id（新 → 旧）。 */
export function readRecentCommands(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    const parsed: unknown = raw === null ? [] : JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter((v): v is string => typeof v === 'string') : [];
  } catch {
    return [];
  }
}

/** 自定义快捷键的设置键前缀（完整键 = `shortcuts.overrides.<id>`）。 */
export const SHORTCUT_OVERRIDE_PREFIX = 'shortcuts.overrides.';

/** 读一个命令的自定义键（未自定义时为 undefined）。 */
export function overrideKeyOf(
  values: Readonly<Record<string, string>>,
  id: string,
): string | undefined {
  const raw = values[`${SHORTCUT_OVERRIDE_PREFIX}${id}`];
  if (raw === undefined) {
    return undefined;
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    return typeof parsed === 'string' ? normalizeShortcut(parsed) : undefined;
  } catch {
    return undefined;
  }
}
