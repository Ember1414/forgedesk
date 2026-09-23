import { ChevronDown } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

import { PLACEHOLDER_RECENT_REPOS, findRecentRepo } from '@/features/repo/recentRepos';
import { Button } from '@/ui/components/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/ui/components/dropdown-menu';
import { useUiStore } from '@/stores/uiStore';

/**
 * 顶栏的仓库切换器。
 *
 * T0.5 起基于组件库的 DropdownMenu（此前是 T0.4 临时手写的一份菜单）：
 * 键盘行为（↑↓ 移动、Home/End、前缀匹配、Esc 关闭并归还焦点）与焦点管理
 * 全部由 Radix 保证，本组件只负责内容与路由跳转。
 *
 * 无障碍名称：触发按钮的可访问名称同时包含"当前仓库"与仓库名，
 * 因为读屏软件会以 aria-label 取代可见文本，只说"未打开仓库"会丢掉控件用途。
 */
export function RepoSwitcher() {
  const { t } = useTranslation('shell');
  const navigate = useNavigate();
  const currentRepoId = useUiStore((state) => state.currentRepoId);
  const setCurrentRepoId = useUiStore((state) => state.setCurrentRepoId);

  const currentRepo = findRecentRepo(currentRepoId);
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
        按 ARIA 菜单模式，菜单以它的触发按钮命名（"当前仓库: forgedesk"），
        aria-label 会被覆盖，写了反而让人误以为它在生效。
      */}
      <DropdownMenuContent align="start" className="w-80">
        <DropdownMenuLabel>{t('titleBar.repoSwitcher.menuLabel')}</DropdownMenuLabel>

        {PLACEHOLDER_RECENT_REPOS.map((repo) => (
          <DropdownMenuItem
            key={repo.id}
            onSelect={() => {
              selectRepo(repo.id);
            }}
            className="data-[highlighted]:bg-surface-sunken"
          >
            <span className="flex min-w-0 flex-col items-start gap-0.5">
              <span className="text-13 font-medium">{repo.name}</span>
              <span className="w-full truncate font-mono text-12 text-fg-subtle">{repo.path}</span>
            </span>
          </DropdownMenuItem>
        ))}

        <DropdownMenuSeparator />
        <p className="px-2 py-1 text-12 text-fg-subtle">
          {t('titleBar.repoSwitcher.placeholderNote')}
        </p>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
