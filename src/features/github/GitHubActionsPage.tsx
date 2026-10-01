/**
 * Actions 列表页（T4.9 UI）。
 *
 * # 仓库上下文是显式输入
 *
 * 与 PR/Issue 页同一形态：`owner/repo` 显式输入 + 游标分页。run 列表
 * 没有状态过滤（GitHub 的 runs 端点不按 state 筛，按需再补 created
 * 排序参数），行点击打开 run 详情（jobs + 取消/重跑 + 日志）。
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError, type NormalizedError } from '@/lib/errors';
import { repoActionsRunsList } from '@/lib/ipc';
import type { WorkflowRunSummary } from '@/lib/ipc';
import { Button } from '@/ui/components/button';
import { ErrorState } from '@/ui/components/error-state';
import { Input } from '@/ui/components/input';

import { ActionsRunDialog } from './ActionsRunDialog';

/** 仓库定位。 */
interface RepoRef {
  readonly owner: string;
  readonly repo: string;
}

/** `owner/repo` 输入的解析；不合式直接拒绝（与 PR/Issue 页同一规则）。 */
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

export function GitHubActionsPage() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();

  const [repoInput, setRepoInput] = useState('');
  const [target, setTarget] = useState<RepoRef | null>(null);
  const [items, setItems] = useState<readonly WorkflowRunSummary[]>([]);
  const [nextPage, setNextPage] = useState<number | null>(null);
  const [phase, setPhase] = useState<'loading' | 'ready' | 'error'>('ready');
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<NormalizedError | null>(null);
  const [runTarget, setRunTarget] = useState<{
    owner: string;
    repo: string;
    run: WorkflowRunSummary;
  } | null>(null);
  const seqRef = useRef(0);

  const load = useCallback(
    async (repo: RepoRef, page: number, replace: boolean) => {
      const seq = ++seqRef.current;
      if (replace) {
        setPhase('loading');
        setError(null);
      } else {
        setLoadingMore(true);
      }
      try {
        const result = await repoActionsRunsList({
          host: 'github.com',
          owner: repo.owner,
          repo: repo.repo,
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

  useEffect(() => {
    if (target === null) {
      return;
    }
    void Promise.resolve().then(() => load(target, 1, true));
  }, [target, load]);

  const refresh = () => {
    if (target !== null) {
      void load(target, 1, true);
    }
  };

  const authRequired = error?.code === 'AUTH_REQUIRED';

  return (
    <section className="flex flex-col gap-3" data-testid="actions-page">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.githubActions.title')}</h1>
        <p className="text-13 text-fg-muted">{t('pages.githubActions.description')}</p>
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
          data-testid="actions-repo-input"
        />
        <Button type="submit" variant="secondary" data-testid="actions-repo-go">
          {t('github.repos.searchGo')}
        </Button>
      </form>

      {phase === 'error' && target !== null && !authRequired && error !== null ? (
        <ErrorState
          title={t(`errors.${error.code}.title`)}
          hint={t('github.repos.listErrorHint')}
          onRetry={() => void load(target, 1, true)}
          retryLabel={t('github.repos.retry')}
        />
      ) : null}

      {phase === 'error' && authRequired ? (
        <ErrorState title={t('github.repos.signInRequired')} hint={t('github.repos.signInHint')} />
      ) : null}

      {phase === 'loading' ? (
        <p className="text-13 text-fg-subtle" data-testid="actions-loading">
          {t('github.repos.loading')}
        </p>
      ) : null}

      {target !== null && phase === 'ready' && items.length === 0 ? (
        <p className="text-13 text-fg-subtle" data-testid="actions-empty">
          {t('github.actions.empty')}
        </p>
      ) : null}

      <ul className="flex flex-col gap-2" data-testid="actions-items">
        {items.map((run) => (
          <li key={run.id}>
            <button
              type="button"
              className="fd-transition flex w-full flex-wrap items-center justify-between gap-2 rounded-md border border-line bg-surface px-3 py-2 text-left hover:border-line-strong"
              onClick={() =>
                setRunTarget({
                  owner: target?.owner ?? '',
                  repo: target?.repo ?? '',
                  run,
                })
              }
              data-testid={`actions-item-${run.id}`}
            >
              <span className="flex min-w-0 flex-col">
                <span className="flex items-center gap-2 text-13 font-medium">
                  <span
                    className="rounded-sm border border-line px-1.5 py-0.5 text-11"
                    data-testid={`actions-run-status-${run.id}`}
                  >
                    {run.status === 'completed' && run.conclusion !== null
                      ? t(`github.actions.conclusion.${run.conclusion}`, {
                          defaultValue: run.conclusion,
                        })
                      : t(`github.actions.status.${run.status}`, { defaultValue: run.status })}
                  </span>
                  <span className="truncate">{run.name}</span>
                </span>
                <span className="truncate text-12 text-fg-subtle">
                  {t('github.actions.runNumber', { number: run.runNumber })} ·{' '}
                  {run.headBranch ?? ''} · {t('github.prs.author', { author: run.actor })}
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
              void load(target, nextPage, false);
            }
          }}
          data-testid="actions-load-more"
        >
          {t('github.repos.loadMore')}
        </Button>
      ) : null}

      <ActionsRunDialog
        target={runTarget}
        onOpenChange={(open) => setRunTarget(open ? runTarget : null)}
        onRunChanged={refresh}
      />
    </section>
  );
}
