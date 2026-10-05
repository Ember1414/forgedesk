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
import type { ExplainEntry, ExplainRisk } from '@/features/terminal/explainer';

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

function EntryRow({ entry }: { readonly entry: ExplainEntry }) {
  const { t } = useTranslation('shell');
  return (
    <div className="border-line bg-surface rounded-md border p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          {entry.name !== null ? (
            <span className="bg-surface-sunken rounded px-1.5 py-0.5 font-mono text-12">
              {entry.name}
            </span>
          ) : null}
          <RiskBadge risk={entry.risk} />
        </div>
        {entry.docsUrl !== undefined ? (
          <button
            type="button"
            className="text-brand hover:underline flex items-center gap-1 text-12"
            onClick={() => void systemOpenUrl(entry.docsUrl ?? '').catch(() => {})}
          >
            <BookOpen aria-hidden="true" className="size-3.5" />
            {t('terminal.explain.docs')}
          </button>
        ) : null}
      </div>
      <p className="text-13 mt-1.5 leading-relaxed">{entry.summary}</p>
    </div>
  );
}

export function CommandDictionaryPage() {
  const { t } = useTranslation('shell');
  const [query, setQuery] = useState('');
  const [risk, setRisk] = useState<string>('all');
  const [category, setCategory] = useState<string>('all');

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return EXPLAIN_DICTIONARY.filter((entry) => {
      if (category !== 'all' && entry.category !== category) {
        return false;
      }
      const riskOf = (entry.subs[0]?.risk ?? entry.command.risk) as ExplainRisk;
      if (risk !== 'all' && riskOf !== risk) {
        return false;
      }
      if (needle === '') {
        return true;
      }
      const haystack = [
        entry.command.summary,
        ...(entry.subs ?? []).map((sub) => `${sub.name} ${sub.summary}`),
      ]
        .join(' ')
        .toLowerCase();
      return haystack.includes(needle);
    });
  }, [category, query, risk]);

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-4 p-4">
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
          {t('terminal.dictionary.count', { count: filtered.length })}
        </span>
      </div>

      <div className="flex flex-col gap-4">
        {filtered.length === 0 ? (
          <p className="text-fg-muted text-13">{t('terminal.dictionary.empty')}</p>
        ) : (
          filtered.map((entry) => (
            <section key={entry.command.summary} className="flex flex-col gap-2">
              <h2 className="text-fg-muted text-12">
                {t(`terminal.dictionary.category.${entry.category}`)}
              </h2>
              <EntryRow entry={entry.command} />
              {entry.subs.map((sub) => (
                <div key={sub.name} className="ps-6">
                  <EntryRow entry={sub} />
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
