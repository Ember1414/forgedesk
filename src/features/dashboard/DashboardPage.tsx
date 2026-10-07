import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

import { AddRepoCard } from '@/features/dashboard/AddRepoCard';
import { repoIdOf, useRecentRepos } from '@/features/repo/recentRepos';
import { useJobStore } from '@/stores/jobStore';
import { useUiStore } from '@/stores/uiStore';
import { Skeleton } from '@/ui/components/skeleton';

/** 仪表盘上展示几条（缓存里是 20 条，这里只是少显示几个）。 */
const DASHBOARD_RECENT_LIMIT = 12;

/**
 * 仪表盘。
 *
 * M0 时这里读的是**占位数据**（`PLACEHOLDER_RECENT_REPOS`），当时没有真实数据源。
 * M1 之后不再是：最近仓库来自 `repo_recent_list`（T1.3 落地的本地记录），
 * 点进去会设置当前仓库并跳到工作区。
 *
 * 为什么这件事必须在 M1 收尾改掉：占位数据看起来与真实数据完全一样，
 * 用户第一眼看到的是四个不存在的仓库路径，点进去必然失败——这比空白页更难判断。
 * 空态说清"还没有打开过仓库、去哪里打开"，是唯一诚实的画法。
 *
 * 真正的聚合（PR/Issue/流水线摘要、本地仓库统计）仍然属于 T4.11。
 */
export function DashboardPage() {
  const { t } = useTranslation('shell');
  const navigate = useNavigate();
  const jobs = useJobStore((state) => state.jobs);
  const setCurrentRepoId = useUiStore((state) => state.setCurrentRepoId);

  const { repos, isPending } = useRecentRepos();
  const recent = repos.slice(0, DASHBOARD_RECENT_LIMIT);

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.dashboard.title')}</h1>
        <p className="text-13 text-fg-muted">{t('pages.dashboard.description')}</p>
      </header>

      {/* GIT-01/02/03：打开 / 克隆 / 初始化。放在最近列表之前——
          首次启动时它是唯一能用的东西 */}
      <AddRepoCard />

      <div className="grid gap-4 lg:grid-cols-2">
        <article className="rounded-lg border border-line bg-surface p-4">
          <h2 className="text-14 font-medium">{t('dashboard.jobsTitle')}</h2>
          {jobs.length === 0 ? (
            <p className="mt-2 text-12 text-fg-subtle">{t('dashboard.jobsEmpty')}</p>
          ) : (
            <ul className="mt-2 flex flex-col gap-1.5">
              {jobs.map((job) => (
                <li key={job.id} className="flex items-center justify-between gap-2 text-13">
                  <span className="truncate">{job.label}</span>
                  <span className="font-mono text-12 text-fg-subtle">
                    {job.progress === null ? job.state : `${Math.round(job.progress * 100)}%`}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </article>

        <article className="rounded-lg border border-line bg-surface p-4">
          <h2 className="text-14 font-medium">{t('dashboard.recentTitle')}</h2>
          {isPending ? (
            <div className="mt-2 flex flex-col gap-2">
              <Skeleton className="h-9" />
              <Skeleton className="h-9" />
            </div>
          ) : recent.length === 0 ? (
            <div className="mt-2 flex flex-col gap-1">
              <p className="text-13 text-fg">{t('dashboard.recentEmpty')}</p>
              <p className="text-12 text-fg-subtle">{t('dashboard.recentEmptyHint')}</p>
            </div>
          ) : (
            <ul className="mt-2 flex flex-col gap-1">
              {recent.map((repo) => (
                <li key={repo.id}>
                  <button
                    type="button"
                    onClick={() => {
                      // 路由参数是字符串，而存储层的记录 id 是数字：
                      // 转换只在 `repoIdOf` 里做一处，别处不要再各自转换
                      const id = repoIdOf(repo);
                      setCurrentRepoId(id);
                      void navigate(`/repo/${id}/status`);
                    }}
                    className="fd-transition flex w-full items-center justify-between gap-2 rounded-sm px-2 py-1.5 text-left hover:bg-surface-sunken"
                  >
                    <span className="min-w-0">
                      <span className="block truncate text-13 font-medium">{repo.name}</span>
                      <span className="block truncate font-mono text-12 text-fg-subtle">
                        {repo.path}
                      </span>
                    </span>
                    <span className="shrink-0 text-12 text-brand">{t('dashboard.openRepo')}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
          <p className="mt-2 text-12 text-fg-subtle">{t('dashboard.recentHint')}</p>
        </article>
      </div>
    </section>
  );
}
