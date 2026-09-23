/**
 * 长任务事件与取消（对应 `docs/API.md` §3 的 `job:*` 事件表）。
 *
 * 事件**不携带用户可见文案**：`phase` 是阶段标识（`receiving` / `counting`…），
 * 界面按它选 i18n 文案；`message` 只是 git 的原始进度行（已脱敏），
 * 用于"进度卡住"时展示真实输出。
 */
import { invokeCommand, listenEvent, type Unlisten } from './client';

/** 进度事件名。 */
export const JOB_PROGRESS_EVENT = 'job:progress';
/** 任务成功结束的事件名。 */
export const JOB_DONE_EVENT = 'job:done';
/** 任务失败结束的事件名。 */
export const JOB_FAILED_EVENT = 'job:failed';

/** `job:progress` 的载荷。 */
export interface JobProgressPayload {
  readonly jobId: string;
  /** 阶段标识（稳定短名，用于选文案）。 */
  readonly phase: string;
  readonly current: number | null;
  readonly total: number | null;
  /** git 的原始进度行（已脱敏）。 */
  readonly message?: string;
}

/** `job:done` 的载荷。 */
export interface JobDonePayload {
  readonly jobId: string;
  readonly result: unknown;
}

/** `job:failed` 的载荷（`error` 的形状见 `src/lib/errors.ts`）。 */
export interface JobFailedPayload {
  readonly jobId: string;
  readonly error: unknown;
}

/** 进度百分比（0–100）；总量未知时返回 `null`（渲染不确定进度条）。 */
export function progressPercent(payload: JobProgressPayload): number | null {
  const { current, total } = payload;
  if (current === null || total === null || total <= 0) {
    return null;
  }
  return Math.min(100, Math.max(0, Math.round((current / total) * 100)));
}

/** 请求取消一个正在运行的任务；返回它此前是否在运行。 */
export function cancelJob(jobId: string): Promise<boolean> {
  return invokeCommand<boolean>('job_cancel', { jobId });
}

/** 订阅进度事件（记得在卸载时调用返回的 unlisten）。 */
export function onJobProgress(handler: (payload: JobProgressPayload) => void): Promise<Unlisten> {
  return listenEvent<JobProgressPayload>(JOB_PROGRESS_EVENT, handler);
}

/** 订阅成功结束事件。 */
export function onJobDone(handler: (payload: JobDonePayload) => void): Promise<Unlisten> {
  return listenEvent<JobDonePayload>(JOB_DONE_EVENT, handler);
}

/** 订阅失败结束事件。 */
export function onJobFailed(handler: (payload: JobFailedPayload) => void): Promise<Unlisten> {
  return listenEvent<JobFailedPayload>(JOB_FAILED_EVENT, handler);
}
