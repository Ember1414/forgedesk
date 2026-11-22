/**
 * Actions 日志对话框（T4.9 UI）：事件流式加载 + 定高虚拟列表。
 *
 * # 流式为什么不卡 UI
 *
 * 日志行经 `actions:log-chunk` 事件分块到达（后端按完整行切分），这里
 * 先写入有界环形缓冲，最多每 50ms 向 VirtualList 发布一次快照。
 * DOM 只保留可视行；旧日志和超长行被裁剪时明确提示，避免虚拟化掩盖内存增长。
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
  cancelJob,
} from '@/lib/ipc';
import { listenEvent } from '@/lib/ipc/client';

import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/ui/components/dialog';
import { VirtualList } from '@/ui/components/virtual-list';
import { ActionsLogBuffer, MAX_LOG_CHARACTERS, logTextTail } from './actionsLogBuffer';
import type { ActionsLogSnapshot } from './actionsLogBuffer';

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
type StreamEvent = { readonly jobId: string } & (
  | { readonly kind: 'chunk'; readonly text: string; readonly totalLines: number }
  | { readonly kind: 'phase'; readonly phase: LogPhase }
);

export function ActionsLogDialog({ target, onOpenChange }: ActionsLogDialogProps) {
  if (target === null) return null;
  return (
    <ActionsLogSession
      key={`${target.owner}/${target.repo}/${target.jobId}`}
      target={target}
      onOpenChange={onOpenChange}
    />
  );
}

function ActionsLogSession({
  target,
  onOpenChange,
}: {
  readonly target: NonNullable<ActionsLogDialogProps['target']>;
  readonly onOpenChange: ActionsLogDialogProps['onOpenChange'];
}) {
  const { t } = useTranslation('shell');
  const [snapshot, setSnapshot] = useState<ActionsLogSnapshot>({
    lines: [],
    totalLines: 0,
    truncated: false,
  });
  const [phase, setPhase] = useState<LogPhase>('starting');
  const { owner, repo, jobId: targetJobId } = target;
  const { lines, totalLines, truncated } = snapshot;

  useEffect(() => {
    let streamJobId: string | null = null;
    const buffer = new ActionsLogBuffer();
    let flushTimer: ReturnType<typeof setTimeout> | undefined;
    const flush = (): void => {
      clearTimeout(flushTimer);
      flushTimer = undefined;
      setSnapshot(buffer.snapshot());
    };
    const scheduleFlush = (): void => {
      flushTimer ??= setTimeout(flush, 50);
    };
    const unlisteners: (() => void)[] = [];
    let earlyEvents: StreamEvent[] = [];
    let earlyCharacters = 0;
    const cancelledFlag = { value: false };
    const dispatch = (event: StreamEvent): void => {
      if (cancelledFlag.value) return;
      // 后台任务可能先发事件、后返回 jobId；收到身份后按原顺序回放。
      // 早到队列也必须有界，否则下载身份返回较慢时会绕过主缓冲限制。
      if (streamJobId === null) {
        if (event.kind === 'chunk' && event.text.length > MAX_LOG_CHARACTERS) {
          event = { ...event, text: logTextTail(event.text, MAX_LOG_CHARACTERS) };
          buffer.markTruncated();
        }
        earlyEvents.push(event);
        if (event.kind === 'chunk') earlyCharacters += event.text.length;
        while (earlyEvents.length > 512 || earlyCharacters > MAX_LOG_CHARACTERS) {
          const removed = earlyEvents.shift();
          if (removed?.kind === 'chunk') earlyCharacters -= removed.text.length;
          buffer.markTruncated();
        }
      } else if (event.jobId === streamJobId) {
        if (event.kind === 'chunk') {
          buffer.append(event.text, event.totalLines);
          scheduleFlush();
        } else {
          flush();
          setPhase(event.phase);
        }
      }
    };
    const attach = (pending: Promise<() => void>): Promise<void> =>
      pending.then((unlisten) => {
        if (cancelledFlag.value) unlisten();
        else unlisteners.push(unlisten);
      });

    const unlistenChunks = listenActionsLogChunks((payload) => {
      dispatch({ ...payload, kind: 'chunk' });
    });
    const unlistenDone = listenEvent<{ jobId: string; result?: { totalLines?: number } }>(
      JOB_DONE_EVENT,
      (payload) => {
        dispatch({ jobId: payload.jobId, kind: 'phase', phase: 'done' });
      },
    );
    const unlistenFailed = listenEvent<{ jobId: string; error?: { code?: string } }>(
      JOB_FAILED_EVENT,
      (payload) => {
        dispatch({
          jobId: payload.jobId,
          kind: 'phase',
          phase: payload.error?.code === 'CANCELLED' ? 'cancelled' : 'failed',
        });
      },
    );

    void Promise.all([attach(unlistenChunks), attach(unlistenDone), attach(unlistenFailed)])
      .then(() => {
        if (cancelledFlag.value) return null;
        return repoActionsJobLogs(HOST, owner, repo, targetJobId);
      })
      .then((job) => {
        if (job === null) return;
        const { jobId } = job;
        if (cancelledFlag.value) {
          void cancelJob(jobId).catch(() => undefined);
          return;
        }
        streamJobId = jobId;
        setPhase('streaming');
        for (const event of earlyEvents) dispatch(event);
        earlyEvents = [];
      })
      .catch(() => {
        if (!cancelledFlag.value) {
          setPhase('failed');
          cancelledFlag.value = true;
          earlyEvents = [];
          clearTimeout(flushTimer);
          unlisteners.forEach((fn) => fn());
        }
      });

    return () => {
      cancelledFlag.value = true;
      earlyEvents = [];
      clearTimeout(flushTimer);
      if (streamJobId !== null) void cancelJob(streamJobId).catch(() => undefined);
      unlisteners.forEach((fn) => fn());
    };
  }, [owner, repo, targetJobId]);

  const statusText =
    phase === 'starting' || phase === 'streaming'
      ? t('github.actions.logsStreaming', { lines: totalLines })
      : phase === 'done'
        ? t('github.actions.logsDone', { lines: totalLines })
        : phase === 'cancelled'
          ? t('github.actions.logsCancelled', { lines: totalLines })
          : t('github.actions.logsFailed');

  return (
    <Dialog open={target !== null} onOpenChange={onOpenChange}>
      <DialogContent
        className="flex max-h-[calc(100dvh-2rem)] max-w-3xl flex-col overflow-hidden"
        closeLabel={t('common:actions.close')}
      >
        <DialogHeader>
          <DialogTitle>{t('github.actions.logsTitle', { job: target?.jobName ?? '' })}</DialogTitle>
        </DialogHeader>
        <p className="text-12 text-fg-muted" data-testid="actions-log-status">
          {statusText}
        </p>
        {truncated ? (
          <p className="shrink-0 text-12 text-fg-muted" data-testid="actions-log-truncated">
            {t('github.actions.logsTruncated', { lines: lines.length })}
          </p>
        ) : null}
        {lines.length === 0 ? (
          <p className="text-13 text-fg-subtle" data-testid="actions-log-empty">
            {phase === 'failed' ? statusText : t('github.actions.logsEmpty')}
          </p>
        ) : (
          <div className="min-h-0 overflow-y-auto rounded-md border border-line bg-surface">
            <VirtualList
              items={lines}
              itemHeight={ROW_HEIGHT}
              height={420}
              className="max-h-[max(96px,calc(100dvh-14rem))]"
              label={t('github.actions.logsTitle', { job: target?.jobName ?? '' })}
              getKey={(_, index) => `log-line-${totalLines - lines.length + index}`}
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
