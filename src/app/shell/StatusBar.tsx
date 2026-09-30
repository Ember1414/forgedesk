import { useQuery } from '@tanstack/react-query';
import { RotateCcw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';

import { useCurrentRepo } from '@/features/repo/recentRepos';
import { operationHistory } from '@/lib/ipc/audit';
import { OPERATION_HISTORY_QUERY_KEY } from '@/lib/queryKeys';
import { countActiveJobs, useJobStore } from '@/stores/jobStore';

/**
 * 最近一次**仍可回滚**的破坏性操作（T3.10 的状态栏指示器）。
 *
 * 只查一条：状态栏不是列表，它只回答"现在有没有一个能回去的点"。
 * 查询条件用 `onlyReversible`（当时确实留了点的记录），再用 `canRollback`
 * 过滤掉锚点已失效的那些——两者缺一不可：前者是历史，后者是现状。
 */
function useLatestRollbackPoint(repoId: number | null) {
  const query = useQuery({
    // key 的前缀必须是 `[OPERATION_HISTORY_QUERY_KEY, repoId]`：`repo:changed`
    // 事件按前缀失效，写成 `[KEY, 'latest', repoId]` 的话，危险操作完成后
    // 这个指示器不会刷新——而它恰恰是"刚做完一次操作"时最该更新的东西
    queryKey: [OPERATION_HISTORY_QUERY_KEY, repoId ?? 0, 'latest'],
    queryFn: () => operationHistory(repoId ?? 0, { onlyReversible: true }, 1, 0),
    enabled: repoId !== null && Number.isFinite(repoId),
    // 状态栏不追实时：10 秒内复用结果，避免每次路由切换都打一次 IPC
    staleTime: 10_000,
  });
  const first = query.data?.entries[0];
  return first !== undefined && first.canRollback ? first : null;
}

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
  const rollbackPoint = useLatestRollbackPoint(repo?.id ?? null);

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

      {/*
        最近可回滚点（T3.10）：本产品最要紧的一句"我现在还回得去"。
        没有可回滚点时它**完全不出现**——状态栏的显眼必须建立在"平时安静"之上，
        否则它会变成一块常年亮着、但没人再看的警示牌。
      */}
      {rollbackPoint === null || rollbackPoint.snapshotId === null ? null : (
        <Link
          to={`/repo/${String(rollbackPoint.repoId)}/operations`}
          className="flex items-center gap-1 rounded-sm border border-brand-subtle bg-brand-subtle px-1.5 text-brand"
          title={t('statusBar.rollbackPointHint')}
          data-testid="status-rollback-point"
        >
          <RotateCcw aria-hidden="true" className="size-3" />
          {t('statusBar.rollbackPoint')}
        </Link>
      )}

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
