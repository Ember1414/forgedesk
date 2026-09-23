import { useEffect, useRef, useState } from 'react';

import { ChevronDown } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

import { PLACEHOLDER_RECENT_REPOS, findRecentRepo } from '@/features/repo/recentRepos';
import { cn } from '@/lib/utils';
import { useUiStore } from '@/stores/uiStore';

/**
 * 顶栏的仓库切换器。
 *
 * 键盘与焦点约定（T0.4 验收项）：
 *   - 触发按钮带 aria-haspopup / aria-expanded，屏幕阅读器能读出展开状态；
 *   - 菜单用 role="menu" + role="menuitem"，Tab 可进入；
 *   - **Esc 关闭菜单并把焦点还给触发按钮**（不能把焦点丢在已被移除的节点上）；
 *   - 点击菜单外部关闭。
 *
 * 这里刻意没有引入 Radix（那是 T0.5 组件库的范围）：本组件只覆盖"一个下拉菜单"，
 * T0.5 落地 DropdownMenu 后应由它替换，避免两份实现长期并存。
 */
export function RepoSwitcher() {
  const { t } = useTranslation('shell');
  const navigate = useNavigate();
  const currentRepoId = useUiStore((state) => state.currentRepoId);
  const setCurrentRepoId = useUiStore((state) => state.setCurrentRepoId);

  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const currentRepo = findRecentRepo(currentRepoId);

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    function handlePointerDown(event: MouseEvent): void {
      if (!containerRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener('mousedown', handlePointerDown);
    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
    };
  }, [open]);

  function closeAndRestoreFocus(): void {
    setOpen(false);
    triggerRef.current?.focus();
  }

  function selectRepo(repoId: string): void {
    setCurrentRepoId(repoId);
    setOpen(false);
    // 选定仓库后直接进入工作区：这是打开仓库后最可能的下一步
    void navigate(`/repo/${repoId}/status`);
  }

  return (
    <div
      ref={containerRef}
      className="relative"
      onKeyDown={(event) => {
        if (event.key === 'Escape' && open) {
          event.stopPropagation();
          closeAndRestoreFocus();
        }
      }}
    >
      <button
        ref={triggerRef}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t('titleBar.repoSwitcher.label')}
        onClick={() => {
          setOpen((previous) => !previous);
        }}
        className={cn(
          'fd-transition flex h-8 max-w-[280px] items-center gap-2 rounded-md border border-line bg-surface px-2.5 text-13',
          'text-fg hover:border-line-strong hover:bg-surface-sunken',
        )}
      >
        <span className="truncate font-medium">
          {currentRepo?.name ?? t('titleBar.repoSwitcher.none')}
        </span>
        <ChevronDown aria-hidden="true" className="size-3.5 shrink-0 text-fg-subtle" />
      </button>

      {open ? (
        <div className="absolute left-0 top-10 z-40 w-80 rounded-lg border border-line bg-surface-raised p-2 shadow-lg">
          <p
            className="px-2 pb-1 pt-0.5 text-12 font-medium text-fg-subtle"
            id="repo-switcher-title"
          >
            {t('titleBar.repoSwitcher.menuLabel')}
          </p>
          <ul role="menu" aria-labelledby="repo-switcher-title" className="flex flex-col">
            {PLACEHOLDER_RECENT_REPOS.map((repo) => (
              <li key={repo.id}>
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    selectRepo(repo.id);
                  }}
                  className={cn(
                    'fd-transition flex w-full flex-col items-start gap-0.5 rounded-sm px-2 py-1.5 text-left',
                    'hover:bg-surface-sunken',
                    repo.id === currentRepoId && 'bg-brand-subtle',
                  )}
                >
                  <span className="text-13 font-medium">{repo.name}</span>
                  <span className="w-full truncate font-mono text-12 text-fg-subtle">
                    {repo.path}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          <p className="mt-1 border-t border-line px-2 pt-2 text-12 text-fg-subtle">
            {t('titleBar.repoSwitcher.placeholderNote')}
          </p>
        </div>
      ) : null}
    </div>
  );
}
