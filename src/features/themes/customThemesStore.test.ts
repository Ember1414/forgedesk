import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { initialCustomThemesState, useCustomThemesStore } from './customThemesStore';
import { currentActiveCustomTheme, setActiveCustomTheme } from './activeCustomTheme';
import { validateThemeJson } from './themeModel';
import { initialSettingsState, useSettingsStore } from '@/stores/settingsStore';
import { settingsSet } from '@/lib/ipc';

vi.mock('@/lib/ipc', () => ({
  settingsAll: vi.fn(async () => ({})),
  settingsSet: vi.fn(async () => undefined),
}));

function validThemeJson(id = 'night-ink'): string {
  return JSON.stringify({
    id,
    name: 'Night Ink',
    appearance: 'dark',
    version: '1.0.0',
    colors: { canvas: '#101418' },
  });
}

beforeEach(() => {
  useCustomThemesStore.setState({ ...initialCustomThemesState });
  useSettingsStore.setState({ ...initialSettingsState });
  setActiveCustomTheme(null);
});

afterEach(() => {
  vi.mocked(settingsSet).mockClear();
  setActiveCustomTheme(null);
});

describe('自定义主题列表', () => {
  it('从设置存储加载列表，坏条目被丢弃而不是崩掉整页', () => {
    const good = JSON.parse(validThemeJson());
    const corrupt = { id: 'BROKEN ID', name: '' };
    useSettingsStore.setState({
      values: { 'themes.customList': JSON.stringify([good, corrupt]) },
      loaded: true,
    });

    useCustomThemesStore.getState().load();

    expect(useCustomThemesStore.getState().themes.map((t) => t.id)).toEqual(['night-ink']);
    expect(useCustomThemesStore.getState().loaded).toBe(true);
  });

  it('导入合法主题后写入设置存储（upsert：同 id 覆盖）', async () => {
    useCustomThemesStore.getState().importFromText(validThemeJson());
    useCustomThemesStore.getState().importFromText(validThemeJson('dawn-mist'));
    const updated = validThemeJson('night-ink').replace('"#101418"', '"#0c1014"');
    const outcome = useCustomThemesStore.getState().importFromText(updated);

    expect(outcome.ok).toBe(true);
    const themes = useCustomThemesStore.getState().themes;
    expect(themes.map((t) => t.id)).toEqual(['dawn-mist', 'night-ink']);
    expect(themes.find((t) => t.id === 'night-ink')?.colors.canvas).toBe('#0c1014');
    await vi.waitFor(() => {
      expect(settingsSet).toHaveBeenCalled();
    });
  });

  it('导入非法 JSON 返回 parse 错误；字段错误逐条返回', () => {
    const parseFail = useCustomThemesStore.getState().importFromText('{broken');
    expect(parseFail.ok).toBe(false);
    if (!parseFail.ok && parseFail.reason === 'parse') {
      expect(parseFail.detail.length).toBeGreaterThan(0);
    }

    const invalid = useCustomThemesStore
      .getState()
      .importFromText(JSON.stringify({ id: 'X', name: '', appearance: 'nope', version: 'bad' }));
    expect(invalid.ok).toBe(false);
    if (!invalid.ok && invalid.reason === 'invalid') {
      expect(invalid.errors.length).toBeGreaterThanOrEqual(4);
    }
    expect(useCustomThemesStore.getState().themes).toEqual([]);
  });

  it('删除激活中的主题时同时取消激活', () => {
    const outcome = useCustomThemesStore.getState().importFromText(validThemeJson());
    expect(outcome.ok).toBe(true);
    if (outcome.ok) {
      setActiveCustomTheme(outcome.theme);
    }
    expect(currentActiveCustomTheme()?.id).toBe('night-ink');

    useCustomThemesStore.getState().remove('night-ink');

    expect(currentActiveCustomTheme()).toBeNull();
    expect(useCustomThemesStore.getState().themes).toEqual([]);
  });

  it('设置里指向的主题被删除后，激活指针在对账时自动取消', () => {
    const theme = validateThemeJson(JSON.parse(validThemeJson()));
    expect(theme.ok).toBe(true);
    if (theme.ok) {
      setActiveCustomTheme(theme.theme);
    }

    // 模拟设置存储里该主题已被其它途径删除
    useSettingsStore.setState({ values: { 'themes.customList': '[]' }, loaded: true });
    useCustomThemesStore.getState().load();

    expect(currentActiveCustomTheme()).toBeNull();
  });
});
