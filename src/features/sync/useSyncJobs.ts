/**
 * 同步长任务的前端编排（T2.6）：发起 → 进度 → 结果 → 失效。
 *
 * # 为什么任务状态放在这里而不是 jobStore
 *
 * `jobStore` 是**跨页面**的任务投影（状态栏数"还有几个任务在跑"），只保留
 * 展示所需的最小字段；而"这一次同步走到哪一步、暴露了哪些冲突文件、被拒时
 * 是哪三条修复路径"是同步条自己的界面状态。两者混在一起会让 jobStore 变成
 * 一个什么都要装的垃圾桶。
 *
 * # 三条容易做错的纪律
 *
 * 1. **jobId 是唯一关联键**：事件是全局广播的，只有我们自己发起的 jobId 才
 *    该被本组件消费（另一台窗口/另一个仓库的任务不能污染这里的状态）；
 * 2. **被取消不是错误**：用户按了取消再弹一个红色错误提示，等于在质疑他的操作；
 * 3. **被拒绝要落到对话框**：`PUSH_REJECTED` 自带三条修复动作，必须用能承载
 *    三个出口的界面展示（`errors.ts` 的 toast 通道只适合放一条）
 *    —— 而且动作要由**这里**执行（只有这里知道 repoId 与本次的 spec）。
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useQueryClient } from '@tanstack/react-query';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';

import { normalizeError, useAppError } from '@/lib/errors';
import type { NormalizedError } from '@/lib/errors';
import {
  cancelJob,
  gitFetch,
  gitPull,
  gitPush,
  onJobDone,
  onJobFailed,
  onJobProgress,
  progressPercent,
  pullConflicts,
  readSyncResult,
} from '@/lib/ipc';
import type { FetchSpec, PullStrategy, PushSpec, SyncJobRef } from '@/lib/ipc';
import { queryKeysForChange } from '@/lib/repoChanged';
import { useJobStore } from '@/stores/jobStore';
import { pushToast } from '@/stores/toastStore';

/** 三种同步操作。 */
export type SyncJobKind = 'fetch' | 'pull' | 'push';

/** 进度条要展示的状态。 */
export interface SyncProgress {
  readonly kind: SyncJobKind;
  /** 阶段短名（`counting` / `receiving`…）：界面按它选 i18n 文案。 */
  readonly phase: string;
  /** 0–1；后端还没给出总量时为 null（渲染不确定进度条）。 */
  readonly percent: number | null;
  readonly current: number | null;
  readonly total: number | null;
  /** 最近的原始输出行（展开"详细日志"时看）。 */
  readonly messages: readonly string[];
}

/** 拉取留下的冲突现场。 */
export interface PullConflict {
  readonly files: readonly string[];
}

/** 详细日志最多保留的行数（进度行可能刷得很快，界面不需要全部）。 */
const MAX_MESSAGES = 50;

/** 事件回调里需要读到的、会随渲染变化的依赖。 */
interface Handlers {
  readonly invalidateRepo: (repoId: number) => void;
  readonly show: (raw: unknown) => NormalizedError;
  readonly t: TFunction<'shell'>;
}

export interface SyncJobs {
  /** 有任务在跑（按钮据此禁用）。 */
  readonly busy: boolean;
  readonly progress: SyncProgress | null;
  /** 拉取产生的冲突（非 null 时界面应弹引导）。 */
  readonly conflict: PullConflict | null;
  /** 推送被拒（非 null 时界面应弹三条修复路径）。 */
  readonly rejection: NormalizedError | null;
  runFetch(spec?: FetchSpec): void;
  runPull(strategy: PullStrategy): void;
  runPush(spec: PushSpec): void;
  cancel(): void;
  dismissConflict(): void;
  dismissRejection(): void;
  /** 被拒后的第二条路：`--force-with-lease` 重推（后端只可能产出 lease 形态）。 */
  retryWithForceWithLease(): void;
  /** 被拒后的第一条路：先拉取，再由用户决定是否重推。 */
  fetchThenRetry(): void;
}

/** 我们自己发起的任务（用于过滤全局事件）。 */
interface PendingJob {
  readonly kind: SyncJobKind;
  readonly repoId: number;
}

export function useSyncJobs(repoId: number): SyncJobs {
  const queryClient = useQueryClient();
  const { t } = useTranslation('shell');
  const { show } = useAppError();

  const pendingRef = useRef(new Map<string, PendingJob>());
  const lastPushRef = useRef<PushSpec | null>(null);
  const [active, setActive] = useState<{ jobId: string; kind: SyncJobKind } | null>(null);
  const [progress, setProgress] = useState<SyncProgress | null>(null);
  const [conflict, setConflict] = useState<PullConflict | null>(null);
  const [rejection, setRejection] = useState<NormalizedError | null>(null);

  const invalidateRepo = useCallback(
    (target: number) => {
      // 引用一变，状态/历史/分支/快照/提交详情/ahead-behind 都可能过期——
      // 用 `repo:changed` 的同一份口径，避免"这里失效了那里没失效"的分叉。
      for (const queryKey of queryKeysForChange(target, 'refs')) {
        void queryClient.invalidateQueries({ queryKey });
      }
    },
    [queryClient],
  );

  // 事件订阅只挂一次（依赖数组为空），回调里要用的东西通过 ref 读最新值
  const handlersRef = useRef<Handlers>({ invalidateRepo, show, t });
  useEffect(() => {
    handlersRef.current = { invalidateRepo, show, t };
  });

  useEffect(() => {
    let unlisteners: (() => void)[] = [];
    let disposed = false;

    void Promise.all([
      onJobProgress((payload) => {
        const job = pendingRef.current.get(payload.jobId);
        if (job === undefined) {
          return;
        }
        const percent = progressPercent(payload);
        if (percent !== null) {
          useJobStore.getState().setJobProgress(payload.jobId, percent / 100);
        }
        setProgress((current) => {
          const sameJob = current !== null && current.kind === job.kind;
          const base = sameJob ? current.messages : [];
          const line = payload.message ?? '';
          return {
            kind: job.kind,
            phase: payload.phase,
            // 阶段切换时总量会短暂缺失：保留上一次的百分比，避免进度条来回跳
            percent: percent === null ? (sameJob ? current.percent : null) : percent / 100,
            current: payload.current,
            total: payload.total,
            messages: line === '' ? base : [...base, line].slice(-MAX_MESSAGES),
          };
        });
      }),

      onJobDone((payload) => {
        const job = pendingRef.current.get(payload.jobId);
        if (job === undefined) {
          return;
        }
        pendingRef.current.delete(payload.jobId);
        useJobStore.getState().setJobState(payload.jobId, 'succeeded');
        setActive((current) => (current?.jobId === payload.jobId ? null : current));
        setProgress((current) => (current?.kind === job.kind ? null : current));
        handlersRef.current.invalidateRepo(job.repoId);

        if (job.kind === 'pull') {
          const files = pullConflicts(readSyncResult(payload.result).pull);
          if (files.length > 0) {
            setConflict({ files });
          }
        }
        if (job.kind === 'push') {
          const pushed = readSyncResult(payload.result).push;
          pushToast({
            tone: 'success',
            title: handlersRef.current.t('sync.pushDone', { remote: pushed?.remote ?? 'origin' }),
          });
        }
      }),

      onJobFailed((payload) => {
        const job = pendingRef.current.get(payload.jobId);
        if (job === undefined) {
          return;
        }
        pendingRef.current.delete(payload.jobId);
        useJobStore.getState().setJobState(payload.jobId, 'failed');
        setActive((current) => (current?.jobId === payload.jobId ? null : current));
        setProgress(null);

        const error = normalizeError(payload.error);
        if (error.code === 'CANCELLED') {
          // 用户自己按的取消：再弹一个错误提示只会让人以为出了故障
          return;
        }
        if (error.code === 'PUSH_REJECTED') {
          // 三条修复路径要落到对话框里（toast 只放得下一条）
          setRejection(error);
          return;
        }
        handlersRef.current.show(payload.error);
      }),
    ])
      .then((fns) => {
        if (disposed) {
          for (const fn of fns) {
            fn();
          }
          return;
        }
        unlisteners = fns;
      })
      .catch(() => {
        // 浏览器预览（没有 Tauri 事件系统）里静默降级：拿不到结果，但界面不崩
      });

    return () => {
      disposed = true;
      for (const fn of unlisteners) {
        fn();
      }
      unlisteners = [];
    };
  }, []);

  const start = useCallback(
    (kind: SyncJobKind, request: () => Promise<SyncJobRef>, pushSpec?: PushSpec) => {
      void (async () => {
        try {
          const { jobId } = await request();
          pendingRef.current.set(jobId, { kind, repoId });
          if (pushSpec !== undefined) {
            lastPushRef.current = pushSpec;
          }
          const jobs = useJobStore.getState();
          jobs.enqueueJob({ id: jobId, kind, label: t(`sync.job.${kind}`) });
          jobs.setJobState(jobId, 'running');
          setActive({ jobId, kind });
          setProgress({
            kind,
            phase: 'starting',
            percent: null,
            current: null,
            total: null,
            messages: [],
          });
        } catch (error) {
          // 任务根本没起来（参数非法、仓库不存在…）：直接走统一错误通道
          show(error);
        }
      })();
    },
    [repoId, show, t],
  );

  const runFetch = useCallback(
    (spec: FetchSpec = {}) => {
      start('fetch', () => gitFetch(repoId, spec));
    },
    [repoId, start],
  );

  const runPull = useCallback(
    (strategy: PullStrategy) => {
      start('pull', () => gitPull(repoId, { strategy }));
    },
    [repoId, start],
  );

  const runPush = useCallback(
    (spec: PushSpec) => {
      start('push', () => gitPush(repoId, spec), spec);
    },
    [repoId, start],
  );

  const cancel = useCallback(() => {
    if (active !== null) {
      void cancelJob(active.jobId);
    }
  }, [active]);

  const retryWithForceWithLease = useCallback(() => {
    const previous = lastPushRef.current ?? {};
    setRejection(null);
    runPush({ ...previous, forceWithLease: true });
  }, [runPush]);

  const fetchThenRetry = useCallback(() => {
    setRejection(null);
    runFetch({});
  }, [runFetch]);

  return {
    busy: active !== null,
    progress,
    conflict,
    rejection,
    runFetch,
    runPull,
    runPush,
    cancel,
    dismissConflict: () => {
      setConflict(null);
    },
    dismissRejection: () => {
      setRejection(null);
    },
    retryWithForceWithLease,
    fetchThenRetry,
  };
}
