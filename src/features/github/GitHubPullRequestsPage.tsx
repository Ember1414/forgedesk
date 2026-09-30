/**
 * Pull Request 列表页（T4.7 UI）。
 *
 * # 仓库上下文是显式输入
 *
 * GitHub 区域是全局页（不挂在某个本地仓库下），PR 列表因此需要
 * `owner/repo` 上下文：默认值取最近一次输入（会话内记忆），
 * 用户也可以直接粘贴任意仓库。后续接入"从本地仓库的远端带出"
 * 时只改默认值来源，不改数据流。
 *
 * # 列表 → 详情 → 合并
 *
 * 行点击打开详情对话框（条件展示 + 三策略合并 + head 预检）；
 * 合并成功后刷新当前列表（已合并的 PR 在 open 视图里自然消失）。
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError, type NormalizedError } from '@/lib/errors';
import { repoPullList } from '@/lib/ipc';
import type { PullSummary } from '@/lib/ipc';
import { PullDetailDialog } from '@/features/github/PullDetailDialog';
import { Button } from '@/ui/components/button';
import { ErrorState } from '@/ui/components/error-state';
import { Input } from '@/ui/components/input';

/** 状态过滤。 */
type PullStateFilter = 'open' | 'closed' | 'all';

const STATE_TABS: readonly PullStateFilter[] = ['open', 'closed', 'all'];

/** 仓库定位。 */
interface RepoRef {
  readonly owner: string;
  readonly repo: string;
}

/** `owner/repo` 输入的解析；不合式直接拒绝（错误在界面上提示）。 */
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

export function GitHubPullRequestsPage() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();

  const [repoInput, setRepoInput] = useState('');
  const [target, setTarget] = useState<RepoRef | null>(null);
  const [stateFilter, setStateFilter] = useState<PullStateFilter>('open');
  const [items, setItems] = useState<readonly PullSummary[]>([]);
  const [nextPage, setNextPage] = useState<number | null>(null);
  const [phase, setPhase] = useState<'loading' | 'ready' | 'error'>('ready');
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<NormalizedError | null>(null);
  const [detailTarget, setDetailTarget] = useState<{
    owner: string;
    repo: string;
    number: number;
  } | null>(null);
  const seqRef = useRef(0);

  const load = useCallback(
    async (repo: RepoRef, filter: PullStateFilter, page: number, replace: boolean) => {
      const seq = ++seqRef.current;
      if (replace) {
        setPhase('loading');
        setError(null);
      } else {
        setLoadingMore(true);
      }
      try {
        const result = await repoPullList({
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

  const authRequired = error?.code === 'AUTH_REQUIRED';

  return (
    <section className="flex flex-col gap-3" data-testid="pull-requests-page">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">
          {t('pages.githubPullRequests.title')}
        </h1>
        <p className="text-13 text-fg-muted">{t('pages.githubPullRequests.description')}</p>
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
          data-testid="prs-repo-input"
        />
        <Button type="submit" variant="secondary" data-testid="prs-repo-go">
          {t('github.repos.searchGo')}
        </Button>
      </form>

      {target !== null ? (
        <div role="tablist" aria-label={t('github.prs.stateTabs')} className="flex flex-wrap gap-2">
          {STATE_TABS.map((filter) => (
            <Button
              key={filter}
              type="button"
              variant={stateFilter === filter ? 'primary' : 'secondary'}
              aria-pressed={stateFilter === filter}
              onClick={() => setStateFilter(filter)}
              data-testid={`prs-tab-${filter}`}
            >
              {t(`github.prs.stateTab.${filter}`)}
            </Button>
          ))}
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
        <p className="text-13 text-fg-subtle" data-testid="prs-loading">
          {t('github.repos.loading')}
        </p>
      ) : null}

      {target !== null && phase === 'ready' && items.length === 0 ? (
        <p className="text-13 text-fg-subtle" data-testid="prs-empty">
          {t('github.prs.empty')}
        </p>
      ) : null}

      <ul className="flex flex-col gap-2" data-testid="prs-items">
        {items.map((pull) => (
          <li key={pull.number}>
            <button
              type="button"
              className="fd-transition flex w-full flex-wrap items-center justify-between gap-2 rounded-md border border-line bg-surface px-3 py-2 text-left hover:border-line-strong"
              onClick={() =>
                setDetailTarget({
                  owner: target?.owner ?? '',
                  repo: target?.repo ?? '',
                  number: pull.number,
                })
              }
              data-testid={`prs-item-${pull.number}`}
            >
              <span className="flex min-w-0 flex-col">
                <span className="flex items-center gap-2 text-13 font-medium">
                  <span className="font-mono text-fg-subtle">#{pull.number}</span>
                  <span className="truncate">{pull.title}</span>
                  {pull.draft ? (
                    <span className="rounded-sm border border-line px-1.5 py-0.5 text-11 text-fg-subtle">
                      {t('github.prs.draftBadge')}
                    </span>
                  ) : null}
                  {pull.merged ? (
                    <span className="rounded-sm border border-line px-1.5 py-0.5 text-11 text-fg-subtle">
                      {t('github.prs.mergedBadge')}
                    </span>
                  ) : null}
                </span>
                <span className="truncate font-mono text-12 text-fg-subtle">
                  {pull.headLabel} → {pull.baseLabel} · {pull.author}
                </span>
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
          data-testid="prs-load-more"
        >
          {t('github.repos.loadMore')}
        </Button>
      ) : null}

      <PullDetailDialog
        target={detailTarget}
        onOpenChange={(open) => setDetailTarget(open ? detailTarget : null)}
        onMerged={() => {
          // 合并后刷新：open 视图里该 PR 消失，closed 视图里状态更新
          if (target !== null) {
            void load(target, stateFilter, 1, true);
          }
        }}
      />
    </section>
  );
}
