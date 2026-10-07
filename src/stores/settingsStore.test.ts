import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { settingsAll, settingsSet } from '@/lib/ipc';
import {
  DENSITY_KEY,
  initialSettingsState,
  parseDensity,
  useSettingsStore,
} from '@/stores/settingsStore';

/**
 * 设置 store 的测试重点：
 *
 * 1. **乐观更新 + 失败回滚**：本地设置是"点完立刻要看到变化"的操作，
 *    但如果写入失败却不回滚，界面就会出现"显示已开启、下次启动又变回去"——
 *    这是最难排查的一类不一致，必须锁住。
 * 2. **脏数据不致命**：存储里可能有人手工改坏一个值，解析失败应回落默认而不是崩界面。
 * 3. **范围隔离**：仓库级设置不能悄悄写到全局。
 */
vi.mock('@/lib/ipc', () => ({
  settingsAll: vi.fn(),
  settingsSet: vi.fn(),
  // errors.ts 在"错误带 detail"时会调它做诊断（T5.6）。mock 里不声明这个导出的话，
  // 只要有用例走到那条路径，失败信息会是"mock 缺导出"——与诊断逻辑本身无关，
  // 排查时会把人引到完全错误的方向（CI 上出现过一次）。
  systemDiagnoseError: vi.fn().mockResolvedValue({ primary: null, related: [] }),
}));

const settingsAllMock = vi.mocked(settingsAll);
const settingsSetMock = vi.mocked(settingsSet);

beforeEach(() => {
  vi.clearAllMocks();
  useSettingsStore.setState(initialSettingsState);
  document.documentElement.removeAttribute('data-density');
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('load', () => {
  it('拉取全局设置并标记已加载', async () => {
    settingsAllMock.mockResolvedValue({ [DENSITY_KEY]: '"compact"', 'ui.other': '1' });

    await useSettingsStore.getState().load();

    const state = useSettingsStore.getState();
    expect(settingsAllMock).toHaveBeenCalledWith('global', undefined);
    expect(state.loaded).toBe(true);
    expect(state.loading).toBe(false);
    expect(state.values[DENSITY_KEY]).toBe('"compact"');
  });

  it('加载后立刻应用界面密度（不等用户进设置页）', async () => {
    settingsAllMock.mockResolvedValue({ [DENSITY_KEY]: '"compact"' });

    await useSettingsStore.getState().load();

    expect(document.documentElement.getAttribute('data-density')).toBe('compact');
  });

  it('已加载过则不重复请求（幂等）', async () => {
    settingsAllMock.mockResolvedValue({});
    await useSettingsStore.getState().load();
    await useSettingsStore.getState().load();

    expect(settingsAllMock).toHaveBeenCalledTimes(1);

    // force 可以强制刷新
    await useSettingsStore.getState().load({ force: true });
    expect(settingsAllMock).toHaveBeenCalledTimes(2);
  });

  it('失败时记录原因且不标记为已加载（留给界面重试）', async () => {
    settingsAllMock.mockRejectedValue(new Error('database is locked'));

    await useSettingsStore.getState().load();

    const state = useSettingsStore.getState();
    expect(state.loaded).toBe(false);
    expect(state.loading).toBe(false);
    expect(state.loadError).toBe('database is locked');
  });

  it('仓库范围会把 repoId 传下去', async () => {
    settingsAllMock.mockResolvedValue({});

    await useSettingsStore.getState().load({ scope: 'repo', repoId: 7 });

    expect(settingsAllMock).toHaveBeenCalledWith('repo', 7);
    expect(useSettingsStore.getState().scope).toBe('repo');
    expect(useSettingsStore.getState().repoId).toBe(7);
  });
});

describe('getJson', () => {
  it('解析已存储的值', async () => {
    settingsAllMock.mockResolvedValue({ answer: '{"mode":"compact","width":280}' });
    await useSettingsStore.getState().load();

    expect(useSettingsStore.getState().getJson('answer', { mode: 'x', width: 0 })).toEqual({
      mode: 'compact',
      width: 280,
    });
  });

  it('未设置或值损坏时返回兜底值（不抛错）', async () => {
    settingsAllMock.mockResolvedValue({ broken: 'not json at all' });
    await useSettingsStore.getState().load();

    expect(useSettingsStore.getState().getJson('missing', 'fallback')).toBe('fallback');
    expect(useSettingsStore.getState().getJson('broken', 'fallback')).toBe('fallback');
  });
});

describe('setJson', () => {
  it('乐观更新本地值并序列化为 JSON 写入后端', async () => {
    settingsSetMock.mockResolvedValue(undefined);

    await useSettingsStore.getState().setJson('ui.answer', { mode: 'compact' });

    expect(useSettingsStore.getState().values['ui.answer']).toBe('{"mode":"compact"}');
    expect(settingsSetMock).toHaveBeenCalledWith(
      'global',
      'ui.answer',
      '{"mode":"compact"}',
      undefined,
    );
  });

  it('写入界面密度时立刻改 <html>', async () => {
    settingsSetMock.mockResolvedValue(undefined);

    await useSettingsStore.getState().setJson(DENSITY_KEY, 'compact');

    expect(document.documentElement.getAttribute('data-density')).toBe('compact');
  });

  it('写入失败时回滚界面与本地值，并向上抛出（避免"假成功"）', async () => {
    settingsAllMock.mockResolvedValue({ [DENSITY_KEY]: '"comfortable"' });
    await useSettingsStore.getState().load();
    settingsSetMock.mockRejectedValue(new Error('disk full'));

    await expect(useSettingsStore.getState().setJson(DENSITY_KEY, 'compact')).rejects.toThrow(
      'disk full',
    );

    expect(useSettingsStore.getState().values[DENSITY_KEY]).toBe('"comfortable"');
    expect(document.documentElement.getAttribute('data-density')).toBe('comfortable');
  });
});

describe('parseDensity', () => {
  it('接受合法值并归一化非法输入', () => {
    expect(parseDensity('"compact"')).toBe('compact');
    expect(parseDensity('"comfortable"')).toBe('comfortable');
    // 非法：不是 JSON / 不是字符串 / 不在枚举内 / 缺省
    expect(parseDensity('compact')).toBe('comfortable');
    expect(parseDensity('42')).toBe('comfortable');
    expect(parseDensity('"cozy"')).toBe('comfortable');
    expect(parseDensity(undefined)).toBe('comfortable');
  });
});
