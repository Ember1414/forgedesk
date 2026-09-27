/**
 * 应用设置（持久化在本地 SQLite 中）。
 *
 * # 与 localStorage 的分工
 *
 * 设置里有两类东西，存储位置不同，这不是历史遗留而是刻意的：
 *
 * | 内容 | 位置 | 原因 |
 * | --- | --- | --- |
 * | 主题（亮/暗/跟随系统） | localStorage（`src/app/theme.ts`） | 必须在**首帧渲染前同步**可用，否则暗色用户会看到一次白闪 |
 * | 其余设置（界面密度等） | SQLite（本 store） | 走 IPC 是异步的，但换来"可查询、可迁移、可与仓库绑定" |
 *
 * 所以主题**不会**出现在这里。若将来要把主题也放进数据库，必须先解决首帧同步问题
 * （例如在 HTML 里内联一段读取 localStorage 缓存的脚本），否则就是拿体验换一致性。
 *
 * # 值一律是 JSON 字符串
 *
 * 与后端契约一致（见 docs/API.md）：存储层不理解具体类型，类型解析由本 store 负责
 * （`getJson` / `setJson`）。新增设置项不需要改后端，也不需要迁移。
 */
import { create } from 'zustand';

import { settingsAll, settingsSet } from '@/lib/ipc';
import type { SettingsScope } from '@/lib/ipc';

/** 界面密度：影响主内容区与列表的留白。 */
export const DENSITIES = ['comfortable', 'compact'] as const;
export type Density = (typeof DENSITIES)[number];

/** 界面密度的设置键。 */
export const DENSITY_KEY = 'ui.density';

/**
 * 仓库自动刷新的开关（T1.10）。
 *
 * 缺省开启：文件监听是"界面自己跟上仓库"的前提；关掉之后退回 15 秒轮询
 * （见 `WorkspaceStatusPage`）。键名与后端 `watch::AUTO_REFRESH_KEY` 一致。
 */
export const WATCH_AUTO_REFRESH_KEY = 'watch.autoRefresh';

/** 去抖动窗口（毫秒）。后端会把它收敛到 [50, 5000]。 */
export const WATCH_DEBOUNCE_KEY = 'watch.debounceMs';

/** 去抖动窗口的可选值（设置页展示这些）。 */
export const DEBOUNCE_CHOICES_MS = [100, 300, 500, 1000] as const;

/** 去抖动窗口的缺省值（与后端 `DEFAULT_DEBOUNCE_MS` 一致）。 */
export const DEFAULT_DEBOUNCE_MS = 300;

/**
 * 审计保留天数（T1.11）。键名与后端 `services::audit::RETENTION_DAYS_KEY` 一致。
 *
 * 缺省 90 天；后端会把值收敛到 `[1, 3650]`，因此界面不必替它做边界检查。
 */
export const AUDIT_RETENTION_DAYS_KEY = 'audit.retentionDays';

/** 审计保留条数上限（缺省 10000；后端收敛到 `[100, 1000000]`）。 */
export const AUDIT_RETENTION_MAX_KEY = 'audit.retentionMax';

/** 审计保留策略的缺省值（与后端默认值一致）。 */
export const AUDIT_RETENTION_DEFAULT_DAYS = 90;
export const AUDIT_RETENTION_DEFAULT_ROWS = 10_000;

const DEFAULT_DENSITY: Density = 'comfortable';

/**
 * 把密度应用到 `<html data-density>`。
 *
 * 与主题一样放在 store 的动作里而不是组件里：任何修改入口（设置页、命令面板、
 * 将来导入配置）都会经过这里，DOM 不会出现"改了 store 但界面没变"的漂移。
 */
export function applyDensity(density: Density): void {
  if (typeof document !== 'undefined') {
    document.documentElement.setAttribute('data-density', density);
  }
}

/** 从存储值解析密度；非法值回落到默认（存储里可能有手工改坏的旧数据）。 */
export function parseDensity(value: string | null | undefined): Density {
  if (value === null || value === undefined) {
    return DEFAULT_DENSITY;
  }
  try {
    const parsed: unknown = JSON.parse(value);
    return typeof parsed === 'string' && (DENSITIES as readonly string[]).includes(parsed)
      ? (parsed as Density)
      : DEFAULT_DENSITY;
  } catch {
    return DEFAULT_DENSITY;
  }
}

export interface SettingsState {
  /** 已加载的原始值（key → JSON 字符串）。 */
  readonly values: Readonly<Record<string, string>>;
  /** 是否已完成首次加载。 */
  readonly loaded: boolean;
  /** 加载是否进行中（用于骨架态）。 */
  readonly loading: boolean;
  /** 上次加载失败的原因（前端归一化后的文案由调用方渲染）。 */
  readonly loadError: string | null;
  /** 当前加载的范围。 */
  readonly scope: SettingsScope;
  /** 仓库级设置对应的仓库 id。 */
  readonly repoId: number | null;

  /** 加载指定范围的全部设置（幂等，可重复调用）。 */
  load(options?: { scope?: SettingsScope; repoId?: number | null; force?: boolean }): Promise<void>;
  /** 读取一个设置项（返回 JSON 字符串；未设置时为 `undefined`）。 */
  get(key: string): string | undefined;
  /** 读取并解析一个设置项，解析失败或未设置时返回 `fallback`。 */
  getJson<T>(key: string, fallback: T): T;
  /** 写入一个设置项（自动序列化为 JSON；乐观更新，失败时回滚并抛错）。 */
  setJson(key: string, value: unknown): Promise<void>;
}

/** 初始状态（导出供测试复位；store 是模块级单例）。 */
export const initialSettingsState = {
  values: {} as Readonly<Record<string, string>>,
  loaded: false,
  loading: false,
  loadError: null as string | null,
  scope: 'global' as SettingsScope,
  repoId: null as number | null,
};

function parseJson<T>(raw: string | undefined, fallback: T): T {
  if (raw === undefined) {
    return fallback;
  }
  try {
    return JSON.parse(raw) as T;
  } catch {
    // 存储里可能有手工改坏的值：写坏一条不该让整个界面崩掉
    return fallback;
  }
}

export const useSettingsStore = create<SettingsState>()((set, get) => ({
  ...initialSettingsState,

  load: async (options) => {
    const scope = options?.scope ?? get().scope;
    const repoId = options?.repoId ?? (scope === 'global' ? null : get().repoId);
    const force = options?.force ?? false;

    if (get().loading || (get().loaded && !force && scope === get().scope)) {
      return;
    }
    set({ loading: true, loadError: null, scope });

    try {
      // 边界归一化：IPC 返回 null/undefined（mock、旧后端）时按空表处理，
      // 否则整个 store 的 values 变成 null，任何 getJson 都会崩掉整页
      const values = (await settingsAll(scope, repoId ?? undefined)) ?? {};
      set({
        values,
        loaded: true,
        loading: false,
        repoId: scope === 'global' ? null : repoId,
      });
      // 密度是"启动即生效"的设置，加载完成后立刻应用，避免界面先按默认渲染完再跳一下
      applyDensity(parseDensity(values[DENSITY_KEY]));
    } catch (error) {
      set({
        loading: false,
        loaded: false,
        loadError: error instanceof Error ? error.message : String(error),
      });
    }
  },

  get: (key) => get().values[key],

  getJson: (key, fallback) => parseJson(get().values[key], fallback),

  setJson: async (key, value) => {
    const scope = get().scope;
    const repoId = get().repoId;
    const serialized = JSON.stringify(value);
    const previous = get().values;

    // 乐观更新：本地设置写入是"用户点完立刻应该看到变化"的操作，
    // 等 IPC 往返再更新会让开关看起来有延迟
    set({ values: { ...previous, [key]: serialized } });
    if (key === DENSITY_KEY) {
      applyDensity(parseDensity(serialized));
    }

    try {
      await settingsSet(scope, key, serialized, repoId ?? undefined);
    } catch (error) {
      // 失败必须回滚：否则界面显示"已开启"，下次启动却变回去——最难排查的一类不一致
      set({ values: previous });
      if (key === DENSITY_KEY) {
        applyDensity(parseDensity(previous[key]));
      }
      throw error;
    }
  },
}));
