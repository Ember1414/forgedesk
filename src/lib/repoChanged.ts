/**
 * `repo:changed` 的统一处理（M1 / T1.10）。
 *
 * # 一个事件，两种来源
 *
 * 事件来自两处：应用自己的写操作（暂存、提交、回滚），以及**文件监听**
 * （终端里的 git 命令、编辑器保存、别的工具）。两者共用同一份载荷与同一套
 * 类别（`workspace` / `refs` / `large`），因此处理逻辑只有这一份。
 *
 * # 为什么要按类别失效
 *
 * "工作区变了"与"引用变了"影响的是不同的查询。全部失效最省事，但会让
 * 只改了一个文件的情况也去重取提交历史——在大仓库上那是明显的白刷。
 *
 * # 为什么要节流
 *
 * 保存一次文件可能连着产生几个事件（编辑器写临时文件再改名）；
 * `git checkout` 更是几十个事件一起到。1 秒窗口内的重复变化合并成一次失效，
 * 保证不会出现"同一个查询被连续打断三次、每次都读回旧值"的抖动。
 * 窗口内**最后一次**变化仍会被处理（尾随调用），因此不会丢事件。
 */

import { useEffect, useRef } from 'react';

import { useQueryClient } from '@tanstack/react-query';
import type { QueryClient } from '@tanstack/react-query';

import { isTauriRuntime } from '@/lib/ipc/client';
import { onRepoChanged } from '@/lib/ipc/workspace';
import type { RepoChangeKind } from '@/lib/ipc/workspace';
import {
  BRANCHES_QUERY_KEY,
  AUTHORS_QUERY_KEY,
  COMMIT_DETAIL_QUERY_KEY,
  DIFF_KEY_PATH_INDEX,
  DIFF_QUERY_KEY,
  LOG_QUERY_KEY,
  SNAPSHOTS_QUERY_KEY,
  STATUS_QUERY_KEY,
  SYNC_STATUS_QUERY_KEY,
} from '@/lib/queryKeys';

/** 事件类别（定义在 IPC 层：它首先是线格式的一部分）。 */
export type { RepoChangeKind };

/** 默认节流窗口（任务定义：1 秒内不重复请求）。 */
export const DEFAULT_THROTTLE_MS = 1_000;

/**
 * 某个类别要失效的查询键前缀。
 *
 * 三处需要解释的取舍：
 * - **引用变化也失效状态**：`git checkout` 之后"已暂存"是相对**新的** HEAD 而言的，
 *   只刷新历史会让状态面板停在一个已经不存在的事实上；
 * - **工作区变化不动历史**：文件内容与 HEAD 无关，刷它是纯浪费；
 * - **`large` 全部失效**：事件量超过阈值时我们连路径都没有，宁可整体重取
 *   （这本来就是"大仓库 checkout"这种场景，用户等得起一次全量）。
 */
export function queryKeysForChange(
  repoId: number,
  kind: RepoChangeKind,
): readonly (readonly unknown[])[] {
  switch (kind) {
    case 'workspace':
      return [[STATUS_QUERY_KEY, repoId]];
    case 'refs':
      return [
        [STATUS_QUERY_KEY, repoId],
        [LOG_QUERY_KEY, repoId],
        [BRANCHES_QUERY_KEY, repoId],
        [SNAPSHOTS_QUERY_KEY, repoId],
        [COMMIT_DETAIL_QUERY_KEY, repoId],
        [AUTHORS_QUERY_KEY, repoId],
        // 同步条上的 ahead/behind 是相对上游算出来的：引用一动它就可能过期
        [SYNC_STATUS_QUERY_KEY, repoId],
      ];
    case 'large':
      return [
        [STATUS_QUERY_KEY, repoId],
        [DIFF_QUERY_KEY, repoId],
        [LOG_QUERY_KEY, repoId],
        [BRANCHES_QUERY_KEY, repoId],
        [SNAPSHOTS_QUERY_KEY, repoId],
        [COMMIT_DETAIL_QUERY_KEY, repoId],
        [AUTHORS_QUERY_KEY, repoId],
        [SYNC_STATUS_QUERY_KEY, repoId],
      ];
    default:
      // 认不出的类别（例如载荷来自更早的版本）按"大量变更"处理：
      // 多刷一次无伤大雅，静默不刷新或直接抛错才会让人以为界面坏了
      return [
        [STATUS_QUERY_KEY, repoId],
        [DIFF_QUERY_KEY, repoId],
        [LOG_QUERY_KEY, repoId],
        [BRANCHES_QUERY_KEY, repoId],
        [SNAPSHOTS_QUERY_KEY, repoId],
        [COMMIT_DETAIL_QUERY_KEY, repoId],
        [AUTHORS_QUERY_KEY, repoId],
        [SYNC_STATUS_QUERY_KEY, repoId],
      ];
  }
}

/** 失效器：把事件收敛成"该失效哪些查询"。 */
export interface RepoChangeInvalidator {
  /** 处理一次变化（按节流窗口合并）。 */
  readonly invalidate: (repoId: number, kind: RepoChangeKind, paths?: readonly string[]) => void;
  /** 清掉挂起的尾随调用（卸载时调用）。 */
  readonly dispose: () => void;
}

/** 可注入的时钟与定时器（测试用它把 1 秒的等待变成瞬时）。 */
export interface InvalidatorOptions {
  readonly throttleMs?: number;
  readonly now?: () => number;
  /**
   * 额外要失效的查询键（调用方自己的查询）。
   *
   * 例：提交面板的"最近提交提示"依赖历史，外部提交之后它必须跟着更新，
   * 而它不属于任何通用类别。
   */
  readonly extraKeys?: readonly (readonly unknown[])[];
}

/**
 * 建一个失效器。
 *
 * `paths` 非空时只失效**受影响的那些文件**的 diff：整仓库的 diff 一起失效，
 * 会让用户正打开着的查看器无谓地重取一次（大文件上很明显）。
 */
export function createRepoChangeInvalidator(
  queryClient: QueryClient,
  options: InvalidatorOptions = {},
): RepoChangeInvalidator {
  const throttleMs = options.throttleMs ?? DEFAULT_THROTTLE_MS;
  const now = options.now ?? (() => Date.now());
  const lastRunAt = new Map<number, number>();
  const pending = new Map<number, ReturnType<typeof setTimeout>>();

  const run = (repoId: number, kind: RepoChangeKind, paths: readonly string[]): void => {
    lastRunAt.set(repoId, now());

    for (const queryKey of queryKeysForChange(repoId, kind)) {
      void queryClient.invalidateQueries({ queryKey });
    }
    for (const queryKey of options.extraKeys ?? []) {
      void queryClient.invalidateQueries({ queryKey });
    }

    if (kind !== 'large' && paths.length > 0) {
      void queryClient.invalidateQueries({
        queryKey: [DIFF_QUERY_KEY, repoId],
        predicate: (query) => paths.includes(String(query.queryKey[DIFF_KEY_PATH_INDEX] ?? '')),
      });
    }
  };

  return {
    invalidate: (repoId, kind, paths = []) => {
      const elapsed = now() - (lastRunAt.get(repoId) ?? Number.NEGATIVE_INFINITY);

      if (elapsed >= throttleMs) {
        // 本窗口内已经覆盖了挂起的那个尾随调用，取消它（否则会白刷一次）
        const scheduled = pending.get(repoId);
        if (scheduled !== undefined) {
          clearTimeout(scheduled);
          pending.delete(repoId);
        }
        run(repoId, kind, paths);
        return;
      }

      // 窗口内：挂一个尾随调用，用**最后一次**的类别与路径
      const scheduled = pending.get(repoId);
      if (scheduled !== undefined) {
        clearTimeout(scheduled);
      }
      pending.set(
        repoId,
        setTimeout(() => {
          pending.delete(repoId);
          run(repoId, kind, paths);
        }, throttleMs - elapsed),
      );
    },
    dispose: () => {
      for (const scheduled of pending.values()) {
        clearTimeout(scheduled);
      }
      pending.clear();
    },
  };
}

/** 订阅选项。 */
export interface RepoChangeSubscription {
  /** `large` 事件到达时回调（界面据此说明"为什么列表整体刷了一次"）。 */
  readonly onLargeChange?: () => void;
  /** 额外要失效的查询键，见 [`InvalidatorOptions::extraKeys`]。 */
  readonly extraKeys?: readonly (readonly unknown[])[];
}

/**
 * 订阅某个仓库的变化并失效对应查询。
 *
 * 浏览器预览与 jsdom 里没有 Tauri 事件系统：整条订阅跳过（由 e2e 覆盖）。
 */
export function useRepoChangeInvalidation(
  repoId: number,
  options: RepoChangeSubscription = {},
): void {
  const queryClient = useQueryClient();
  // 选项放进 ref：调用方常常直接传内联对象 / 箭头函数，把它写进依赖数组会让
  // 每次渲染都重新订阅一次（卸载再订阅会丢掉这期间的事件）
  const optionsRef = useRef(options);
  useEffect(() => {
    optionsRef.current = options;
  });

  useEffect(() => {
    if (!isTauriRuntime() || !Number.isFinite(repoId)) {
      return;
    }

    let unlisten: (() => void) | undefined;
    let cancelled = false;
    // 每个挂载周期一个失效器：节流窗口的状态不该跨仓库或跨挂载复用
    const invalidator = createRepoChangeInvalidator(queryClient, {
      extraKeys: optionsRef.current.extraKeys ?? [],
    });

    void onRepoChanged((payload) => {
      if (payload.repoId !== repoId) {
        return;
      }
      invalidator.invalidate(repoId, payload.kind, payload.paths);
      if (payload.kind === 'large') {
        optionsRef.current.onLargeChange?.();
      }
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    });

    return () => {
      cancelled = true;
      unlisten?.();
      invalidator.dispose();
    };
  }, [queryClient, repoId]);
}
