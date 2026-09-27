import { useTranslation } from 'react-i18next';

import { useCurrentRepo } from '@/features/repo/recentRepos';
import { countActiveJobs, useJobStore } from '@/stores/jobStore';

/**
 * 底部状态栏：当前仓库 / 分支 / 操作状态 / 后台任务。
 *
 * 设计决策：状态栏承载"随时可确认的上下文"，因此四段信息的排列顺序是
 * 「我在哪（仓库）→ 在哪个分支 → 现在在做什么（操作状态）→ 还有什么在跑（任务数）」，
 * 从左到右由静态到动态。后台任务数放在最右，因为它变化最频繁，
 * 放在固定位置（而不是会因文案长度跳动的位置）能减少视觉噪音。
 *
 * 数据来源：
 *   - 仓库：最近打开列表里的当前仓库（`features/repo/recentRepos`）；
 *   - 分支：记录里的默认分支（真实分支由工作区状态给出，状态栏只做"我在哪个仓库"
 *     这一层提示）；
 *   - 操作状态：恒为"空闲"（真正的进行中操作由仓库页的操作横幅展示）；
 *   - 任务数：来自 jobStore（后端事件接入后自动生效）。
 */
export function StatusBar() {
  const { t } = useTranslation('shell');
  const jobs = useJobStore((state) => state.jobs);

  const repo = useCurrentRepo();
  const activeJobs = countActiveJobs(jobs);

  return (
    <footer
      aria-label={t('statusBar.ariaLabel')}
      className="flex h-7 shrink-0 items-center gap-3 border-t border-line bg-surface px-3 text-12 text-fg-muted"
    >
      <span className="flex min-w-0 items-center gap-1.5">
        <span className="text-fg-subtle">{t('statusBar.repo')}</span>
        <span className="truncate font-medium text-fg">{repo?.name ?? t('statusBar.noRepo')}</span>
      </span>

      <span aria-hidden="true" className="text-line-strong">
        |
      </span>

      <span className="flex items-center gap-1.5">
        <span className="text-fg-subtle">{t('statusBar.branch')}</span>
        <span className="font-mono">{repo?.defaultBranch ?? t('statusBar.branchUnknown')}</span>
      </span>

      {/* 状态栏只到"哪个仓库"这一层：真正的分支与操作状态属于工作区页 */}

      <span className="ml-auto flex items-center gap-1.5">
        <span aria-hidden="true" className="size-1.5 rounded-full bg-success" />
        <span>{t('statusBar.idle')}</span>
      </span>

      <span aria-hidden="true" className="text-line-strong">
        |
      </span>

      <span className="flex items-center gap-1.5">
        <span className="text-fg-subtle">{t('statusBar.jobs')}</span>
        <span className="font-mono">{activeJobs}</span>
      </span>
    </footer>
  );
}
