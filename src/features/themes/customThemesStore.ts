/**
 * 自定义主题列表（T6.6 §3）。
 *
 * 持久化复用 settingsStore（SQLite KV，键 `themes.customList`，值为 ThemeDefinition[]）：
 * 后端存储层不理解具体类型，新增设置项不需要迁移（见 settingsStore 头注释）。
 * 激活指针不在这里——它走 activeCustomTheme 的 localStorage 缓存（首帧同步要求）。
 *
 * 导入是 upsert：同 id 再导入视为更新（"改一版重新导入"是主题开发者的高频路径，
 * 弹"已存在"错误只会逼用户先删后导）。内置 id 在校验层就被拒绝，不会进入本列表。
 */
import { create } from 'zustand';

import {
  reconcileActiveCustomTheme,
  currentActiveCustomTheme,
  setActiveCustomTheme,
} from './activeCustomTheme';
import { validateThemeJson } from './themeModel';
import type { ThemeDefinition, ThemeFieldError } from './themeModel';
import { useSettingsStore } from '@/stores/settingsStore';

/** settingsStore 中的键。 */
export const CUSTOM_THEMES_KEY = 'themes.customList';

function readListFromSettings(): ThemeDefinition[] {
  const stored = useSettingsStore.getState().getJson<unknown>(CUSTOM_THEMES_KEY, []);
  if (!Array.isArray(stored)) {
    return [];
  }
  // 逐条复验：设置值可能被手工改坏；坏条目丢弃而不是让整页崩掉
  return stored.flatMap((entry) => {
    const result = validateThemeJson(entry);
    return result.ok ? [result.theme] : [];
  });
}

async function writeListToSettings(themes: readonly ThemeDefinition[]): Promise<void> {
  await useSettingsStore.getState().setJson(CUSTOM_THEMES_KEY, themes);
}

export type ImportOutcome =
  | { readonly ok: true; readonly theme: ThemeDefinition }
  | { readonly ok: false; readonly reason: 'parse'; readonly detail: string }
  | { readonly ok: false; readonly reason: 'invalid'; readonly errors: readonly ThemeFieldError[] };

export interface CustomThemesState {
  readonly themes: readonly ThemeDefinition[];
  readonly loaded: boolean;
  /** 从 settingsStore 读取列表并对账激活指针（幂等，settings 需先完成全局加载）。 */
  load(): void;
  /** 导入主题 JSON 文本（upsert by id）；持久化失败时回滚并抛出。 */
  importFromText(text: string): ImportOutcome;
  /** 删除自定义主题；若它是激活主题则同时取消激活。 */
  remove(id: string): void;
}

/** 初始状态（导出供测试复位）。 */
export const initialCustomThemesState = {
  themes: [] as readonly ThemeDefinition[],
  loaded: false,
};

export const useCustomThemesStore = create<CustomThemesState>()((set, get) => ({
  ...initialCustomThemesState,

  load: () => {
    const themes = readListFromSettings();
    set({ themes, loaded: true });
    reconcileActiveCustomTheme(themes);
  },

  importFromText: (text) => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch (error) {
      return {
        ok: false,
        reason: 'parse',
        detail: error instanceof Error ? error.message : String(error),
      };
    }
    const result = validateThemeJson(parsed);
    if (!result.ok) {
      return { ok: false, reason: 'invalid', errors: result.errors };
    }
    const imported = result.theme;
    const previous = get().themes;
    const next = [...previous.filter((theme) => theme.id !== imported.id), imported];
    set({ themes: next });
    void writeListToSettings(next).catch((error) => {
      // 持久化失败必须回滚内存态：否则重启后"主题不见了"而界面却显示已导入
      set({ themes: previous });
      throw error;
    });
    return { ok: true, theme: imported };
  },

  remove: (id) => {
    if (currentActiveCustomTheme()?.id === id) {
      setActiveCustomTheme(null);
    }
    const previous = get().themes;
    const next = previous.filter((theme) => theme.id !== id);
    set({ themes: next });
    void writeListToSettings(next).catch((error) => {
      set({ themes: previous });
      throw error;
    });
  },
}));
