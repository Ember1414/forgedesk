/**
 * Actions（T4.9）：命令 DTO、具名封装与日志分块事件订阅。
 *
 * # 日志是事件流不是返回值
 *
 * `repoActionsJobLogs` 立即返回 `jobId`（JobRunner 长任务）；日志行经
 * `actions:log-chunk` 事件分块推送（`text` 是完整行的文本块），结束时
 * `job:done` 载荷 `{ totalLines }`。订阅按 `jobId` 过滤——多个日志窗口
 * 并存时互不串流。取消走通用 `jobCancel`。
 *
 * # 状态与结论是两回事
 *
 * `status`（queued/in_progress/completed）是调度状态，`conclusion`
 * （success/failure/cancelled…）只有 completed 后才有值：UI 判定
 * "可取消 / 可重跑"先看 status 再看 conclusion。
 */
import { invokeCommand, listenEvent, type Unlisten } from './client';

/** workflow run 列表条目。 */
export interface WorkflowRunSummary {
  readonly id: number;
  /** 展示标题。 */
  readonly name: string;
  readonly headBranch?: string | null;
  readonly headSha?: string | null;
  /** `queued` / `in_progress` / `completed`。 */
  readonly status: string;
  /** 结论（未完成为 null）。 */
  readonly conclusion: string | null;
  readonly event?: string | null;
  readonly actor: string;
  readonly runNumber: number;
  readonly createdAt?: string | null;
  readonly updatedAt?: string | null;
  readonly htmlUrl: string;
}

/** workflow run 分页。 */
export interface RunPage {
  readonly items: readonly WorkflowRunSummary[];
  readonly nextPage: number | null;
}

/** run 的一个 job。 */
export interface RunJob {
  readonly id: number;
  readonly name: string;
  readonly status: string;
  readonly conclusion: string | null;
  readonly startedAt?: string | null;
  readonly completedAt?: string | null;
}

/** `actions:log-chunk` 的载荷。 */
export interface ActionsLogChunkPayload {
  readonly jobId: string;
  /** 本块的完整行（含行尾换行）。 */
  readonly text: string;
  /** 截至本块的累计行数。 */
  readonly totalLines: number;
}

/** 日志分块事件名（与后端 `EVENT_ACTIONS_LOG_CHUNK` 同一契约）。 */
export const ACTIONS_LOG_CHUNK_EVENT = 'actions:log-chunk';

/** 订阅日志分块事件。 */
export function listenActionsLogChunks(
  handler: (payload: ActionsLogChunkPayload) => void,
): Promise<Unlisten> {
  return listenEvent<ActionsLogChunkPayload>(ACTIONS_LOG_CHUNK_EVENT, handler);
}

/** 列出 workflow run。 */
export function repoActionsRunsList(request: {
  readonly host: string;
  readonly owner: string;
  readonly repo: string;
  readonly repoId?: number;
  readonly page?: number;
  readonly perPage?: number;
}): Promise<RunPage> {
  return invokeCommand<RunPage>('repo_actions_runs_list', request);
}

/** 一个 run 的 job 列表。 */
export function repoActionsRunJobs(
  host: string,
  owner: string,
  repo: string,
  runId: number,
  repoId?: number,
): Promise<RunJob[]> {
  return invokeCommand<RunJob[]>('repo_actions_run_jobs', {
    host,
    owner,
    repo,
    runId,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/** 取消一个 run。 */
export function repoActionsRunCancel(
  host: string,
  owner: string,
  repo: string,
  runId: number,
  repoId?: number,
): Promise<void> {
  return invokeCommand<void>('repo_actions_run_cancel', {
    host,
    owner,
    repo,
    runId,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/** 重跑一个 run。 */
export function repoActionsRunRerun(
  host: string,
  owner: string,
  repo: string,
  runId: number,
  repoId?: number,
): Promise<void> {
  return invokeCommand<void>('repo_actions_run_rerun', {
    host,
    owner,
    repo,
    runId,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/** 流式加载一个 job 的日志：返回 jobId，行经 `actions:log-chunk` 送达。 */
export function repoActionsJobLogs(
  host: string,
  owner: string,
  repo: string,
  jobId: number,
  repoId?: number,
): Promise<{ readonly jobId: string }> {
  return invokeCommand<{ readonly jobId: string }>('repo_actions_job_logs', {
    host,
    owner,
    repo,
    jobId,
    ...(repoId === undefined ? {} : { repoId }),
  });
}
