import { ChevronDown } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

import { repoIdOf, useCurrentRepo, useRecentRepos } from '@/features/repo/recentRepos';
import { Button } from '@/ui/components/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/ui/components/dropdown-menu';
import { Skeleton } from '@/ui/components/skeleton';
import { useUiStore } from '@/stores/uiStore';

/**
 * 顶栏的仓库切换器。
 *
 * T0.5 起基于组件库的 DropdownMenu（键盘行为、焦点管理由 Radix 保证）；
 * **M1 收尾时换成真实数据**：列表来自 `repo_recent_list`，与仪表盘、
 * 状态栏、仓库页标题读同一份缓存（见 `features/repo/recentRepos`）。
 *
 * 曾经这里渲染的是写死的示例仓库，并配一句"这是占位数据"的说明。
 * 那种画法比空态更糟：示例仓库看起来与真实仓库完全一样，点进去必然失败，
 * 用户会以为应用坏了。现在没有数据就如实说"还没有打开过仓库"。
 *
 * 无障碍名称：触发按钮的可访问名称同时包含"当前仓库"与仓库名，
 * 因为读屏软件会以 aria-label 取代可见文本，只说"未打开仓库"会丢掉控件用途。
 */
export function RepoSwitcher() {
  const { t } = useTranslation('shell');
  const navigate = useNavigate();
  const setCurrentRepoId = useUiStore((state) => state.setCurrentRepoId);

  const { repos, isPending } = useRecentRepos();
  const currentRepo = useCurrentRepo();
  const currentName = currentRepo?.name ?? t('titleBar.repoSwitcher.none');

  function selectRepo(repoId: string): void {
    setCurrentRepoId(repoId);
    // 选定仓库后直接进入工作区：这是打开仓库后最可能的下一步
    void navigate(`/repo/${repoId}/status`);
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="secondary"
          aria-label={`${t('titleBar.repoSwitcher.label')}: ${currentName}`}
          className="max-w-[280px] justify-start"
        >
          <span className="truncate font-medium">{currentName}</span>
          <ChevronDown aria-hidden="true" className="size-3.5 shrink-0 text-fg-subtle" />
        </Button>
      </DropdownMenuTrigger>

      {/*
        不要在这里传 aria-label：Radix 会把菜单的 aria-labelledby 指向触发器，
        按 ARIA 菜单模式，菜单以它的触发按钮命名（"当前仓库: ForgeDesk"），
        aria-label 会被覆盖，写了反而让人误以为它在生效。
      */}
      <DropdownMenuContent align="start" className="w-80">
        <DropdownMenuLabel>{t('titleBar.repoSwitcher.menuLabel')}</DropdownMenuLabel>

        {isPending ? (
          <div className="flex flex-col gap-2 px-2 py-1.5">
            <Skeleton className="h-8" />
            <Skeleton className="h-8" />
          </div>
        ) : repos.length === 0 ? (
          <p className="px-2 py-1.5 text-12 text-fg-subtle">{t('titleBar.repoSwitcher.empty')}</p>
        ) : (
          repos.map((repo) => {
            const id = repoIdOf(repo);
            return (
              <DropdownMenuItem
                key={id}
                onSelect={() => {
                  selectRepo(id);
                }}
                className="data-[highlighted]:bg-surface-sunken"
              >
                <span className="flex min-w-0 flex-col items-start gap-0.5">
                  <span className="flex items-center gap-1.5">
                    <span className="truncate text-13 font-medium">{repo.name}</span>
                    {repo.isOpen ? (
                      <span className="shrink-0 rounded-sm bg-surface-sunken px-1 text-11 text-fg-subtle">
                        {t('titleBar.repoSwitcher.opened')}
                      </span>
                    ) : null}
                  </span>
                  <span className="w-full truncate font-mono text-12 text-fg-subtle">
                    {repo.path}
                  </span>
                </span>
              </DropdownMenuItem>
            );
          })
        )}

        <DropdownMenuSeparator />
        {/* 打开/克隆/初始化的入口还没做：如实说明，而不是给一个点了必然失败的菜单项 */}
        <p className="px-2 py-1 text-12 text-fg-subtle">{t('titleBar.repoSwitcher.openPending')}</p>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
