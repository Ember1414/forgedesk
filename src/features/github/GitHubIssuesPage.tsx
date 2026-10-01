/**
 * Issue 列表页（T4.8 UI）。
 *
 * # 仓库上下文是显式输入
 *
 * 与 PR 页同一形态：`owner/repo` 显式输入，状态过滤 + 游标分页 +
 * 列表 → 详情。区别是这里多了创建入口（新建对话框），以及列表条目
 * 展示标签/指派/评论数——Issue 的筛选维度比 PR 少，但元数据更多。
 *
 * # 列表不含 PR
 *
 * GitHub 的 issues 端点会把 PR 混进来；后端（provider 层）已按
 * `pull_request` 键过滤，前端拿到的就是纯 Issue。
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError, type NormalizedError } from '@/lib/errors';
import { repoIssueList } from '@/lib/ipc';
import type { IssueSummary } from '@/lib/ipc';
import { Button } from '@/ui/components/button';
import { ErrorState } from '@/ui/components/error-state';
import { Input } from '@/ui/components/input';

import { IssueCreateDialog } from './IssueCreateDialog';
import { IssueDetailDialog } from './IssueDetailDialog';

/** 状态过滤。 */
type IssueStateFilter = 'open' | 'closed' | 'all';

const STATE_TABS: readonly IssueStateFilter[] = ['open', 'closed', 'all'];

/** 仓库定位。 */
interface RepoRef {
  readonly owner: string;
  readonly repo: string;
}

/** `owner/repo` 输入的解析；不合式直接拒绝（与 PR 页同一规则）。 */
function parseRepoRef(raw: string): RepoRef | null {
  const parts = raw
    .trim()
    .split('/')
    .filter((part) => part !== '');
  const owner = parts[0];
  const repo = parts[1];
  if (owner === undefined || repo === undefined || parts.length !== 2) {
    return null;
  }
  return { owner, repo };
}

export function GitHubIssuesPage() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();

  const [repoInput, setRepoInput] = useState('');
  const [target, setTarget] = useState<RepoRef | null>(null);
  const [stateFilter, setStateFilter] = useState<IssueStateFilter>('open');
  const [items, setItems] = useState<readonly IssueSummary[]>([]);
  const [nextPage, setNextPage] = useState<number | null>(null);
  const [phase, setPhase] = useState<'loading' | 'ready' | 'error'>('ready');
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<NormalizedError | null>(null);
  const [detailTarget, setDetailTarget] = useState<{
    owner: string;
    repo: string;
    number: number;
  } | null>(null);
  const [creating, setCreating] = useState(false);
  const seqRef = useRef(0);

  const load = useCallback(
    async (repo: RepoRef, filter: IssueStateFilter, page: number, replace: boolean) => {
      const seq = ++seqRef.current;
      if (replace) {
        setPhase('loading');
        setError(null);
      } else {
        setLoadingMore(true);
      }
      try {
        const result = await repoIssueList({
          host: 'github.com',
          owner: repo.owner,
          repo: repo.repo,
          stateFilter: filter,
          page,
        });
        if (seq !== seqRef.current) {
          return;
        }
        setItems((current) => (replace ? result.items : [...current, ...result.items]));
        setNextPage(result.nextPage);
        setPhase('ready');
      } catch (raw) {
        if (seq !== seqRef.current) {
          return;
        }
        if (replace) {
          setError(show(raw));
          setPhase('error');
        } else {
          show(raw);
        }
      } finally {
        if (seq === seqRef.current) {
          setLoadingMore(false);
        }
      }
    },
    [show],
  );

  const submitRepo = () => {
    const parsed = parseRepoRef(repoInput);
    if (parsed === null) {
      return;
    }
    setTarget(parsed);
    setItems([]);
    setNextPage(null);
  };

  // 目标或过滤条件变化 → 重新加载第一页
  useEffect(() => {
    if (target === null) {
      return;
    }
    void Promise.resolve().then(() => load(target, stateFilter, 1, true));
  }, [target, stateFilter, load]);

  const refresh = () => {
    if (target !== null) {
      void load(target, stateFilter, 1, true);
    }
  };

  const authRequired = error?.code === 'AUTH_REQUIRED';

  return (
    <section className="flex flex-col gap-3" data-testid="issues-page">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.githubIssues.title')}</h1>
        <p className="text-13 text-fg-muted">{t('pages.githubIssues.description')}</p>
      </header>

      <form
        className="flex gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          submitRepo();
        }}
      >
        <Input
          value={repoInput}
          onChange={(event) => setRepoInput(event.target.value)}
          placeholder={t('github.prs.repoPlaceholder')}
          aria-label={t('github.prs.repoPlaceholder')}
          data-testid="issues-repo-input"
        />
        <Button type="submit" variant="secondary" data-testid="issues-repo-go">
          {t('github.repos.searchGo')}
        </Button>
      </form>

      {target !== null ? (
        <div className="flex flex-wrap items-center gap-2">
          <div role="tablist" aria-label={t('github.issues.stateTabs')} className="flex gap-2">
            {STATE_TABS.map((filter) => (
              <Button
                key={filter}
                type="button"
                variant={stateFilter === filter ? 'primary' : 'secondary'}
                aria-pressed={stateFilter === filter}
                onClick={() => setStateFilter(filter)}
                data-testid={`issues-tab-${filter}`}
              >
                {t(`github.issues.stateTab.${filter}`)}
              </Button>
            ))}
          </div>
          <Button
            type="button"
            variant="secondary"
            className="ml-auto"
            onClick={() => setCreating(true)}
            data-testid="issues-create-open"
          >
            {t('github.issues.createOpen')}
          </Button>
        </div>
      ) : null}

      {phase === 'error' && target !== null && !authRequired && error !== null ? (
        <ErrorState
          title={t(`errors.${error.code}.title`)}
          hint={t('github.repos.listErrorHint')}
          onRetry={() => void load(target, stateFilter, 1, true)}
          retryLabel={t('github.repos.retry')}
        />
      ) : null}

      {phase === 'error' && authRequired ? (
        <ErrorState title={t('github.repos.signInRequired')} hint={t('github.repos.signInHint')} />
      ) : null}

      {phase === 'loading' ? (
        <p className="text-13 text-fg-subtle" data-testid="issues-loading">
          {t('github.repos.loading')}
        </p>
      ) : null}

      {target !== null && phase === 'ready' && items.length === 0 ? (
        <p className="text-13 text-fg-subtle" data-testid="issues-empty">
          {t('github.issues.empty')}
        </p>
      ) : null}

      <ul className="flex flex-col gap-2" data-testid="issues-items">
        {items.map((issue) => (
          <li key={issue.number}>
            <button
              type="button"
              className="fd-transition flex w-full flex-wrap items-center justify-between gap-2 rounded-md border border-line bg-surface px-3 py-2 text-left hover:border-line-strong"
              onClick={() =>
                setDetailTarget({
                  owner: target?.owner ?? '',
                  repo: target?.repo ?? '',
                  number: issue.number,
                })
              }
              data-testid={`issues-item-${issue.number}`}
            >
              <span className="flex min-w-0 flex-col">
                <span className="flex items-center gap-2 text-13 font-medium">
                  <span className="font-mono text-fg-subtle">#{issue.number}</span>
                  <span className="truncate">{issue.title}</span>
                  {issue.labels.map((label) => (
                    <span
                      key={label}
                      className="rounded-sm border border-line px-1.5 py-0.5 text-11 text-fg-muted"
                    >
                      {label}
                    </span>
                  ))}
                </span>
                <span className="truncate text-12 text-fg-subtle">
                  {t('github.prs.author', { author: issue.author })} ·{' '}
                  {issue.assignees.length > 0
                    ? issue.assignees.join(', ')
                    : t('github.issues.unassigned')}
                </span>
              </span>
              <span
                className="font-mono text-12 text-fg-subtle"
                aria-label={t('github.prs.commentsTitle')}
              >
                {issue.comments}
              </span>
            </button>
          </li>
        ))}
      </ul>

      {nextPage !== null && phase === 'ready' ? (
        <Button
          type="button"
          variant="secondary"
          disabled={loadingMore}
          onClick={() => {
            if (target !== null) {
              void load(target, stateFilter, nextPage, false);
            }
          }}
          data-testid="issues-load-more"
        >
          {t('github.repos.loadMore')}
        </Button>
      ) : null}

      <IssueCreateDialog
        target={creating && target !== null ? { owner: target.owner, repo: target.repo } : null}
        onOpenChange={(open) => setCreating(open)}
        onCreated={() => refresh()}
      />

      <IssueDetailDialog
        target={detailTarget}
        onOpenChange={(open) => setDetailTarget(open ? detailTarget : null)}
        onChanged={refresh}
      />
    </section>
  );
}
