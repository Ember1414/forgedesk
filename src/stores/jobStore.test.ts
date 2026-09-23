import { beforeEach, describe, expect, it } from 'vitest';

import { countActiveJobs, initialJobState, isJobFinished, useJobStore } from '@/stores/jobStore';

/**
 * jobStore 测试。
 *
 * 这些不变量直接决定状态栏上那个数字是否可信：
 *   - 终态任务不计入"进行中"；
 *   - 成功任务进度钉在 100%（避免进度条停在中间误导用户）；
 *   - 重复 id 不产生重复行（后端事件重放时不会出现两行）。
 */
beforeEach(() => {
  useJobStore.setState(initialJobState);
});

describe('enqueueJob', () => {
  it('入队后是 queued 且没有进度', () => {
    useJobStore.getState().enqueueJob({ id: 'job-1', kind: 'fetch', label: '获取远程更新' });
    const [job] = useJobStore.getState().jobs;

    expect(job).toBeDefined();
    expect(job?.state).toBe('queued');
    expect(job?.progress).toBeNull();
    expect(job?.finishedAt).toBeNull();
  });

  it('同 id 重复入队被忽略（事件重放不会出现重复行）', () => {
    useJobStore.getState().enqueueJob({ id: 'job-1', kind: 'fetch', label: '第一次' });
    useJobStore.getState().enqueueJob({ id: 'job-1', kind: 'fetch', label: '第二次' });

    expect(useJobStore.getState().jobs).toHaveLength(1);
    expect(useJobStore.getState().jobs[0]?.label).toBe('第一次');
  });
});

describe('进度', () => {
  it('进度被限制在 0–1', () => {
    useJobStore.getState().enqueueJob({ id: 'job-1', kind: 'clone', label: '克隆' });
    useJobStore.getState().setJobProgress('job-1', 1.8);
    expect(useJobStore.getState().jobs[0]?.progress).toBe(1);

    useJobStore.getState().setJobProgress('job-1', -0.5);
    expect(useJobStore.getState().jobs[0]?.progress).toBe(0);
  });

  it('未知任务 id 不影响其它任务', () => {
    useJobStore.getState().enqueueJob({ id: 'job-1', kind: 'clone', label: '克隆' });
    useJobStore.getState().setJobProgress('unknown', 0.5);
    expect(useJobStore.getState().jobs[0]?.progress).toBeNull();
  });
});

describe('状态流转', () => {
  it('成功时进度钉在 1 并记录结束时间', () => {
    useJobStore.getState().enqueueJob({ id: 'job-1', kind: 'push', label: '推送' });
    useJobStore.getState().setJobState('job-1', 'running');
    useJobStore.getState().setJobState('job-1', 'succeeded');

    const [job] = useJobStore.getState().jobs;
    expect(job?.state).toBe('succeeded');
    expect(job?.progress).toBe(1);
    expect(job?.finishedAt).not.toBeNull();
    expect(job === undefined ? false : isJobFinished(job)).toBe(true);
  });

  it('失败不会把进度改成 1（保留真实进度，便于判断卡在哪）', () => {
    useJobStore.getState().enqueueJob({ id: 'job-1', kind: 'push', label: '推送' });
    useJobStore.getState().setJobProgress('job-1', 0.42);
    useJobStore.getState().setJobState('job-1', 'failed');

    const [job] = useJobStore.getState().jobs;
    expect(job?.progress).toBe(0.42);
    expect(job?.finishedAt).not.toBeNull();
  });
});

describe('进行中计数与清理', () => {
  it('只统计 queued 与 running', () => {
    const { enqueueJob, setJobState } = useJobStore.getState();
    enqueueJob({ id: 'a', kind: 'fetch', label: 'A' });
    enqueueJob({ id: 'b', kind: 'fetch', label: 'B' });
    enqueueJob({ id: 'c', kind: 'fetch', label: 'C' });
    setJobState('b', 'running');
    setJobState('c', 'succeeded');

    expect(countActiveJobs(useJobStore.getState().jobs)).toBe(2);
  });

  it('removeJob 与 clearFinishedJobs', () => {
    const { enqueueJob, setJobState, removeJob, clearFinishedJobs } = useJobStore.getState();
    enqueueJob({ id: 'a', kind: 'fetch', label: 'A' });
    enqueueJob({ id: 'b', kind: 'fetch', label: 'B' });
    setJobState('a', 'succeeded');

    removeJob('b');
    expect(useJobStore.getState().jobs.map((job) => job.id)).toEqual(['a']);

    clearFinishedJobs();
    expect(useJobStore.getState().jobs).toHaveLength(0);
  });
});
