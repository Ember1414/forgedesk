// i18n-ignore-file
// 开发专用面板（只挂在 dev 构建的组件页上），文案面向开发者，
// 因此整文件豁免 i18n:lint；正式界面一律走 i18n key。
// 注意：该标记必须出现在文件前 6 行内（检查脚本只扫文件头部）。
import { useCallback, useEffect, useState } from 'react';

import { useQuery, useQueryClient } from '@tanstack/react-query';

import { Button } from '@/ui/components/button';
import { Input } from '@/ui/components/input';
import { SelectField } from '@/ui/components/select';
import { useAppError } from '@/lib/errors';
import {
  cancelJob,
  isTauriRuntime,
  onJobDone,
  onJobFailed,
  onJobProgress,
  progressPercent,
  repoClone,
  repoClose,
  repoDiscover,
  repoForget,
  repoInit,
  repoOpen,
  repoRecentList,
  type JobProgressPayload,
} from '@/lib/ipc';

const GITIGNORE_OPTIONS = [
  { value: 'rust', label: 'rust' },
  { value: 'node', label: 'node' },
  { value: 'python', label: 'python' },
  { value: 'go', label: 'go' },
  { value: 'java', label: 'java' },
];

const LICENSE_OPTIONS = [
  // Radix 的 SelectItem 不接受空字符串值，因此"不生成"用一个哨兵值
  { value: 'none', label: '不生成' },
  { value: 'MIT', label: 'MIT' },
  { value: 'Apache-2.0', label: 'Apache-2.0' },
  { value: 'BSD-3-Clause', label: 'BSD-3-Clause' },
];

/** 一条任务在界面上的记录（只用于这个验证面板）。 */
interface TrackedJob {
  readonly id: string;
  readonly label: string;
  phase: string;
  percent: number | null;
  state: 'running' | 'succeeded' | 'failed';
  detail: string;
}

/**
 * 仓库生命周期的最小调用验证（T1.3）。
 *
 * 为什么放在开发页面而不是真实界面：T1.3 的交付物是**后端能力与契约**，
 * 真实 UI 属于 T1.4。这里的目标是"每条命令都能被点一次、结果都能看到"，
 * 从而在没有正式界面的情况下验证 IPC 通路、错误形状与事件载荷。
 */
export function RepositoryLifecyclePanel() {
  const { show } = useAppError();
  const [path, setPath] = useState('');
  const [cloneUrl, setCloneUrl] = useState('');
  const [cloneInto, setCloneInto] = useState('');
  const [cloneDepth, setCloneDepth] = useState('');
  const [gitignore, setGitignore] = useState('rust');
  const [license, setLicense] = useState('none');
  const [output, setOutput] = useState('（还没有调用）');
  const [jobs, setJobs] = useState<readonly TrackedJob[]>([]);
  const queryClient = useQueryClient();

  // 最近仓库是**服务端状态**，走 TanStack Query（AGENTS §6 的单一真相源约定）：
  // 自己用 useEffect + useState 拉取会踩上 react-hooks 的 set-state-in-effect，
  // 而且要在每个改动点手写失效逻辑。
  const recentQuery = useQuery({
    queryKey: ['repo', 'recent'],
    queryFn: () => repoRecentList(),
    enabled: isTauriRuntime(),
    retry: 0,
  });
  const recent = recentQuery.data ?? [];

  const refresh = useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ['repo', 'recent'] });
  }, [queryClient]);

  useEffect(() => {
    if (recentQuery.isError) {
      show(recentQuery.error);
    }
  }, [recentQuery.error, recentQuery.isError, show]);

  // 订阅三条任务事件；卸载时必须 unlisten，否则监听器会叠加
  useEffect(() => {
    if (!isTauriRuntime()) {
      return undefined;
    }
    const unlisteners: Array<() => void> = [];
    let disposed = false;

    const patchJob = (jobId: string, patch: Partial<TrackedJob>) => {
      setJobs((current) => current.map((job) => (job.id === jobId ? { ...job, ...patch } : job)));
    };

    void (async () => {
      const unlistenProgress = await onJobProgress((payload: JobProgressPayload) => {
        patchJob(payload.jobId, {
          phase: payload.phase,
          percent: progressPercent(payload),
          state: 'running',
        });
      });
      const unlistenDone = await onJobDone((payload) => {
        patchJob(payload.jobId, {
          state: 'succeeded',
          percent: 100,
          detail: JSON.stringify(payload.result).slice(0, 400),
        });
        void refresh();
      });
      const unlistenFailed = await onJobFailed((payload) => {
        patchJob(payload.jobId, {
          state: 'failed',
          detail: JSON.stringify(payload.error).slice(0, 400),
        });
        show(payload.error);
      });

      // 组件可能在 await 期间就卸载了，此时必须立刻退订
      if (disposed) {
        unlistenProgress();
        unlistenDone();
        unlistenFailed();
        return;
      }
      unlisteners.push(unlistenProgress, unlistenDone, unlistenFailed);
    })();

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => {
        unlisten();
      });
    };
  }, [refresh, show]);

  /** 统一处理"调用 → 展示结果 / 展示错误"的样板。 */
  const run = async (label: string, action: () => Promise<unknown>): Promise<void> => {
    if (!isTauriRuntime()) {
      setOutput(`${label}：需要在 Tauri 宿主中运行（当前是浏览器预览）`);
      return;
    }
    try {
      const result = await action();
      setOutput(`${label}：\n${JSON.stringify(result, null, 2)}`);
    } catch (error) {
      setOutput(`${label} 失败：\n${JSON.stringify(error, null, 2)}`);
      show(error);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <p className="text-12 text-fg-subtle">
        真实 UI 属于 T1.4；这里只验证命令通路、错误形状与任务事件。所有输入都会
        在后端二次校验（非法输入返回 VALIDATION）。
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={path}
          onChange={(event) => {
            setPath(event.target.value);
          }}
          placeholder="仓库路径（可用 E:\\Projects\\ForgeDesk 或某个子目录）"
          className="min-w-72 flex-1"
        />
        <Button
          variant="secondary"
          onClick={() => {
            void run('repo_discover', () => repoDiscover(path));
          }}
        >
          发现
        </Button>
        <Button
          variant="secondary"
          onClick={() => {
            void run('repo_open', () => repoOpen(path));
          }}
        >
          打开
        </Button>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <SelectField
          label=".gitignore 模板"
          value={gitignore}
          onValueChange={setGitignore}
          options={GITIGNORE_OPTIONS}
          className="w-40"
        />
        <SelectField
          label="许可证模板"
          value={license}
          onValueChange={setLicense}
          options={LICENSE_OPTIONS}
          className="w-48"
        />
        <Button
          variant="secondary"
          onClick={() => {
            void run('repo_init', () =>
              repoInit({
                path,
                gitignore,
                ...(license === 'none' ? {} : { license, licenseHolder: 'ForgeDesk user' }),
              }),
            );
          }}
        >
          初始化（含模板）
        </Button>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={cloneUrl}
          onChange={(event) => {
            setCloneUrl(event.target.value);
          }}
          placeholder="远端 URL（HTTPS 或 SSH）"
          className="min-w-64 flex-1"
        />
        <Input
          value={cloneInto}
          onChange={(event) => {
            setCloneInto(event.target.value);
          }}
          placeholder="克隆到（目标目录）"
          className="min-w-56 flex-1"
        />
        <Input
          value={cloneDepth}
          onChange={(event) => {
            setCloneDepth(event.target.value);
          }}
          placeholder="depth"
          inputMode="numeric"
          className="w-20"
        />
        <Button
          onClick={() => {
            if (!isTauriRuntime()) {
              setOutput('repo_clone：需要在 Tauri 宿主中运行（当前是浏览器预览）');
              return;
            }
            const depth = Number.parseInt(cloneDepth, 10);
            const label = `克隆 ${cloneInto}`;
            void repoClone({
              url: cloneUrl,
              into: cloneInto,
              ...(Number.isFinite(depth) && depth > 0 ? { depth } : {}),
            })
              .then((ref) => {
                setJobs((current) => [
                  ...current,
                  {
                    id: ref.jobId,
                    label,
                    phase: 'queued',
                    percent: null,
                    state: 'running',
                    detail: '',
                  },
                ]);
                setOutput(`repo_clone 已入队：${ref.jobId}`);
              })
              .catch((error: unknown) => {
                show(error);
              });
          }}
        >
          克隆（长任务）
        </Button>
      </div>

      <div>
        <h3 className="text-13 font-medium">任务（job:progress / job:done / job:failed）</h3>
        {jobs.length === 0 ? (
          <p className="mt-1 text-12 text-fg-subtle">还没有任务。</p>
        ) : (
          <ul className="mt-2 flex flex-col gap-2">
            {jobs.map((job) => (
              <li
                key={job.id}
                className="flex flex-wrap items-center gap-2 rounded-md border border-line bg-surface-sunken px-3 py-2 text-12"
              >
                <span className="font-mono">{job.id.slice(0, 8)}</span>
                <span>{job.label}</span>
                <span className="text-fg-subtle">{job.phase}</span>
                <span>{job.percent === null ? '—' : `${String(job.percent)}%`}</span>
                <span className="text-fg-subtle">{job.state}</span>
                {job.state === 'running' ? (
                  <Button
                    variant="danger"
                    size="sm"
                    onClick={() => {
                      void cancelJob(job.id)
                        .then((wasRunning) => {
                          setOutput(`job_cancel ${job.id} → ${String(wasRunning)}`);
                        })
                        .catch(show);
                    }}
                  >
                    取消
                  </Button>
                ) : null}
                {job.detail === '' ? null : (
                  <span className="w-full break-all text-fg-subtle">{job.detail}</span>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>

      <div>
        <div className="flex items-center gap-2">
          <h3 className="text-13 font-medium">最近仓库（repo_recent_list）</h3>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              void refresh();
            }}
          >
            刷新
          </Button>
        </div>
        {recent.length === 0 ? (
          <p className="mt-1 text-12 text-fg-subtle">列表为空。</p>
        ) : (
          <ul className="mt-2 flex flex-col gap-2">
            {recent.map((item) => (
              <li
                key={item.id}
                className="flex flex-wrap items-center gap-2 rounded-md border border-line bg-surface-sunken px-3 py-2 text-12"
              >
                <span className="font-mono">{item.id}</span>
                <span className="font-medium">{item.name}</span>
                <span className="text-fg-subtle">{item.path}</span>
                <span className="text-fg-subtle">
                  {item.defaultBranch ?? '—'} · {item.isOpen ? '已打开' : '未打开'}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    void run('repo_open', () => repoOpen(item.path)).then(() => refresh());
                  }}
                >
                  打开
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    void run('repo_close', () => repoClose(item.id)).then(() => refresh());
                  }}
                >
                  关闭
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    void run('repo_forget', () => repoForget(item.id)).then(() => refresh());
                  }}
                >
                  从列表移除
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <pre className="max-h-64 overflow-auto rounded-md border border-line bg-surface-sunken p-3 text-12 whitespace-pre-wrap">
        {output}
      </pre>
    </div>
  );
}
