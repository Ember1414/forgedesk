import { useTranslation } from 'react-i18next';

import { findRecentRepo } from '@/features/repo/recentRepos';
import { countActiveJobs, useJobStore } from '@/stores/jobStore';
import { useUiStore } from '@/stores/uiStore';

/**
 * 底部状态栏：当前仓库 / 分支 / 操作状态 / 后台任务。
 *
 * 设计决策：状态栏承载"随时可确认的上下文"，因此四段信息的排列顺序是
 * 「我在哪（仓库）→ 在哪个分支 → 现在在做什么（操作状态）→ 还有什么在跑（任务数）」，
 * 从左到右由静态到动态。后台任务数放在最右，因为它变化最频繁，
 * 放在固定位置（而不是会因文案长度跳动的位置）能减少视觉噪音。
 *
 * 数据来源：
 *   - 仓库与分支：M0 来自占位数据；分支名将在 T1.x 由 git status 提供。
 *   - 操作状态：M0 恒为"空闲"；M1 接入 JobRunner 后反映真实的排队/执行状态。
 *   - 任务数：来自 jobStore（后端事件接入后自动生效）。
 */
export function StatusBar() {
  const { t } = useTranslation('shell');
  const currentRepoId = useUiStore((state) => state.currentRepoId);
  const jobs = useJobStore((state) => state.jobs);

  const repo = findRecentRepo(currentRepoId);
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
