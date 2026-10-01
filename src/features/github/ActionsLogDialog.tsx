/**
 * Actions 日志对话框（T4.9 UI）：事件流式加载 + 定高虚拟列表。
 *
 * # 流式为什么不卡 UI
 *
 * 日志行经 `actions:log-chunk` 事件分块到达（后端按完整行切分），这里
 * 只把新行 append 进数组并交给 [`VirtualList`]——DOM 里永远只有可视区
 * 的几十行，5MB 还是 50MB 对渲染层的成本是同一常数（M4 验收的落点）。
 * 行内容是 CI 日志原文，React 文本节点展示（默认转义），无 HTML 注入面。
 *
 * # 生命周期
 *
 * 打开时订阅事件（按 jobId 过滤）并发起 `repoActionsJobLogs` 长任务；
 * `job:done` → 停止；`job:failed` 且 code=CANCELLED → 显示"已取消"（用户
 * 自己取消不是错误）；其余失败 → 错误态。关闭时退订（Unlisten）。
 */
import { useEffect, useState } from 'react';

import { useTranslation } from 'react-i18next';

import {
  listenActionsLogChunks,
  repoActionsJobLogs,
  JOB_DONE_EVENT,
  JOB_FAILED_EVENT,
} from '@/lib/ipc';
import { listenEvent } from '@/lib/ipc/client';

import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/ui/components/dialog';
import { VirtualList } from '@/ui/components/virtual-list';

/** 与其他 GitHub 页一致的站点常量。 */
const HOST = 'github.com';

/** 行高（px）：虚拟化的数学依赖它，与行样式 leading-6 保持一致。 */
const ROW_HEIGHT = 24;

export interface ActionsLogDialogProps {
  /** 目标 job；`null` 表示关闭。 */
  readonly target: {
    readonly owner: string;
    readonly repo: string;
    readonly jobId: number;
    readonly jobName: string;
  } | null;
  readonly onOpenChange: (open: boolean) => void;
}

type LogPhase = 'starting' | 'streaming' | 'done' | 'cancelled' | 'failed';

/** 把一个事件块按行拆开（行尾换行符由后端保证在块内）。 */
function appendChunk(lines: readonly string[], text: string): readonly string[] {
  const parts = text.split('\n');
  // split 后尾元素是最后一个换行后的空串（有换行结尾时），丢弃
  if (parts.length > 1 && parts[parts.length - 1] === '') {
    parts.pop();
  }
  return [...lines, ...parts];
}

export function ActionsLogDialog({ target, onOpenChange }: ActionsLogDialogProps) {
  const { t } = useTranslation('shell');
  const [lines, setLines] = useState<readonly string[]>([]);
  const [phase, setPhase] = useState<LogPhase>('starting');

  useEffect(() => {
    if (target === null) {
      return;
    }
    let streamJobId: string | null = null;
    let unlisteners: readonly (() => void)[] = [];
    const cancelledFlag = { value: false };

    const unlistenChunks = listenActionsLogChunks((payload) => {
      if (payload.jobId !== streamJobId || cancelledFlag.value) {
        return;
      }
      setLines((current) => appendChunk(current, payload.text));
    });
    const unlistenDone = listenEvent<{ jobId: string; result?: { totalLines?: number } }>(
      JOB_DONE_EVENT,
      (payload) => {
        if (payload.jobId !== streamJobId || cancelledFlag.value) {
          return;
        }
        setPhase('done');
      },
    );
    const unlistenFailed = listenEvent<{ jobId: string; error?: { code?: string } }>(
      JOB_FAILED_EVENT,
      (payload) => {
        if (payload.jobId !== streamJobId || cancelledFlag.value) {
          return;
        }
        setPhase(payload.error?.code === 'CANCELLED' ? 'cancelled' : 'failed');
      },
    );

    void Promise.resolve()
      .then(() => repoActionsJobLogs(HOST, target.owner, target.repo, target.jobId))
      .then(({ jobId }) => {
        if (cancelledFlag.value) {
          return;
        }
        streamJobId = jobId;
        setPhase('streaming');
      })
      .catch(() => {
        if (!cancelledFlag.value) {
          setPhase('failed');
        }
      });

    void Promise.all([unlistenChunks, unlistenDone, unlistenFailed]).then((unlisten) => {
      if (cancelledFlag.value) {
        unlisten.forEach((fn) => fn());
      } else {
        unlisteners = unlisten;
      }
    });

    return () => {
      cancelledFlag.value = true;
      unlisteners.forEach((fn) => fn());
    };
  }, [target]);

  const statusText =
    phase === 'starting' || phase === 'streaming'
      ? t('github.actions.logsStreaming', { lines: lines.length })
      : phase === 'done'
        ? t('github.actions.logsDone', { lines: lines.length })
        : phase === 'cancelled'
          ? t('github.actions.logsCancelled', { lines: lines.length })
          : t('github.actions.logsFailed');

  return (
    <Dialog open={target !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl" closeLabel={t('common:actions.close')}>
        <DialogHeader>
          <DialogTitle>{t('github.actions.logsTitle', { job: target?.jobName ?? '' })}</DialogTitle>
        </DialogHeader>
        <p className="text-12 text-fg-muted" data-testid="actions-log-status">
          {statusText}
        </p>
        {lines.length === 0 ? (
          <p className="text-13 text-fg-subtle" data-testid="actions-log-empty">
            {phase === 'failed' ? statusText : t('github.actions.logsEmpty')}
          </p>
        ) : (
          <div className="overflow-hidden rounded-md border border-line bg-surface">
            <VirtualList
              items={lines}
              itemHeight={ROW_HEIGHT}
              height={420}
              label={t('github.actions.logsTitle', { job: target?.jobName ?? '' })}
              getKey={(_, index) => `log-line-${index}`}
              renderItem={(line) => (
                <div className="font-mono text-12 leading-6 whitespace-pre">{line}</div>
              )}
            />
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
