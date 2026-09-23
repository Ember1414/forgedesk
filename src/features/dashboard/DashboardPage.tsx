import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

import { PLACEHOLDER_RECENT_REPOS } from '@/features/repo/recentRepos';
import { useJobStore } from '@/stores/jobStore';
import { useUiStore } from '@/stores/uiStore';

/**
 * 仪表盘。
 *
 * M0 的定位：不做聚合卡片，只把**已经存在的数据源**接起来，
 * 用来证明"外壳 + store + 路由"三者能协同工作：
 *   · 后台任务面板读 jobStore（空态是当前的正常状态）
 *   · 最近仓库面板读占位数据，点进去会设置当前仓库并跳到工作区
 *
 * 真正的仪表盘聚合（T4.11）会加入 PR/Issue/流水线摘要与本地仓库统计。
 */
export function DashboardPage() {
  const { t } = useTranslation('shell');
  const navigate = useNavigate();
  const jobs = useJobStore((state) => state.jobs);
  const setCurrentRepoId = useUiStore((state) => state.setCurrentRepoId);

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <h1 className="text-20 font-semibold tracking-tight">{t('pages.dashboard.title')}</h1>
          <span className="rounded-sm border border-line bg-surface-sunken px-2 py-0.5 font-mono text-12 text-fg-subtle">
            {t('placeholder.planned')} T4.11
          </span>
        </div>
        <p className="text-13 text-fg-muted">{t('pages.dashboard.description')}</p>
      </header>

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
          <ul className="mt-2 flex flex-col gap-1">
            {PLACEHOLDER_RECENT_REPOS.map((repo) => (
              <li key={repo.id}>
                <button
                  type="button"
                  onClick={() => {
                    setCurrentRepoId(repo.id);
                    void navigate(`/repo/${repo.id}/status`);
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
          <p className="mt-2 text-12 text-fg-subtle">{t('dashboard.recentHint')}</p>
        </article>
      </div>

      <p className="rounded-md border border-dashed border-line bg-surface p-4 text-12 text-fg-subtle">
        {t('placeholder.note')}
      </p>
    </section>
  );
}
