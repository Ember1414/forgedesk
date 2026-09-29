/**
 * reflog 列表（T2.8）。
 *
 * # 为什么"恢复"是建新分支
 *
 * reflog 里的每一条都还指向一个真实存在的提交（在 `gc` 回收之前）。把选中那条
 * 恢复成一个**新分支**不移动任何现有引用——不可能因此丢东西。"把当前分支重置过去"
 * 才是危险的那条路，它由 `HistoryOpsPanel` 的重置动作（带计划预览）提供。
 */
import { useState } from 'react';

import { useMutation, useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';

import { useAppError } from '@/lib/errors';
import { gitReflog, gitReflogCreateBranch } from '@/lib/ipc';
import { reflogKey } from '@/lib/queryKeys';

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/ui/components/alert-dialog';
import { Button } from '@/ui/components/button';
import { Input } from '@/ui/components/input';

export function ReflogList() {
  const params = useParams();
  const repoId = Number(params.repoId);
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const [prompt, setPrompt] = useState<{ index: number; oid: string } | null>(null);
  const [name, setName] = useState('');

  const reflog = useQuery({
    queryKey: reflogKey(repoId),
    queryFn: () => gitReflog(repoId, 20),
    enabled: Number.isFinite(repoId) && repoId > 0,
  });

  const recover = useMutation({
    mutationFn: (input: { index: number; branchName: string }) =>
      gitReflogCreateBranch(repoId, input.index, input.branchName),
    onSuccess: () => {
      setPrompt(null);
      setName('');
    },
    onError: show,
  });

  const entries = reflog.data ?? [];

  return (
    <div className="flex flex-col gap-1">
      <h3 className="text-13 font-medium">{t('historyOps.reflog.title')}</h3>

      {reflog.isError ? (
        <p className="text-12 text-fg-subtle" data-testid="reflog-unavailable">
          {t('historyOps.reflog.unavailable')}
        </p>
      ) : reflog.data === undefined ? null : entries.length === 0 ? (
        <p className="text-12 text-fg-subtle">{t('historyOps.reflog.empty')}</p>
      ) : (
        <ul className="flex flex-col gap-0.5" data-testid="reflog-list">
          {entries.map((entry) => (
            <li key={`${entry.index}-${entry.oid}`} className="flex items-center gap-2 text-12">
              <span className="font-mono text-fg-subtle">{entry.oid.slice(0, 7)}</span>
              <span className="flex-1 truncate">{entry.message}</span>
              <Button
                variant="ghost"
                size="sm"
                disabled={recover.isPending}
                onClick={() => {
                  setPrompt({ index: entry.index, oid: entry.oid });
                  setName('');
                }}
                data-testid={`reflog-recover-${entry.index}`}
              >
                {t('historyOps.reflog.recover')}
              </Button>
            </li>
          ))}
        </ul>
      )}

      <AlertDialog
        open={prompt !== null}
        onOpenChange={(next) => {
          if (!next) {
            setPrompt(null);
          }
        }}
      >
        <AlertDialogContent
          impact={t('historyOps.reflog.recoverImpact')}
          impactLabel={t('historyOps.reflog.recoverImpactLabel')}
          data-testid="reflog-recover-dialog"
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t('historyOps.reflog.recoverTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('historyOps.reflog.recoverHint', { oid: prompt?.oid.slice(0, 7) ?? '' })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <Input
            value={name}
            placeholder={t('historyOps.reflog.namePlaceholder')}
            onChange={(event) => {
              setName(event.target.value);
            }}
            data-testid="reflog-branch-name"
          />
          <AlertDialogFooter>
            <AlertDialogCancel>{t('common:actions.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              disabled={name.trim() === ''}
              onClick={() =>
                prompt !== null && recover.mutate({ index: prompt.index, branchName: name.trim() })
              }
              data-testid="reflog-recover-confirm"
            >
              {t('historyOps.reflog.recoverConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
