/**
 * 后台任务列表（占位实现）。
 *
 * 定位：M0 阶段后端还没有 JobRunner（见 AGENTS.md §6「长任务可取消」，实现落在 M1），
 * 这里先把**前端展示所需的形状**定下来，让状态栏、仪表盘等界面不必等后端就能接线。
 * 后续接入后端 `job://progress` 事件时，只需要把事件映射为本 store 的动作，
 * 组件不需要改动。
 *
 * 约定：
 *   - 任务记录是只读投影，id 由后端生成（M1 后由 JobRunner 提供），前端不自造语义。
 *   - 不在这里保存任务的日志内容（日志走 diagnostics，避免把大字符串塞进 store）。
 */
import { create } from 'zustand';

export const JOB_STATES = ['queued', 'running', 'succeeded', 'failed', 'cancelled'] as const;
export type JobState = (typeof JOB_STATES)[number];

/** 终态：不会再有进度更新。 */
const TERMINAL_STATES: readonly JobState[] = ['succeeded', 'failed', 'cancelled'];

export interface JobRecord {
  readonly id: string;
  /** 任务类型标识（如 'fetch'、'clone'、'snapshot'），用于选择展示文案。 */
  readonly kind: string;
  /** 已本地化的展示文案；由调用方提供，避免 store 依赖 i18n。 */
  readonly label: string;
  readonly state: JobState;
  /** 0–1 的进度；后端无法提供进度时为 null（用于渲染不确定进度条）。 */
  readonly progress: number | null;
  readonly startedAt: number;
  readonly finishedAt: number | null;
}

export interface JobStoreState {
  readonly jobs: readonly JobRecord[];

  enqueueJob(input: { id: string; kind: string; label: string }): void;
  setJobState(jobId: string, state: JobState): void;
  setJobProgress(jobId: string, progress: number): void;
  removeJob(jobId: string): void;
  clearFinishedJobs(): void;
}

/** 仍在排队或运行中的任务数量（状态栏与"退出前确认"会用到）。 */
export function countActiveJobs(jobs: readonly JobRecord[]): number {
  return jobs.filter((job) => job.state === 'queued' || job.state === 'running').length;
}

/** 任务是否处于终态。 */
export function isJobFinished(job: JobRecord): boolean {
  return TERMINAL_STATES.includes(job.state);
}

/**
 * 初始状态（导出供测试复位；store 是模块级单例，不复位会产生顺序依赖）。
 */
export const initialJobState = { jobs: [] as readonly JobRecord[] };

export const useJobStore = create<JobStoreState>()((set) => ({
  ...initialJobState,

  enqueueJob: ({ id, kind, label }) => {
    set((state) => {
      // 同一个 id 重复入队时忽略，避免界面出现两行同源任务（事件重放不会造成重复行）
      if (state.jobs.some((job) => job.id === id)) {
        return state;
      }
      return {
        jobs: [
          ...state.jobs,
          {
            id,
            kind,
            label,
            state: 'queued',
            progress: null,
            startedAt: Date.now(),
            finishedAt: null,
          },
        ],
      };
    });
  },

  setJobState: (jobId, nextState) => {
    set((state) => ({
      jobs: state.jobs.map((job) => {
        if (job.id !== jobId) {
          return job;
        }
        const finished = TERMINAL_STATES.includes(nextState);
        return {
          ...job,
          state: nextState,
          finishedAt: finished ? Date.now() : null,
          // 进入终态时把进度钉在两端，避免进度条停在 87% 这类误导位置
          progress: finished ? (nextState === 'succeeded' ? 1 : job.progress) : job.progress,
        };
      }),
    }));
  },

  setJobProgress: (jobId, progress) => {
    const clamped = Math.min(1, Math.max(0, progress));
    set((state) => ({
      jobs: state.jobs.map((job) => (job.id === jobId ? { ...job, progress: clamped } : job)),
    }));
  },

  removeJob: (jobId) => {
    set((state) => ({ jobs: state.jobs.filter((job) => job.id !== jobId) }));
  },

  clearFinishedJobs: () => {
    set((state) => ({ jobs: state.jobs.filter((job) => !TERMINAL_STATES.includes(job.state)) }));
  },
}));
