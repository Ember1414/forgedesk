/**
 * 远程仓库页（T4.5）：账号名下 / 全部协作 / 星标 / 搜索 四个视角。
 *
 * # 分页为什么是"加载更多"而不是无限查询
 *
 * 后端把 GitHub 的 `Link` 头解析成页码游标（`nextPage === null` 即末页）；
 * 累积列表是**页面状态**（切标签即重置），不属于服务端缓存——
 * 与 `graphPagingStore` 的取舍一致，不进 Query 缓存。
 *
 * # Star 的语义边界
 *
 * 列表接口不返回"当前账号是否已星标"，因此按钮语义按标签区分：
 * 星标标签页里是"取消星标"（确定已星标），其余标签页是"加星"
 * （对已星标仓库重复 PUT 是幂等的，无副作用）。诚实优先于聪明。
 *
 * # 匿名与登录
 *
 * 前三个标签需要已登录账号（后端无账号时直接 `AUTH_REQUIRED`）；
 * 搜索匿名可用。收到 `AUTH_REQUIRED` 时给出"去登录"的直达入口
 * （设置 → 代码托管账号），而不是一条让人摸不着头脑的错误。
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

import { ReadmeDialog } from '@/features/github/ReadmeDialog';
import { useAppError, type NormalizedError } from '@/lib/errors';
import {
  repoRemoteFork,
  repoRemoteList,
  repoRemoteSearch,
  repoRemoteStar,
  repoRemoteStarred,
} from '@/lib/ipc';
import type { RemoteRepo, RemoteRepoPage, RemoteRepoScope } from '@/lib/ipc';
import { pushToast } from '@/stores/toastStore';

import { Button } from '@/ui/components/button';
import { ErrorState } from '@/ui/components/error-state';
import { Input } from '@/ui/components/input';

/** 数据视角。 */
type RepoTab = RemoteRepoScope | 'starred' | 'search';

/** 本页固定指向的站点（当前唯一已实现的 provider）。 */
const HOST = 'github.com';

const TABS: readonly { readonly key: RepoTab; readonly labelKey: string }[] = [
  { key: 'owned', labelKey: 'github.repos.tabOwned' },
  { key: 'all', labelKey: 'github.repos.tabAll' },
  { key: 'starred', labelKey: 'github.repos.tabStarred' },
  { key: 'search', labelKey: 'github.repos.tabSearch' },
];

/** 加载阶段。 */
type Phase = 'loading' | 'ready' | 'error';

interface RepoSource {
  readonly tab: RepoTab;
  readonly query: string;
}

export function GitHubReposPage() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const navigate = useNavigate();

  const [tab, setTab] = useState<RepoTab>('owned');
  const [searchInput, setSearchInput] = useState('');
  const [searchQuery, setSearchQuery] = useState('');
  const [items, setItems] = useState<readonly RemoteRepo[]>([]);
  const [nextPage, setNextPage] = useState<number | null>(null);
  const [phase, setPhase] = useState<Phase>('loading');
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<NormalizedError | null>(null);
  const [busyRepo, setBusyRepo] = useState<string | null>(null);
  const [copiedRepo, setCopiedRepo] = useState<string | null>(null);
  const [readmeRepo, setReadmeRepo] = useState<RemoteRepo | null>(null);
  // 只认最后一次请求的结果：切标签/连点加载更多时的过期响应直接丢弃
  const seqRef = useRef(0);

  const fetchPage = useCallback(
    async (target: RepoSource, page: number): Promise<RemoteRepoPage> => {
      switch (target.tab) {
        case 'owned':
          return repoRemoteList(HOST, { scope: 'owned', page });
        case 'all':
          return repoRemoteList(HOST, { scope: 'all', page });
        case 'starred':
          return repoRemoteStarred(HOST, { page });
        case 'search':
          return repoRemoteSearch(HOST, target.query, { page });
      }
    },
    [],
  );

  const load = useCallback(
    async (target: RepoSource, page: number, replace: boolean) => {
      const seq = ++seqRef.current;
      if (replace) {
        setPhase('loading');
        setError(null);
      } else {
        setLoadingMore(true);
      }
      try {
        const result = await fetchPage(target, page);
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
    [fetchPage, show],
  );

  // 切换标签 / 提交搜索 → 重新加载第一页。
  // 空搜索是"尚未开始"而不是空结果：经微任务复位（lint：不在 effect 里同步 setState）
  useEffect(() => {
    if (tab === 'search' && searchQuery.trim() === '') {
      void Promise.resolve().then(() => {
        seqRef.current += 1;
        setItems([]);
        setNextPage(null);
        setError(null);
        setPhase('ready');
      });
      return;
    }
    void Promise.resolve().then(() => load({ tab, query: searchQuery }, 1, true));
  }, [tab, searchQuery, load]);

  const switchTab = (next: RepoTab) => {
    if (next === tab) {
      return;
    }
    setTab(next);
    setItems([]);
    setNextPage(null);
    if (next !== 'search') {
      // 离开搜索时保留输入框内容，但清掉生效的查询词
      setSearchQuery('');
      setSearchInput('');
    }
  };

  const toggleStar = async (repo: RemoteRepo) => {
    const starred = tab === 'starred';
    setBusyRepo(repo.fullName);
    try {
      await repoRemoteStar(HOST, repo.owner, repo.name, !starred);
      if (starred) {
        // 星标页里"取消"后从列表移除；其他页无法知道旧状态，保持原位
        setItems((current) => current.filter((item) => item.fullName !== repo.fullName));
      }
      pushToast({
        tone: 'success',
        title: t(starred ? 'github.repos.unstarToast' : 'github.repos.starToast', {
          fullName: repo.fullName,
        }),
      });
    } catch (raw) {
      show(raw);
    } finally {
      setBusyRepo(null);
    }
  };

  const fork = async (repo: RemoteRepo) => {
    setBusyRepo(repo.fullName);
    try {
      const copy = await repoRemoteFork(HOST, repo.owner, repo.name);
      pushToast({
        tone: 'success',
        title: t('github.repos.forkToast', { fullName: copy.fullName }),
      });
    } catch (raw) {
      show(raw);
    } finally {
      setBusyRepo(null);
    }
  };

  const copyUrl = (repo: RemoteRepo) => {
    void navigator.clipboard?.writeText(repo.htmlUrl).catch(() => undefined);
    setCopiedRepo(repo.fullName);
    window.setTimeout(() => setCopiedRepo(null), 1500);
  };

  const authRequired = error?.code === 'AUTH_REQUIRED';

  return (
    <section className="flex flex-col gap-3" data-testid="remote-repos-page">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.githubRepos.title')}</h1>
        <p className="text-13 text-fg-muted">{t('pages.githubRepos.description')}</p>
      </header>

      <div
        role="tablist"
        aria-label={t('github.repos.tablistLabel')}
        className="flex flex-wrap gap-2"
      >
        {TABS.map(({ key, labelKey }) => (
          <Button
            key={key}
            type="button"
            variant={tab === key ? 'primary' : 'secondary'}
            aria-pressed={tab === key}
            onClick={() => switchTab(key)}
            data-testid={`repos-tab-${key}`}
          >
            {t(labelKey)}
          </Button>
        ))}
      </div>

      {tab === 'search' ? (
        <form
          className="flex gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            setSearchQuery(searchInput.trim());
          }}
        >
          <Input
            value={searchInput}
            onChange={(event) => setSearchInput(event.target.value)}
            placeholder={t('github.repos.searchPlaceholder')}
            aria-label={t('github.repos.searchPlaceholder')}
            data-testid="repos-search-input"
          />
          <Button type="submit" variant="secondary" data-testid="repos-search-go">
            {t('github.repos.searchGo')}
          </Button>
        </form>
      ) : null}

      {phase === 'error' && error !== null && !authRequired ? (
        <ErrorState
          title={t(`errors.${error.code}.title`)}
          hint={t('github.repos.listErrorHint')}
          onRetry={() => void load({ tab, query: searchQuery }, 1, true)}
          retryLabel={t('github.repos.retry')}
          retryLoading={false}
        />
      ) : null}

      {phase === 'error' && authRequired ? (
        <ErrorState
          title={t('github.repos.signInRequired')}
          hint={t('github.repos.signInHint')}
          actions={
            <Button
              type="button"
              onClick={() => void navigate('/settings/github')}
              data-testid="repos-sign-in-go"
            >
              {t('github.repos.signInGo')}
            </Button>
          }
        />
      ) : null}

      {phase === 'loading' ? (
        <p className="text-13 text-fg-subtle" data-testid="repos-loading">
          {t('github.repos.loading')}
        </p>
      ) : null}

      {phase === 'ready' && items.length === 0 ? (
        <p className="text-13 text-fg-subtle" data-testid="repos-empty">
          {tab === 'search' && searchQuery === ''
            ? t('github.repos.searchHint')
            : t('github.repos.empty')}
        </p>
      ) : null}

      <ul className="flex flex-col gap-2" data-testid="repos-items">
        {items.map((repo) => (
          <li
            key={`${repo.id}-${repo.fullName}`}
            className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-line bg-surface px-3 py-2"
            data-testid="repos-item"
          >
            <div className="flex min-w-0 flex-col">
              <span className="flex items-center gap-2 text-13 font-medium">
                <span className="truncate">{repo.fullName}</span>
                {repo.private ? (
                  <span className="rounded-sm border border-line px-1.5 py-0.5 text-11 text-fg-subtle">
                    {t('github.repos.privateBadge')}
                  </span>
                ) : null}
                {repo.fork ? (
                  <span className="rounded-sm border border-line px-1.5 py-0.5 text-11 text-fg-subtle">
                    {t('github.repos.forkBadge')}
                  </span>
                ) : null}
              </span>
              {repo.description !== undefined && repo.description !== '' ? (
                <span className="truncate text-12 text-fg-muted">{repo.description}</span>
              ) : null}
              <span className="text-12 text-fg-subtle">
                {t('github.repos.stars', { count: repo.stars })}
                {repo.defaultBranch !== undefined
                  ? ` · ${t('github.repos.defaultBranch', { branch: repo.defaultBranch })}`
                  : ''}
              </span>
            </div>
            <div className="flex shrink-0 gap-1">
              <Button
                type="button"
                variant="secondary"
                disabled={busyRepo !== null}
                onClick={() => void toggleStar(repo)}
                data-testid={`repos-star-${repo.name}`}
              >
                {tab === 'starred' ? t('github.repos.unstarAction') : t('github.repos.starAction')}
              </Button>
              <Button
                type="button"
                variant="secondary"
                disabled={busyRepo !== null}
                onClick={() => void fork(repo)}
                data-testid={`repos-fork-${repo.name}`}
              >
                {t('github.repos.forkAction')}
              </Button>
              <Button
                type="button"
                variant="ghost"
                onClick={() => setReadmeRepo(repo)}
                data-testid={`repos-readme-${repo.name}`}
              >
                {t('github.repos.readmeAction')}
              </Button>
              <Button
                type="button"
                variant="ghost"
                onClick={() => copyUrl(repo)}
                data-testid={`repos-copy-${repo.name}`}
              >
                {copiedRepo === repo.fullName
                  ? t('github.repos.copied')
                  : t('github.repos.copyUrl')}
              </Button>
            </div>
          </li>
        ))}
      </ul>

      <ReadmeDialog
        repo={readmeRepo}
        onOpenChange={(open) => setReadmeRepo(open ? readmeRepo : null)}
      />

      {nextPage !== null && phase === 'ready' ? (
        <Button
          type="button"
          variant="secondary"
          disabled={loadingMore}
          onClick={() => void load({ tab, query: searchQuery }, nextPage, false)}
          data-testid="repos-load-more"
        >
          {t('github.repos.loadMore')}
        </Button>
      ) : null}
    </section>
  );
}
