/**
 * 命令字典页（T5.4）：本地知识库的全量浏览与筛选。
 *
 * 数据与终端解释卡片同源（assets/git-explains.yaml，构建期打包，零网络）；
 * 按风险与分类筛选、按关键字搜索（命中命令名、子命令名或解释文本）。
 */
import { useMemo, useState } from 'react';

import { BookOpen, CircleAlert, CircleCheck, TriangleAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { systemOpenUrl } from '@/lib/ipc';
import { Input } from '@/ui/components/input';
import { SelectField } from '@/ui/components/select';
import { ToggleGroup } from '@/ui/components/toggle-group';
import { cn } from '@/lib/utils';

import { EXPLAIN_CATEGORIES, EXPLAIN_DICTIONARY } from '@/features/terminal/explainer';
import type { DictionaryEntry, ExplainEntry, ExplainRisk } from '@/features/terminal/explainer';

const RISKS: readonly ExplainRisk[] = ['safe', 'caution', 'dangerous'];

function RiskBadge({ risk }: { readonly risk: ExplainRisk }) {
  const { t } = useTranslation('shell');
  const map = {
    safe: { icon: CircleCheck, className: 'text-success' },
    caution: { icon: CircleAlert, className: 'text-warning' },
    dangerous: { icon: TriangleAlert, className: 'text-danger' },
  } as const;
  const { icon: Icon, className } = map[risk];
  return (
    <span className={cn('flex items-center gap-1 text-12 font-medium', className)}>
      <Icon aria-hidden="true" className="size-3.5" />
      {t(`terminal.explain.risk.${risk}`)}
    </span>
  );
}

/**
 * 一行条目：左列是名字（命令名或子命令/参数名），右列是风险、说明与文档链接。
 *
 * 名字列固定宽度（`w-44`）而不是让内容撑开：一页几十条里名字长短差很大，
 * 不固定就会出现"说明文字的起始位置每条都不一样"——用户反馈的"不对齐"。
 */
function EntryRow({
  name,
  entry,
  emphasis = false,
}: {
  readonly name: string;
  readonly entry: ExplainEntry;
  /** 命令级条目（true）比子命令更醒目：名字用等宽字体 + 前景色。 */
  readonly emphasis?: boolean;
}) {
  const { t } = useTranslation('shell');
  /**
   * 典型用法与名字完全相同时不重复渲染。
   *
   * 知识库里 `init` 的 example 就是 `git init`，`add` 是 `git add src/main.rs`——
   * 前者与名字逐字相同，再画一遍只是噪音（同一屏里同名出现两次还会让人以为
   * 列表里有两条 init）。
   */
  const example = entry.example?.trim();
  const showExample = example !== undefined && example !== '' && example !== `git ${name}`;

  return (
    <div
      className={cn(
        'border-line bg-surface flex items-start gap-3 rounded-md border p-3',
        !emphasis && 'bg-surface-sunken/40',
      )}
    >
      <div className="w-44 shrink-0">
        <span
          className={cn(
            'bg-surface-sunken inline-block max-w-full truncate rounded px-1.5 py-0.5 font-mono',
            emphasis ? 'text-13 font-medium text-fg' : 'text-12 text-fg-muted',
          )}
          title={name}
        >
          {emphasis ? `git ${name}` : name}
        </span>
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center justify-between gap-2">
          <RiskBadge risk={entry.risk} />
          {entry.docsUrl !== undefined ? (
            <button
              type="button"
              className="text-brand hover:underline flex shrink-0 items-center gap-1 text-12"
              onClick={() => void systemOpenUrl(entry.docsUrl ?? '').catch(() => {})}
            >
              <BookOpen aria-hidden="true" className="size-3.5" />
              {t('terminal.explain.docs')}
            </button>
          ) : null}
        </div>
        <p className="text-13 leading-relaxed">{entry.summary}</p>
        {showExample ? (
          <code className="text-fg-subtle truncate font-mono text-12">{example}</code>
        ) : null}
      </div>
    </div>
  );
}

export function CommandDictionaryPage() {
  const { t } = useTranslation('shell');
  const [query, setQuery] = useState('');
  const [risk, setRisk] = useState<string>('all');
  const [category, setCategory] = useState<string>('all');

  /**
   * 筛选 + **按分类分组**。
   *
   * 分组在筛选之后做：分类标题只出现一次（此前是每个条目各带一个分类标题，
   * 于是同一分类名在页面上重复几十遍，看起来像排版坏了）。
   *
   * 风险筛选按**命令自身**的风险判断，不再拿 `subs[0].risk` 顶替——
   * 那会让"只列危险命令"筛出"第一个子命令恰好危险"的普通命令。
   */
  const groups = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const matches = EXPLAIN_DICTIONARY.filter((entry) => {
      if (category !== 'all' && entry.category !== category) {
        return false;
      }
      if (risk !== 'all' && entry.command.risk !== risk) {
        return false;
      }
      if (needle === '') {
        return true;
      }
      // 命中范围：命令名、命令说明、以及子命令的名字与说明
      const haystack = [
        entry.commandName,
        entry.command.summary,
        ...(entry.subs ?? []).map((sub) => `${sub.name} ${sub.summary}`),
      ]
        .join(' ')
        .toLowerCase();
      return haystack.includes(needle);
    });

    const byCategory = new Map<string, DictionaryEntry[]>();
    for (const entry of matches) {
      const list = byCategory.get(entry.category);
      if (list === undefined) {
        byCategory.set(entry.category, [entry]);
      } else {
        list.push(entry);
      }
    }
    // 分组顺序沿用知识库里的分类出现顺序（EXPLAIN_CATEGORIES）
    return EXPLAIN_CATEGORIES.flatMap((name) => {
      const list = byCategory.get(name);
      return list === undefined ? [] : [{ category: name, entries: list }];
    });
  }, [category, query, risk]);

  const matchCount = groups.reduce((sum, group) => sum + group.entries.length, 0);

  return (
    <div className="mx-auto flex max-w-5xl flex-col gap-4 p-4" data-testid="command-dictionary">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.commands.title')}</h1>
        <p className="text-fg-muted text-13">{t('pages.commands.description')}</p>
      </header>

      <div className="flex flex-wrap items-center gap-2">
        <Input
          srLabel={t('terminal.dictionary.search')}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t('terminal.dictionary.search')}
          className="w-56"
        />
        <ToggleGroup
          label={t('terminal.dictionary.riskFilter')}
          value={risk}
          onValueChange={setRisk}
          options={[
            { value: 'all', label: t('terminal.dictionary.all') },
            ...RISKS.map((level) => ({ value: level, label: t(`terminal.explain.risk.${level}`) })),
          ]}
        />
        <SelectField
          label={t('terminal.dictionary.categoryFilter')}
          value={category}
          onValueChange={setCategory}
          options={[
            { value: 'all', label: t('terminal.dictionary.allCategories') },
            ...EXPLAIN_CATEGORIES.map((name) => ({
              value: name,
              label: t(`terminal.dictionary.category.${name}`),
            })),
          ]}
          className="w-40"
        />
        <span className="text-fg-subtle text-12">
          {t('terminal.dictionary.count', { count: matchCount })}
        </span>
      </div>

      <div className="flex flex-col gap-6">
        {groups.length === 0 ? (
          <p className="text-fg-muted text-13">{t('terminal.dictionary.empty')}</p>
        ) : (
          groups.map((group) => (
            <section key={group.category} className="flex flex-col gap-2">
              <h2 className="text-fg-muted border-line border-b pb-1 text-12 font-medium">
                {t(`terminal.dictionary.category.${group.category}`)}
              </h2>
              {group.entries.map((entry) => (
                <div key={entry.commandName} className="flex flex-col gap-2">
                  <EntryRow name={entry.commandName} entry={entry.command} emphasis />
                  {entry.subs.map((sub) => (
                    <EntryRow key={sub.name ?? ''} name={sub.name ?? ''} entry={sub} />
                  ))}
                </div>
              ))}
            </section>
          ))
        )}
      </div>

      <footer className="text-fg-subtle text-11">
        {t('terminal.explain.snapshotNote')}{' '}
        <button
          type="button"
          className="text-brand hover:underline"
          onClick={() =>
            void systemOpenUrl(
              'https://github.com/Ember1414/forgedesk/issues/new?labels=git-explains',
            ).catch(() => {})
          }
        >
          {t('terminal.explain.feedback')}
        </button>
      </footer>
    </div>
  );
}
