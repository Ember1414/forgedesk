import { describe, expect, it } from 'vitest';

import {
  applyTimePreset,
  dayEndSeconds,
  dayStartSeconds,
  filtersFromSearchParams,
  filtersFromSettingValue,
  filtersToQuery,
  filtersToSearchParams,
  filtersToSettingValue,
  hasActiveFilters,
  searchParamsHaveFilters,
} from '@/features/history/historyFilters';
import { EMPTY_FILTERS_STATE } from '@/features/history/historyFilters';

/** 基准时间：2026-09-28 00:00 本地时区的 Unix 秒。 */
const TODAY = '2026-09-28';

describe('dayStartSeconds / dayEndSeconds（本地时区换算）', () => {
  it('起始=当天 00:00，截止=当天 23:59:59（截止日含当天）', () => {
    const start = dayStartSeconds('2026-09-28');
    const end = dayEndSeconds('2026-09-28');
    expect(end).toBe((start ?? 0) + 86_399);

    const date = new Date((start ?? 0) * 1000);
    expect(date.getFullYear()).toBe(2026);
    expect(date.getMonth()).toBe(8);
    expect(date.getDate()).toBe(28);
  });

  it('非法输入返回 null 而不是抛错', () => {
    expect(dayStartSeconds('')).toBeNull();
    expect(dayStartSeconds('2026-9-28')).toBeNull();
    expect(dayStartSeconds('not-a-day')).toBeNull();
  });
});

describe('filtersToQuery（UI 模型 → git_log_page 载荷）', () => {
  it('空筛选产出空载荷', () => {
    expect(filtersToQuery(EMPTY_FILTERS_STATE)).toEqual({});
  });

  it('分支多选优先于全部分支（两者同设时 revisions 赢，与后端约定一致）', () => {
    const query = filtersToQuery({
      ...EMPTY_FILTERS_STATE,
      revisions: ['main', 'dev'],
      allBranches: true,
    });
    expect(query.revisions).toEqual(['main', 'dev']);
    expect(query.allBranches).toBeUndefined();
  });

  it('全部分支 → allBranches=true', () => {
    const query = filtersToQuery({ ...EMPTY_FILTERS_STATE, allBranches: true });
    expect(query.allBranches).toBe(true);
    expect(query.revisions).toBeUndefined();
  });

  it('时间范围换算成本地时区的 Unix 秒闭区间', () => {
    const query = filtersToQuery({ ...EMPTY_FILTERS_STATE, sinceDay: TODAY, untilDay: TODAY });
    expect(query.since).toBe(dayStartSeconds(TODAY));
    expect(query.until).toBe(dayEndSeconds(TODAY));
  });

  it('关键词 trim 后进载荷，并带上大小写开关', () => {
    const query = filtersToQuery({
      ...EMPTY_FILTERS_STATE,
      keyword: '  fix login  ',
      caseInsensitive: false,
    });
    expect(query.messageContains).toBe('fix login');
    expect(query.caseInsensitive).toBe(false);
  });

  it('空关键词不写大小写开关', () => {
    const query = filtersToQuery({ ...EMPTY_FILTERS_STATE, caseInsensitive: false });
    expect(query.messageContains).toBeUndefined();
    expect(query.caseInsensitive).toBeUndefined();
  });

  it('单路径 + followRenames 才写 follow；多路径时忽略', () => {
    const one = filtersToQuery({
      ...EMPTY_FILTERS_STATE,
      paths: ['src/lib.rs'],
      followRenames: true,
    });
    expect(one.paths).toEqual(['src/lib.rs']);
    expect(one.followRenames).toBe(true);

    const many = filtersToQuery({
      ...EMPTY_FILTERS_STATE,
      paths: ['a.txt', 'b.txt'],
      followRenames: true,
    });
    expect(many.followRenames).toBeUndefined();
  });

  it('仅合并 / 仅我的提交进载荷', () => {
    const query = filtersToQuery({ ...EMPTY_FILTERS_STATE, mergesOnly: true, myCommitsOnly: true });
    expect(query.mergesOnly).toBe(true);
    expect(query.myCommitsOnly).toBe(true);
  });
});

describe('URL 编解码（刷新保持 / 分享）', () => {
  it('往返一致（round-trip）', () => {
    const state = {
      ...EMPTY_FILTERS_STATE,
      revisions: ['main', 'feature/x'],
      author: 'ada@example.com',
      sinceDay: TODAY,
      untilDay: '2026-10-01',
      keyword: 'fix login',
      caseInsensitive: false,
      mergesOnly: true,
      myCommitsOnly: true,
      paths: ['src/lib.rs'],
      followRenames: true,
    };
    const decoded = filtersFromSearchParams(filtersToSearchParams(state));
    expect(decoded).toEqual(state);
  });

  it('缺省字段不占 URL 参数位（默认链接是干净的）', () => {
    const params = filtersToSearchParams(EMPTY_FILTERS_STATE);
    expect(params.toString()).toBe('');
    expect(searchParamsHaveFilters(params)).toBe(false);
  });

  it('非法值回退到缺省而不是抛错', () => {
    const decoded = filtersFromSearchParams(new URLSearchParams('since=bad-date&rev=&all=1&q='));
    expect(decoded.sinceDay).toBeNull();
    expect(decoded.revisions).toEqual([]);
    expect(decoded.allBranches).toBe(true);
    expect(decoded.keyword).toBe('');
  });

  it('case=1 表示区分大小写；缺省是忽略', () => {
    expect(filtersFromSearchParams(new URLSearchParams('q=fix&case=1')).caseInsensitive).toBe(
      false,
    );
    expect(filtersFromSearchParams(new URLSearchParams('q=fix')).caseInsensitive).toBe(true);
  });

  it('有筛选参数时 searchParamsHaveFilters 为真', () => {
    expect(searchParamsHaveFilters(new URLSearchParams('q=fix'))).toBe(true);
    expect(searchParamsHaveFilters(new URLSearchParams('merges=1'))).toBe(true);
  });
});

describe('settings 持久化（repo 级）', () => {
  it('settings 值与 URL 同构，可互转', () => {
    const state = {
      ...EMPTY_FILTERS_STATE,
      allBranches: true,
      keyword: 'refactor',
      mergesOnly: true,
    };
    const stored = filtersToSettingValue(state);
    expect(filtersFromSettingValue(stored)).toEqual(state);
  });

  it('空值 / 坏值回退到空筛选', () => {
    expect(filtersFromSettingValue(null)).toEqual(EMPTY_FILTERS_STATE);
    expect(filtersFromSettingValue('')).toEqual(EMPTY_FILTERS_STATE);
    expect(filtersFromSettingValue('&&&')).toEqual(EMPTY_FILTERS_STATE);
  });
});

describe('hasActiveFilters（空态的"清除筛选"判据）', () => {
  it('全缺省为假；任一字段有值为真', () => {
    expect(hasActiveFilters(EMPTY_FILTERS_STATE)).toBe(false);
    expect(hasActiveFilters({ ...EMPTY_FILTERS_STATE, keyword: 'x' })).toBe(true);
    expect(hasActiveFilters({ ...EMPTY_FILTERS_STATE, paths: ['a'] })).toBe(true);
    expect(hasActiveFilters({ ...EMPTY_FILTERS_STATE, sinceDay: TODAY })).toBe(true);
  });
});

describe('applyTimePreset（预设 → 日期区间）', () => {
  it('今天 = 单日闭区间', () => {
    expect(applyTimePreset('today', TODAY)).toEqual({ sinceDay: TODAY, untilDay: TODAY });
  });

  it('近 7 天 = [今天-6, 今天]；近 30 天 = [今天-29, 今天]', () => {
    const week = applyTimePreset('week', TODAY);
    expect(week.sinceDay).toBe('2026-09-22');
    expect(week.untilDay).toBe(TODAY);
    const month = applyTimePreset('month', TODAY);
    expect(month.sinceDay).toBe('2026-08-30');
  });

  it('全部 = 清空时间', () => {
    expect(applyTimePreset('all', TODAY)).toEqual({ sinceDay: null, untilDay: null });
  });

  it('没有今天锚点时不产生区间', () => {
    expect(applyTimePreset('week', null)).toEqual({ sinceDay: null, untilDay: null });
  });
});
