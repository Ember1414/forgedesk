/**
 * 历史筛选栏（T2.3）。
 *
 * # 职责边界
 *
 * 本组件是**受控的**：筛选状态（`HistoryFiltersState`）的真相源在 URL 上
 * （`HistoryPage` 经 `useSearchParams` 持有），这里只负责把状态画成控件、
 * 把交互翻译成 `onChange`。它自己不发请求、不知道 `git_log_page` 的存在——
 * 筛选 → 载荷的换算在 `historyFilters.ts`（有单测）。
 *
 * # 控件清单（任务书 T2.3 第 1 条）
 *
 * - 关键词输入（300ms 防抖提交；大小写开关内联在输入框右侧）
 * - 筛选弹出面板：分支多选（含"全部分支"互斥项）、作者（仓库作者列表单选）、
 *   时间范围（预设 + 自定义日期）、仅合并 / 仅我的提交开关
 * - 活跃筛选 chips：每个生效的筛选一个可移除的胶囊——**这一排比弹出面板本身
 *   更重要**，它让"当前为什么看不到某条提交"永远可见。
 */
import { useEffect, useMemo, useState } from 'react';

import { Filter, Search, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { AuthorSummary, Branch } from '@/lib/ipc/history';
import { cn } from '@/lib/utils';

import { Button } from '@/ui/components/button';
import { Checkbox } from '@/ui/components/checkbox';
import { Input } from '@/ui/components/input';
import { Popover, PopoverContent, PopoverTrigger } from '@/ui/components/popover';
import { Switch } from '@/ui/components/switch';

import {
  applyTimePreset,
  type HistoryFiltersState,
  type TimePreset,
} from '@/features/history/historyFilters';

/** 关键词输入的防抖（毫秒）：比逐键请求省，比"按回车才搜"顺手。 */
const KEYWORD_DEBOUNCE_MS = 300;

/** 时间预设选项（'custom' 由日期输入隐式表达，不在按钮列里）。 */
const TIME_PRESETS: readonly TimePreset[] = ['today', 'week', 'month', 'quarter', 'all'];

export interface HistoryFilterBarProps {
  readonly state: HistoryFiltersState;
  readonly onChange: (next: HistoryFiltersState) => void;
  /** 仓库作者列表（空列表时作者控件退化为输入禁用态）。 */
  readonly authors: readonly AuthorSummary[];
  readonly branches: readonly Branch[];
  readonly className?: string;
}

export function HistoryFilterBar({
  state,
  onChange,
  authors,
  branches,
  className,
}: HistoryFilterBarProps) {
  const { t } = useTranslation('shell');

  // 关键词本地缓冲：URL 是真相源，但逐键改 URL 会让历史栈与请求一起抖。
  // 外部变化（清除筛选、URL 直达）在渲染期同步进来（"props 变化时调整状态"
  // 模式：有守卫的渲染期 setState，而不是 effect 里 setState——后者会触发
  // 级联渲染，react-hooks 规则直接禁掉）；本地输入经防抖提交。
  const [keywordDraft, setKeywordDraft] = useState(state.keyword);
  const [committedKeyword, setCommittedKeyword] = useState(state.keyword);
  if (committedKeyword !== state.keyword) {
    setCommittedKeyword(state.keyword);
    setKeywordDraft(state.keyword);
  }
  useEffect(() => {
    if (keywordDraft === committedKeyword) {
      return;
    }
    const timer = setTimeout(() => {
      setCommittedKeyword(keywordDraft);
      onChange({ ...state, keyword: keywordDraft });
    }, KEYWORD_DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
    };
  });

  // 活跃筛选 chips：每颗携带"移除它之后"的完整状态（next），点掉 = 单独撤销那一条。
  const activeChips = useMemo(() => buildActiveChips(state, t), [state, t]);
  const activeCount = activeChips.length;

  return (
    <div
      className={cn('flex min-w-0 flex-wrap items-center gap-1.5', className)}
      data-testid="history-filter-bar"
    >
      {/* 关键词 + 大小写开关 */}
      <div className="relative min-w-44">
        <Search
          aria-hidden="true"
          className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-fg-subtle"
        />
        <Input
          value={keywordDraft}
          placeholder={t('history.filter.searchPlaceholder')}
          onChange={(event) => {
            setKeywordDraft(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              setKeywordDraft(state.keyword);
            }
          }}
          className="h-7 pl-7 pr-14 text-12"
          aria-label={t('history.filter.searchLabel')}
          data-testid="history-search-input"
        />
        <button
          type="button"
          aria-pressed={!state.caseInsensitive}
          title={t('history.filter.caseToggle')}
          onClick={() => {
            onChange({ ...state, caseInsensitive: !state.caseInsensitive });
          }}
          className={cn(
            'absolute right-1.5 top-1/2 -translate-y-1/2 rounded px-1 font-mono text-10',
            state.caseInsensitive
              ? 'text-fg-subtle hover:bg-surface-sunken'
              : 'bg-brand-subtle text-brand',
          )}
          data-testid="history-search-case"
        >
          Aa
        </button>
      </div>

      {/* 筛选弹出面板 */}
      <Popover>
        <PopoverTrigger asChild>
          <Button size="sm" variant="secondary" data-testid="history-filter-open">
            <Filter aria-hidden="true" className="size-3.5" />
            {t('history.filter.label')}
            {activeCount > 0 ? (
              <span className="rounded-full bg-brand px-1.5 text-10 text-fd-fg-inverted">
                {activeCount}
              </span>
            ) : null}
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-80 p-3 text-12">
          <div className="flex flex-col gap-3">
            {/* 分支多选（含"全部分支"互斥项） */}
            <section className="flex flex-col gap-1.5">
              <p className="text-11 font-medium text-fg-subtle">{t('history.filter.branches')}</p>
              <label className="flex items-center gap-2">
                <Checkbox
                  checked={state.allBranches}
                  onCheckedChange={(checked) => {
                    onChange({
                      ...state,
                      allBranches: checked === true,
                      revisions: checked === true ? [] : state.revisions,
                    });
                  }}
                  data-testid="history-filter-all-branches"
                />
                {t('history.filter.allBranches')}
              </label>
              {branches.length === 0 ? (
                <p className="text-11 text-fg-subtle">{t('history.filter.noBranches')}</p>
              ) : (
                <div className="max-h-32 overflow-y-auto rounded-sm border border-line p-1">
                  {branches.map((branch) => (
                    <label
                      key={branch.name}
                      className="flex items-center gap-2 rounded-sm px-1 py-0.5 hover:bg-surface-sunken"
                    >
                      <Checkbox
                        checked={state.revisions.includes(branch.name)}
                        disabled={state.allBranches}
                        onCheckedChange={(checked) => {
                          const next =
                            checked === true
                              ? [...state.revisions, branch.name]
                              : state.revisions.filter((name) => name !== branch.name);
                          onChange({ ...state, revisions: next, allBranches: false });
                        }}
                      />
                      <span className="truncate font-mono text-11" title={branch.name}>
                        {branch.name}
                      </span>
                      {branch.isHead ? (
                        <span className="ml-auto shrink-0 text-10 text-brand">
                          {t('history.filter.headMark')}
                        </span>
                      ) : null}
                    </label>
                  ))}
                </div>
              )}
            </section>

            {/* 作者（仓库作者列表单选） */}
            <section className="flex flex-col gap-1.5">
              <p className="text-11 font-medium text-fg-subtle">{t('history.filter.authors')}</p>
              {authors.length === 0 ? (
                <p className="text-11 text-fg-subtle">{t('history.filter.noAuthors')}</p>
              ) : (
                <select
                  value={state.author ?? ''}
                  onChange={(event) => {
                    const value = event.target.value;
                    onChange({ ...state, author: value === '' ? null : value });
                  }}
                  className="h-7 rounded-md border border-line bg-surface px-2 text-12"
                  data-testid="history-filter-author"
                >
                  <option value="">{t('history.filter.anyAuthor')}</option>
                  {authors.map((author) => (
                    <option key={author.email} value={author.email}>
                      {author.name} ({author.commitCount})
                    </option>
                  ))}
                </select>
              )}
            </section>

            {/* 时间范围：预设按钮列 + 自定义日期 */}
            <section className="flex flex-col gap-1.5">
              <p className="text-11 font-medium text-fg-subtle">{t('history.filter.time')}</p>
              <div className="flex flex-wrap gap-1">
                {TIME_PRESETS.map((preset) => {
                  const active = isPresetActive(state, preset);
                  return (
                    <button
                      key={preset}
                      type="button"
                      aria-pressed={active}
                      onClick={() => {
                        const range = applyTimePreset(preset, todayString());
                        onChange({ ...state, ...range });
                      }}
                      className={cn(
                        'fd-transition rounded-md border px-1.5 py-0.5 text-11',
                        active
                          ? 'border-brand bg-brand-subtle text-brand'
                          : 'border-line hover:bg-surface-sunken',
                      )}
                    >
                      {t(`history.filter.preset.${preset}`)}
                    </button>
                  );
                })}
              </div>
              <div className="flex items-center gap-1.5">
                <input
                  type="date"
                  value={state.sinceDay ?? ''}
                  onChange={(event) => {
                    onChange({
                      ...state,
                      sinceDay: event.target.value === '' ? null : event.target.value,
                    });
                  }}
                  aria-label={t('history.filter.since')}
                  className="h-7 min-w-0 flex-1 rounded-md border border-line bg-surface px-2 text-11"
                  data-testid="history-filter-since"
                />
                <span className="text-fg-subtle">–</span>
                <input
                  type="date"
                  value={state.untilDay ?? ''}
                  onChange={(event) => {
                    onChange({
                      ...state,
                      untilDay: event.target.value === '' ? null : event.target.value,
                    });
                  }}
                  aria-label={t('history.filter.until')}
                  className="h-7 min-w-0 flex-1 rounded-md border border-line bg-surface px-2 text-11"
                  data-testid="history-filter-until"
                />
              </div>
            </section>

            {/* 开关 */}
            <section className="flex flex-col gap-1.5">
              <label className="flex items-center justify-between gap-2">
                <span>{t('history.filter.mergesOnly')}</span>
                <Switch
                  checked={state.mergesOnly}
                  onCheckedChange={(checked) => {
                    onChange({ ...state, mergesOnly: checked });
                  }}
                  data-testid="history-filter-merges"
                />
              </label>
              <label className="flex items-center justify-between gap-2">
                <span>{t('history.filter.myCommitsOnly')}</span>
                <Switch
                  checked={state.myCommitsOnly}
                  onCheckedChange={(checked) => {
                    onChange({ ...state, myCommitsOnly: checked });
                  }}
                  data-testid="history-filter-mine"
                />
              </label>
            </section>
          </div>
        </PopoverContent>
      </Popover>

      {/* 活跃筛选 chips（路径过滤也在这里出现——它来自文件历史入口）。
          一颗 chip = 一个在起作用的筛选，点掉它 = 单独撤掉那一条。 */}
      {activeChips.map((chip) => (
        <button
          key={chip.key}
          type="button"
          title={t('history.filter.removeChip')}
          onClick={() => {
            onChange(chip.next);
          }}
          className="fd-transition flex items-center gap-1 rounded-full border border-line bg-surface-sunken px-1.5 py-0.5 text-11 text-fg-muted hover:bg-brand-subtle hover:text-brand"
          data-testid={chip.testId}
        >
          <span className="max-w-40 truncate">{chip.label}</span>
          <X aria-hidden="true" className="size-3" />
        </button>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------- 内部

/** 一颗活跃筛选 chip：`next` 是"移除它之后"的完整状态。 */
interface ActiveChip {
  readonly key: string;
  readonly label: string;
  readonly next: HistoryFiltersState;
  readonly testId: string;
}

/** 把当前状态翻译成 chips（i18n 注入；无 chip = 没有任何筛选在起作用）。 */
function buildActiveChips(
  state: HistoryFiltersState,
  t: (key: string) => string,
): readonly ActiveChip[] {
  const chips: ActiveChip[] = [];
  if (state.allBranches) {
    chips.push({
      key: 'all',
      label: t('history.filter.allBranches'),
      next: { ...state, allBranches: false },
      testId: 'history-chip-all',
    });
  }
  for (const revision of state.revisions) {
    chips.push({
      key: `rev:${revision}`,
      label: revision,
      next: { ...state, revisions: state.revisions.filter((name) => name !== revision) },
      testId: `history-chip-rev`,
    });
  }
  if (state.author !== null && state.author !== '') {
    chips.push({
      key: 'author',
      label: `${t('history.filter.authors')}: ${state.author}`,
      next: { ...state, author: null },
      testId: 'history-chip-author',
    });
  }
  if (state.sinceDay !== null || state.untilDay !== null) {
    const range = `${state.sinceDay ?? '…'} – ${state.untilDay ?? '…'}`;
    chips.push({
      key: 'time',
      label: range,
      next: { ...state, sinceDay: null, untilDay: null },
      testId: 'history-chip-time',
    });
  }
  if (state.keyword.trim() !== '') {
    chips.push({
      key: 'keyword',
      label: `"${state.keyword.trim()}"`,
      next: { ...state, keyword: '' },
      testId: 'history-chip-keyword',
    });
  }
  if (state.mergesOnly) {
    chips.push({
      key: 'merges',
      label: t('history.filter.mergesOnly'),
      next: { ...state, mergesOnly: false },
      testId: 'history-chip-merges',
    });
  }
  if (state.myCommitsOnly) {
    chips.push({
      key: 'mine',
      label: t('history.filter.myCommitsOnly'),
      next: { ...state, myCommitsOnly: false },
      testId: 'history-chip-mine',
    });
  }
  for (const path of state.paths) {
    chips.push({
      key: `path:${path}`,
      label: path,
      next: {
        ...state,
        paths: state.paths.filter((entry) => entry !== path),
        followRenames: false,
      },
      testId: 'history-chip-path',
    });
  }
  return chips;
}

function isPresetActive(state: HistoryFiltersState, preset: TimePreset): boolean {
  const range = applyTimePreset(preset, todayString());
  return state.sinceDay === range.sinceDay && state.untilDay === range.untilDay;
}

/** 今天（本地时区）的 `YYYY-MM-DD`。 */
function todayString(): string {
  const now = new Date();
  const month = `${now.getMonth() + 1}`.padStart(2, '0');
  const day = `${now.getDate()}`.padStart(2, '0');
  return `${now.getFullYear()}-${month}-${day}`;
}
