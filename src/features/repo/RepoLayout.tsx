import { useTranslation } from 'react-i18next';
import { NavLink, Outlet, useParams } from 'react-router-dom';

import { DETAIL_PANEL_POSITIONS, useUiStore } from '@/stores/uiStore';
import type { DetailPanelPosition } from '@/stores/uiStore';
import { useRepoById } from '@/features/repo/recentRepos';
import { BranchSwitcher } from '@/features/branches/BranchSwitcher';
import { SyncBar } from '@/features/sync/SyncBar';
import { MergeBanner } from '@/features/branches/MergeBanner';
import { CommitDetailPanel } from '@/features/history/CommitDetailPanel';
import { ToggleGroup } from '@/ui/components/toggle-group';
import { cn } from '@/lib/utils';

/**
 * 仓库级页面的公共外壳：仓库标题 + 仓库内标签导航 + 详情面板 + 子路由出口。
 *
 * 设计决策（原创）：
 *   把仓库内的五个页面（工作区/历史/分支/冲突/终端）做成**同一层的标签**，
 *   而不是侧栏的五个平级项——它们共享同一个仓库上下文，标签形态能明确表达
 *   "我在同一个仓库里换视角"，而侧栏表达的是"换一个完全不同的区域"。
 *
 * 详情面板位置由 uiStore 控制（右侧 / 底部 / 隐藏），
 * 让"看变更的同时能看 diff""终端要占满宽度"这类不同工作方式可以各取所需。
 */
const REPO_TABS = [
  { segment: 'status', labelKey: 'items.status' },
  { segment: 'commit', labelKey: 'items.commit' },
  { segment: 'snapshots', labelKey: 'items.snapshots' },
  { segment: 'history', labelKey: 'items.history' },
  { segment: 'branches', labelKey: 'items.branches' },
  { segment: 'conflict', labelKey: 'items.conflict' },
  { segment: 'terminal', labelKey: 'items.terminal' },
  { segment: 'settings', labelKey: 'items.repoSettings' },
] as const;

export function RepoLayout() {
  const { t } = useTranslation('shell');
  const { repoId } = useParams();
  const detailPanel = useUiStore((state) => state.detailPanel);
  const setDetailPanel = useUiStore((state) => state.setDetailPanel);

  // 名称与路径来自本地记录（与顶栏切换器同一份缓存）；查不到就退回路由段本身
  const repo = useRepoById(repoId);

  return (
    <section className="flex h-full flex-col gap-3">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1.5">
          <div className="flex flex-wrap items-baseline gap-2">
            <h1 className="text-20 font-semibold tracking-tight">{repo?.name ?? repoId}</h1>
            <span className="truncate font-mono text-12 text-fg-subtle">{repo?.path ?? ''}</span>
          </div>
          <nav aria-label={t('tabs.repo')} className="flex flex-wrap gap-1">
            {REPO_TABS.map((tab) => (
              <NavLink
                key={tab.segment}
                to={tab.segment}
                className={({ isActive }) =>
                  cn(
                    'fd-transition rounded-md px-2.5 py-1 text-13',
                    isActive
                      ? 'bg-brand-subtle font-medium text-brand'
                      : 'text-fg-muted hover:bg-surface-sunken hover:text-fg',
                  )
                }
              >
                {t(tab.labelKey)}
              </NavLink>
            ))}
          </nav>
        </div>

        {/* 分支切换器（T2.5）：显示当前分支，下拉可搜索 + 快速创建。
            切换动作与分支页共用同一命令（三策略语义在后端）。 */}
        <BranchSwitcher />

        <ToggleGroup
          label={t('panel.label')}
          value={detailPanel}
          options={DETAIL_PANEL_POSITIONS.map((position) => ({
            value: position,
            label: t(`panel.${position}`),
          }))}
          onValueChange={(next) => {
            // 选项值来自 DETAIL_PANEL_POSITIONS，这里的收窄是编译期保证的
            setDetailPanel(next as DetailPanelPosition);
          }}
        />
      </header>

      {/*
        同步条（T2.6）：Fetch / Pull / Push 与 ahead-behind 常驻在仓库外壳上，
        让"远端同步"不必先跳到某个页面才能做——它是仓库级动作，不是某个页签的功能。
      */}
      <MergeBanner />
      <SyncBar />

      <div
        className={cn(
          'flex min-h-0 flex-1',
          detailPanel === 'bottom' ? 'flex-col gap-3' : 'flex-row gap-3',
        )}
      >
        <div className="min-h-0 min-w-0 flex-1 overflow-auto">
          <Outlet />
        </div>

        {detailPanel !== 'hidden' ? (
          <aside
            aria-label={t('panel.title')}
            className={cn(
              'shrink-0 rounded-lg border border-line bg-surface p-3',
              detailPanel === 'right' ? 'w-72' : 'h-28',
            )}
          >
            {/*
              详情位由提交详情面板接管（T2.2）：选中历史页的某个提交即在此展示元数据。
              没有选中提交时退回原来的占位说明——面板挂在所有仓库子页共享的外壳上，
              因此必须对"当前不在历史页 / 没选提交"这两种情况给出合理默认。
            */}
            <CommitDetailPanel
              repoId={Number(repoId)}
              fallback={
                <>
                  <h2 className="text-13 font-medium">{t('panel.title')}</h2>
                  <p className="mt-1 text-12 text-fg-subtle">{t('panel.placeholder')}</p>
                </>
              }
            />
          </aside>
        ) : null}
      </div>
    </section>
  );
}
