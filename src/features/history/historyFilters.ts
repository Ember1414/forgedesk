/**
 * 历史筛选的状态模型（T2.3）。
 *
 * # 三个形状，各管一段路
 *
 * - [`HistoryFiltersState`]：**UI 模型**——时间用 `YYYY-MM-DD`（输入框的原生形状）、
 *   作者按人（不是子串）、分支是数组。组件层直接绑定它。
 * - URL query（`filtersToSearchParams` / `filtersFromSearchParams`）：**可分享的
 *   状态**——刷新保持、贴给别人可复现，是筛选状态的**真相源**（`HistoryPage`
 *   用 `useSearchParams` 绑定）。
 * - `HistoryFilters`（IPC 形状，`filtersToQuery`）：发给 `git_log_page` 的载荷——
 *   时间换算成 Unix 秒（since = 当天 00:00、until = 当天 23:59:59，**本地时区**，
 *   与用户对"10 月 1 日之后"的直觉一致）。
 *
 * 全部纯函数：URL 编解码、载荷换算、活跃判定都有单测（验收项
 * "单测覆盖 filter → query 参数映射"就在这里）。
 */
import type { HistoryFilters } from '@/features/history/useGraphQuery';

/** 仓库级设置里存筛选状态的键（`settings_get` / `settings_set`，scope=repo）。 */
export const HISTORY_FILTERS_SETTING_KEY = 'history.filters';

/** 时间范围预设（"全部"就是没有 since/until，不单列一个值）。 */
export type TimePreset = 'today' | 'week' | 'month' | 'quarter' | 'all' | 'custom';

/** 筛选栏的 UI 模型。 */
export interface HistoryFiltersState {
  /** 选中的分支（多选；非空时 allBranches 为 false——两者在 UI 上互斥）。 */
  readonly revisions: readonly string[];
  /** 全部分支（`--all`）。 */
  readonly allBranches: boolean;
  /** 作者（单选，来自作者列表；`null` = 不过滤）。 */
  readonly author: string | null;
  /** 起始日期（含），`YYYY-MM-DD`；`null` = 不限。 */
  readonly sinceDay: string | null;
  /** 截止日期（含），`YYYY-MM-DD`；`null` = 不限。 */
  readonly untilDay: string | null;
  /** 消息关键词（字面匹配）。 */
  readonly keyword: string;
  /** 关键词忽略大小写（搜索框缺省忽略——对用户友好；UI 可切回区分）。 */
  readonly caseInsensitive: boolean;
  /** 仅显示合并提交。 */
  readonly mergesOnly: boolean;
  /** 仅显示我的提交（后端按仓库 user.email 翻译）。 */
  readonly myCommitsOnly: boolean;
  /** 路径过滤（文件历史入口；`followRenames` 只对单路径生效）。 */
  readonly paths: readonly string[];
  /** 路径过滤时跟随重命名（`--follow`）。 */
  readonly followRenames: boolean;
}

/** 空筛选（引用稳定的模块级常量）。 */
export const EMPTY_FILTERS_STATE: HistoryFiltersState = {
  revisions: [],
  allBranches: false,
  author: null,
  sinceDay: null,
  untilDay: null,
  keyword: '',
  caseInsensitive: true,
  mergesOnly: false,
  myCommitsOnly: false,
  paths: [],
  followRenames: false,
};

/** 日期字符串 → 当天 00:00（本地时区）的 Unix 秒；非法输入返回 null。 */
export function dayStartSeconds(day: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (match === null) {
    return null;
  }
  const date = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  if (Number.isNaN(date.getTime())) {
    return null;
  }
  return Math.floor(date.getTime() / 1000);
}

/** 日期字符串 → 当天 23:59:59（本地时区）的 Unix 秒（截止日**含**当天）。 */
export function dayEndSeconds(day: string): number | null {
  const start = dayStartSeconds(day);
  return start === null ? null : start + 86_399;
}

/** UI 模型 → `git_log_page` 的查询载荷（不含分页两兄弟）。 */
export function filtersToQuery(state: HistoryFiltersState): HistoryFilters {
  const query: { -readonly [K in keyof HistoryFilters]?: HistoryFilters[K] } = {};
  if (state.revisions.length > 0) {
    query.revisions = [...state.revisions];
  } else if (state.allBranches) {
    query.allBranches = true;
  }
  if (state.author !== null && state.author !== '') {
    query.author = state.author;
  }
  if (state.sinceDay !== null) {
    const since = dayStartSeconds(state.sinceDay);
    if (since !== null) {
      query.since = since;
    }
  }
  if (state.untilDay !== null) {
    const until = dayEndSeconds(state.untilDay);
    if (until !== null) {
      query.until = until;
    }
  }
  const keyword = state.keyword.trim();
  if (keyword !== '') {
    query.messageContains = keyword;
    query.caseInsensitive = state.caseInsensitive;
  }
  if (state.mergesOnly) {
    query.mergesOnly = true;
  }
  if (state.myCommitsOnly) {
    query.myCommitsOnly = true;
  }
  if (state.paths.length > 0) {
    query.paths = [...state.paths];
  }
  if (state.followRenames && state.paths.length === 1) {
    query.followRenames = true;
  }
  return query;
}

/** 是否有任何筛选在起作用（决定"清除筛选"按钮与空态文案）。 */
export function hasActiveFilters(state: HistoryFiltersState): boolean {
  return (
    state.revisions.length > 0 ||
    state.allBranches ||
    (state.author !== null && state.author !== '') ||
    state.sinceDay !== null ||
    state.untilDay !== null ||
    state.keyword.trim() !== '' ||
    state.mergesOnly ||
    state.myCommitsOnly ||
    state.paths.length > 0
  );
}

// ---------------------------------------------------------------- URL 编解码

/** URL 参数名（保持短：分享链接里它们会出现很多次）。 */
const PARAM_REVISIONS = 'rev';
const PARAM_ALL = 'all';
const PARAM_AUTHOR = 'author';
const PARAM_SINCE = 'since';
const PARAM_UNTIL = 'until';
const PARAM_KEYWORD = 'q';
const PARAM_CASE = 'case';
const PARAM_MERGES = 'merges';
const PARAM_MINE = 'mine';
const PARAM_PATHS = 'path';
const PARAM_FOLLOW = 'follow';

/**
 * UI 模型 → URL query。
 *
 * 只有**非缺省**字段才写进 URL：默认状态的分享链接不带一串 `false`。
 * `caseInsensitive=true` 与空 keyword 是缺省，不占参数位。
 */
export function filtersToSearchParams(state: HistoryFiltersState): URLSearchParams {
  const params = new URLSearchParams();
  if (state.revisions.length > 0) {
    for (const revision of state.revisions) {
      params.append(PARAM_REVISIONS, revision);
    }
  } else if (state.allBranches) {
    params.set(PARAM_ALL, '1');
  }
  if (state.author !== null && state.author !== '') {
    params.set(PARAM_AUTHOR, state.author);
  }
  if (state.sinceDay !== null) {
    params.set(PARAM_SINCE, state.sinceDay);
  }
  if (state.untilDay !== null) {
    params.set(PARAM_UNTIL, state.untilDay);
  }
  const keyword = state.keyword.trim();
  if (keyword !== '') {
    params.set(PARAM_KEYWORD, keyword);
    if (!state.caseInsensitive) {
      params.set(PARAM_CASE, '1');
    }
  }
  if (state.mergesOnly) {
    params.set(PARAM_MERGES, '1');
  }
  if (state.myCommitsOnly) {
    params.set(PARAM_MINE, '1');
  }
  for (const path of state.paths) {
    params.append(PARAM_PATHS, path);
  }
  if (state.followRenames && state.paths.length === 1) {
    params.set(PARAM_FOLLOW, '1');
  }
  return params;
}

/** URL query → UI 模型（未知参数忽略；非法值按缺省处理，不抛错）。 */
export function filtersFromSearchParams(params: URLSearchParams): HistoryFiltersState {
  const revisions = params.getAll(PARAM_REVISIONS).filter((revision) => revision !== '');
  const paths = params.getAll(PARAM_PATHS).filter((path) => path !== '');
  const sinceDay = normalizeDay(params.get(PARAM_SINCE));
  const untilDay = normalizeDay(params.get(PARAM_UNTIL));
  return {
    revisions,
    allBranches: revisions.length === 0 && params.get(PARAM_ALL) === '1',
    author: emptyToNull(params.get(PARAM_AUTHOR)),
    sinceDay,
    untilDay,
    keyword: params.get(PARAM_KEYWORD) ?? '',
    caseInsensitive: params.get(PARAM_CASE) !== '1',
    mergesOnly: params.get(PARAM_MERGES) === '1',
    myCommitsOnly: params.get(PARAM_MINE) === '1',
    paths,
    followRenames: paths.length === 1 && params.get(PARAM_FOLLOW) === '1',
  };
}

/** URL 上是否有任何筛选参数（决定"初始时要不要用 settings 回填"）。 */
export function searchParamsHaveFilters(params: URLSearchParams): boolean {
  return filtersToSearchParams(filtersFromSearchParams(params)).toString() !== '';
}

function emptyToNull(value: string | null): string | null {
  return value === null || value === '' ? null : value;
}

function normalizeDay(value: string | null): string | null {
  if (value === null || dayStartSeconds(value) === null) {
    return null;
  }
  return value;
}

// ---------------------------------------------------------------- settings 持久化

/**
 * 把 UI 模型压成 settings 里存的 JSON（与 URL 同构：只存非缺省字段）。
 *
 * 为什么不走 URLSearchParams：settings 的值是任意字符串，直接存
 * `URLSearchParams.toString()` 即可复用同一套编码（且肉眼可读）。
 */
export function filtersToSettingValue(state: HistoryFiltersState): string {
  return filtersToSearchParams(state).toString();
}

/** settings 里存的值 → UI 模型；空串/坏值回退到空筛选。 */
export function filtersFromSettingValue(value: string | null | undefined): HistoryFiltersState {
  if (value === null || value === undefined || value === '') {
    return EMPTY_FILTERS_STATE;
  }
  return filtersFromSearchParams(new URLSearchParams(value));
}

/**
 * 预设时间范围 → (sinceDay, untilDay)。
 *
 * 以**当天**为锚点（本地时区）：传 `null` 表示"没有今天"（测试注入）。
 */
export function applyTimePreset(
  preset: TimePreset,
  today: string | null,
): { sinceDay: string | null; untilDay: string | null } {
  if (today === null) {
    return { sinceDay: null, untilDay: null };
  }
  const base = dayStartSeconds(today);
  if (base === null) {
    return { sinceDay: null, untilDay: null };
  }
  const daySeconds = (offset: number): string => {
    const date = new Date((base + offset * 86_400) * 1000);
    const month = `${date.getMonth() + 1}`.padStart(2, '0');
    const day = `${date.getDate()}`.padStart(2, '0');
    return `${date.getFullYear()}-${month}-${day}`;
  };
  switch (preset) {
    case 'today':
      return { sinceDay: today, untilDay: today };
    case 'week':
      return { sinceDay: daySeconds(-6), untilDay: today };
    case 'month':
      return { sinceDay: daySeconds(-29), untilDay: today };
    case 'quarter':
      return { sinceDay: daySeconds(-89), untilDay: today };
    case 'all':
      return { sinceDay: null, untilDay: null };
    case 'custom':
      return { sinceDay: null, untilDay: null };
  }
}
