/**
 * 仓库外壳上的分支切换器（T2.5）。
 *
 * # 与 RepoSwitcher（仓库切换器）的分工
 *
 * 仓库切换器在 AppShell 顶栏，换的是"哪个仓库"；本组件在仓库外壳的页签行，
 * 换的是"这个仓库的哪个分支"。切换动作与分支页共用 `gitBranchSwitch`
 * （三策略语义在后端：不干净时 stash 自动恢复；这里缺省用 stash 策略，
 * 失败时弹出后端的结构化错误——用户在分支页可以走 Force / 取消的显式路径）。
 *
 * # 快速创建
 *
 * 下拉底部一个"创建分支"输入项：输入新名字回车即创建并切换
 * （`checkout=true` 的干净切换）。这是任务书"下拉 + 搜索 + 快速创建"的三合一。
 */
import { useMemo, useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, ChevronDown, GitBranchPlus } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';

import { useAppError } from '@/lib/errors';
import { gitBranchCreate, gitBranchSwitch } from '@/lib/ipc/branches';
import { gitBranchList } from '@/lib/ipc/history';
import type { Branch } from '@/lib/ipc/history';
import { BRANCHES_QUERY_KEY, LOG_QUERY_KEY } from '@/lib/queryKeys';
import { cn } from '@/lib/utils';

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

export function BranchSwitcher() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const queryClient = useQueryClient();
  const { show } = useAppError();
  const [filter, setFilter] = useState('');
  const [creating, setCreating] = useState('');
  const [open, setOpen] = useState(false);

  const branchesQuery = useQuery({
    queryKey: [BRANCHES_QUERY_KEY, repoId],
    queryFn: () => gitBranchList(repoId),
    enabled: Number.isFinite(repoId),
    staleTime: 5_000,
  });

  const invalidate = useMemo(
    () => () => {
      void queryClient.invalidateQueries({ queryKey: [BRANCHES_QUERY_KEY, repoId] });
      void queryClient.invalidateQueries({ queryKey: [LOG_QUERY_KEY, repoId] });
    },
    [queryClient, repoId],
  );

  const switchMutation = useMutation({
    mutationFn: (name: string) => gitBranchSwitch(repoId, name, 'stash'),
    onSuccess: () => {
      invalidate();
      setOpen(false);
      setFilter('');
    },
    onError: show,
  });
  const createMutation = useMutation({
    mutationFn: (name: string) =>
      gitBranchCreate(repoId, { name, startPoint: null, checkout: true }),
    onSuccess: () => {
      invalidate();
      setOpen(false);
      setCreating('');
      setFilter('');
    },
    onError: show,
  });

  if (!Number.isFinite(repoId)) {
    return null;
  }

  const branches = branchesQuery.data ?? [];
  const current = branches.find((branch) => branch.isHead);
  const needle = filter.trim().toLowerCase();
  const visible = branches.filter(
    (branch) => needle === '' || branch.name.toLowerCase().includes(needle),
  );

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="secondary"
          size="sm"
          aria-label={`${t('branches.switcherLabel')}: ${current?.name ?? '—'}`}
          className="max-w-48 justify-start"
          data-testid="branch-switcher"
        >
          <span className="truncate font-mono text-12">{current?.name ?? '—'}</span>
          <ChevronDown aria-hidden="true" className="size-3 shrink-0 text-fg-subtle" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <DropdownMenuLabel>{t('branches.switcherLabel')}</DropdownMenuLabel>
        <div className="px-2 pb-1">
          <input
            value={filter}
            onChange={(event) => {
              setFilter(event.target.value);
            }}
            placeholder={t('branches.searchPlaceholder')}
            aria-label={t('branches.searchPlaceholder')}
            className="h-7 w-full rounded-md border border-line bg-surface px-2 text-12"
            data-testid="branch-switcher-filter"
          />
        </div>
        {branchesQuery.isPending ? (
          <div className="flex flex-col gap-1.5 px-2 py-1.5">
            <Skeleton className="h-6" />
            <Skeleton className="h-6" />
          </div>
        ) : (
          <div className="max-h-56 overflow-y-auto">
            {visible.map((branch) => (
              <BranchItem
                key={branch.name}
                branch={branch}
                disabled={switchMutation.isPending}
                onPick={() => switchMutation.mutate(branch.name)}
              />
            ))}
            {visible.length === 0 ? (
              <p className="px-2 py-1.5 text-12 text-fg-subtle">{t('branches.switcherNoMatch')}</p>
            ) : null}
          </div>
        )}
        <DropdownMenuSeparator />
        <div className="flex items-center gap-1 px-2 pb-1.5">
          <GitBranchPlus aria-hidden="true" className="size-3.5 shrink-0 text-fg-subtle" />
          <input
            value={creating}
            onChange={(event) => {
              setCreating(event.target.value);
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && creating.trim() !== '') {
                createMutation.mutate(creating.trim());
              }
            }}
            placeholder={t('branches.quickCreate')}
            aria-label={t('branches.quickCreate')}
            className="h-7 min-w-0 flex-1 rounded-md border border-line bg-surface px-2 text-12"
            data-testid="branch-switcher-create"
          />
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** 下拉里的一行：当前分支打勾；点击 = stash 策略切换。 */
function BranchItem({
  branch,
  disabled,
  onPick,
}: {
  readonly branch: Branch;
  readonly disabled: boolean;
  readonly onPick: () => void;
}) {
  const { t } = useTranslation('shell');
  return (
    <DropdownMenuItem
      disabled={disabled || branch.isHead}
      onSelect={onPick}
      className={cn('data-[highlighted]:bg-surface-sunken', branch.isHead && 'opacity-80')}
      data-testid={`branch-switcher-item-${branch.name}`}
    >
      <span className="min-w-0 flex-1 truncate font-mono text-12">{branch.name}</span>
      {branch.isHead ? (
        <Check
          aria-hidden="true"
          className="size-3.5 shrink-0 text-brand"
          aria-label={t('branches.current')}
        />
      ) : null}
    </DropdownMenuItem>
  );
}
