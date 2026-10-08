/**
 * 历史操作面板（T2.8）：拣选 / 反转 / 重置到选中提交（reflog 恢复在 `ReflogList`）。
 *
 * # 为什么它读 `graphSelectionStore` 而不是自己选提交
 *
 * 历史页已经有"选中提交"的单一真相（图上点选 / 列表点选都会写进 store）。
 * 面板只是给选中提交加一组动作，另建一套选择状态只会出现"面板指的是 A、
 * 详情面板显示的是 B"的分裂。
 *
 * # 重置的三步
 *
 * 界面上是"选模式 → 看计划 → 输确认词（仅 hard）"：计划里最重要的是
 * `remote.notOnRemote`（有多少提交**远端也没有**）与将被丢弃的改动清单。
 * 确认词来自后端（`confirmationWord`），前端不自己编——执行闸门属于后端，
 * 前端只负责把要求展示出来。
 *
 * # 为什么整块压缩成一行 + reflog 默认折叠（2026-10-08）
 *
 * 这是历史的**次要**面板，主内容是上面的提交图。此前它是"标题 + 提示 + 控件行 +
 * 20 条 reflog"的竖排堆叠（≈650px），在 768/900 高的窗口里会把 `flex-1` 的图画布
 * 挤到 0 高度：用户看到"提交加载了但图上什么都没有，也滚不动"（外层是 h-full，
 * 没有溢出可以滚动）。现在标题/提示/控件在同一行（窄窗口自然换行），reflog 折叠
 * 且展开时自带 max-h 滚动，面板不再参与"抢高度"。
 */
import { useState } from 'react';

import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, RotateCcw, Scissors, Undo2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';

import { useAppError } from '@/lib/errors';
import { gitCherryPick, gitResetExecute, gitResetPrepare, gitRevert } from '@/lib/ipc';
import type { ResetPlan } from '@/lib/ipc';
import { logKeyPrefix, reflogKey } from '@/lib/queryKeys';
import { cn } from '@/lib/utils';

import { useGraphSelectionStore } from '@/features/history/graphSelectionStore';
import { ReflogList } from '@/features/history/ReflogList';

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
import { ToggleGroup } from '@/ui/components/toggle-group';

type ResetMode = ResetPlan['mode'];

export function HistoryOpsPanel() {
  const params = useParams();
  const repoId = Number(params.repoId);
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const queryClient = useQueryClient();

  const detailOid = useGraphSelectionStore((state) => state.detailOid);

  const [plan, setPlan] = useState<ResetPlan | null>(null);
  const [mode, setMode] = useState<ResetMode>('mixed');
  const [confirmation, setConfirmation] = useState('');
  /** reflog 列表默认折叠（见文件头"为什么整块压缩成一行"）。 */
  const [reflogOpen, setReflogOpen] = useState(false);

  const invalidate = () => {
    // 重置/拣选/反转之后：历史、状态、详情都要重新看
    void queryClient.invalidateQueries({ queryKey: logKeyPrefix(repoId) });
    void queryClient.invalidateQueries({ queryKey: reflogKey(repoId) });
    void queryClient.invalidateQueries();
  };

  const pick = useMutation({
    mutationFn: (revision: string) => gitCherryPick(repoId, { revision }),
    onSuccess: () => invalidate(),
    onError: show,
  });

  const undo = useMutation({
    mutationFn: (revision: string) => gitRevert(repoId, { revision }),
    onSuccess: () => invalidate(),
    onError: show,
  });

  const prepare = useMutation({
    mutationFn: (target: string) => gitResetPrepare(repoId, { revision: target, mode }),
    onSuccess: (next) => {
      setPlan(next);
      setConfirmation('');
    },
    onError: show,
  });

  const execute = useMutation({
    // 发送的是**用户输入**的确认词，不是后端给的期望值——
    // 把期望值原样发回去等于没有这道闸门（测试抓到过这个错误）。
    // 参数在点击时捕获：AlertDialogAction 会先关闭对话框（plan 被清空），
    // 再从 state 里读就只剩空串了
    mutationFn: (input: { planId: string; confirmation?: string | undefined }) =>
      gitResetExecute(repoId, input.planId, input.confirmation),
    onSuccess: () => {
      setPlan(null);
      invalidate();
    },
    onError: show,
  });

  const busy = pick.isPending || undo.isPending || prepare.isPending || execute.isPending;

  return (
    <section className="flex shrink-0 flex-col gap-2" data-testid="history-ops">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-16 font-semibold tracking-tight">{t('historyOps.title')}</h2>

        <p className="text-12 text-fg-subtle">
          {detailOid === null
            ? t('historyOps.pickHint')
            : t('historyOps.selectedHint', { oid: detailOid.slice(0, 7) })}
        </p>

        <span className="flex-1" />

        <ToggleGroup
          label={t('historyOps.mode.label')}
          value={mode}
          options={[
            { value: 'soft', label: t('historyOps.mode.soft') },
            { value: 'mixed', label: t('historyOps.mode.mixed') },
            { value: 'hard', label: t('historyOps.mode.hard') },
          ]}
          onValueChange={(next) => setMode(next as ResetMode)}
        />
        <Button
          variant="ghost"
          size="sm"
          disabled={busy || detailOid === null}
          onClick={() => prepare.mutate(detailOid ?? '')}
          data-testid="history-ops-reset"
        >
          <RotateCcw aria-hidden="true" className="size-4" />
          {t('historyOps.resetHere')}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={busy || detailOid === null}
          onClick={() => pick.mutate(detailOid ?? '')}
          data-testid="history-ops-cherry-pick"
        >
          <Scissors aria-hidden="true" className="size-4" />
          {t('historyOps.cherryPick')}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={busy || detailOid === null}
          onClick={() => undo.mutate(detailOid ?? '')}
          data-testid="history-ops-revert"
        >
          <Undo2 aria-hidden="true" className="size-4" />
          {t('historyOps.revert')}
        </Button>

        {/* reflog 默认折叠：它是"找回误删"的备用路径，不该常驻占掉半屏高度。
            折叠用 CSS 隐藏而不是卸载——`ReflogList` 里的查询与订阅保持在位，
            展开时不需要再等一次往返。 */}
        <Button
          variant="ghost"
          size="sm"
          aria-expanded={reflogOpen}
          aria-controls="history-ops-reflog"
          onClick={() => setReflogOpen((open) => !open)}
          data-testid="history-ops-reflog-toggle"
        >
          <ChevronDown
            aria-hidden="true"
            className={cn('size-4 fd-transition', reflogOpen && 'rotate-180')}
          />
          {t('historyOps.reflog.title')}
        </Button>
      </div>

      <div id="history-ops-reflog" className={cn(reflogOpen ? 'max-h-40 overflow-auto' : 'hidden')}>
        <ReflogList />
      </div>

      {/* 重置计划：把将被丢弃的东西摊开，hard 还要输入确认词 */}
      <AlertDialog
        open={plan !== null}
        onOpenChange={(next) => {
          if (!next) {
            setPlan(null);
          }
        }}
      >
        <AlertDialogContent
          impact={t('historyOps.resetPlan.impact', {
            count: plan?.remote.notOnRemote ?? 0,
            total: plan?.discardedCount ?? 0,
          })}
          impactLabel={t('historyOps.resetPlan.impactLabel')}
          data-testid="reset-plan-dialog"
        >
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('historyOps.resetPlan.title', { mode: plan?.mode ?? '' })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('historyOps.resetPlan.description', {
                subject: plan?.targetSubject ?? '',
                discarded: plan?.discardedCount ?? 0,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>

          <ul className="max-h-40 overflow-auto text-12" data-testid="reset-plan-commits">
            {(plan?.discarded ?? []).map((commit) => (
              <li key={commit.oid} className="font-mono">
                {commit.oid.slice(0, 7)} {commit.subject}
              </li>
            ))}
            {plan?.discardedTruncated === true ? (
              <li className="text-fg-subtle">{t('historyOps.resetPlan.more')}</li>
            ) : null}
          </ul>

          {plan !== undefined && plan !== null && plan.lostStaged.length > 0 ? (
            <p className="text-12 text-fg-muted">
              {t('historyOps.resetPlan.lostStaged', { count: plan.lostStaged.length })}
            </p>
          ) : null}
          {plan !== undefined && plan !== null && plan.lostWorktree.length > 0 ? (
            <p className="text-12 text-fg-muted">
              {t('historyOps.resetPlan.lostWorktree', { count: plan.lostWorktree.length })}
            </p>
          ) : null}

          {plan?.requiresConfirmation === true ? (
            <label className="flex flex-col gap-1">
              <span className="text-12 text-fg-muted">
                {t('historyOps.resetPlan.confirmationPrompt', {
                  word: plan.confirmationWord ?? '',
                })}
              </span>
              <Input
                value={confirmation}
                onChange={(event) => {
                  setConfirmation(event.target.value);
                }}
                data-testid="reset-confirm-input"
              />
            </label>
          ) : null}

          <AlertDialogFooter>
            <AlertDialogCancel>{t('common:actions.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              disabled={
                execute.isPending ||
                (plan?.requiresConfirmation === true &&
                  confirmation.trim() !== (plan.confirmationWord ?? ''))
              }
              onClick={() =>
                plan !== null &&
                execute.mutate({ planId: plan.planId, confirmation: confirmation || undefined })
              }
              data-testid="reset-confirm"
            >
              {t('historyOps.resetPlan.confirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
