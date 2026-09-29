/**
 * "添加仓库"卡片（GIT-01/02/03）：仪表盘上的打开 / 克隆 / 初始化三入口。
 *
 * # 为什么放在仪表盘
 *
 * 首次启动时用户唯一会到的地方就是仪表盘（外壳默认路由），"把第一个仓库请进
 * 应用"天然属于这里；已打开的仓库走下方的最近列表。T1.3 的任务书把真实 UI
 * 挂到了 T1.4，而 T1.4 只交付了状态面板——这个卡片补上那次漏排（M2 后插队任务）。
 *
 * # 三条路径的形态差异
 *
 * - **打开 / 初始化**是同步命令：拿到 `OpenedRepository` 就能直接跳进工作区；
 * - **克隆**是长任务（JobRunner）：`repo_clone` 只返回 jobId，仓库记录要等
 *   `job:done` 事件的 `result.recordId`。任务跟踪照 `useSyncJobs` 的模式：
 *   pendingRef 过滤自己的任务，进度同步进全局 jobStore（仪表盘的任务卡片）。
 *
 * # 目录选择
 *
 * `pickFolder`（dialog 插件）在真实宿主里可用；e2e / 普通浏览器里返回 null，
 * 此时"浏览"按钮隐藏、手输路径照常工作——测试与降级走同一条输入框路径。
 */
import { useEffect, useRef, useState } from 'react';

import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

import { useAppError } from '@/lib/errors';
import { onJobDone, onJobFailed, onJobProgress, pickFolder } from '@/lib/ipc';
import { repoClone, repoInit, repoOpen } from '@/lib/ipc';
import { RECENT_REPOS_QUERY_KEY } from '@/lib/queryKeys';
import { Button } from '@/ui/components/button';
import { Input } from '@/ui/components/input';
import { ToggleGroup } from '@/ui/components/toggle-group';

import { useJobStore } from '@/stores/jobStore';
import { useUiStore } from '@/stores/uiStore';

/** 三个入口的模式标识。 */
const MODES = ['open', 'clone', 'init'] as const;
type Mode = (typeof MODES)[number];

/** `job:done` 事件里 `repo_clone` 结果的形状（后端 `OpenedRepositoryDto`）。 */
interface CloneJobResult {
  readonly recordId?: number;
}

/** 打开/初始化成功后的公共收尾：登记当前仓库 + 跳工作区。 */
function useOpenSucceeded() {
  const navigate = useNavigate();
  const setCurrentRepoId = useUiStore((state) => state.setCurrentRepoId);
  return (recordId: number) => {
    // 路由段与 store 里都是字符串（与 recentRepos 的约定一致）
    const id = String(recordId);
    setCurrentRepoId(id);
    void navigate(`/repo/${id}/status`);
  };
}

export function AddRepoCard() {
  const { t } = useTranslation('shell');
  const queryClient = useQueryClient();
  const { show } = useAppError();
  const openSucceeded = useOpenSucceeded();
  const enqueueJob = useJobStore((state) => state.enqueueJob);
  const setJobState = useJobStore((state) => state.setJobState);

  const [mode, setMode] = useState<Mode>('open');
  /** 当前克隆任务的 jobId（null = 没有在跟踪的克隆）；声明在订阅 effect 之前。 */
  const cloneJobIdRef = useRef<string | null>(null);
  const [path, setPath] = useState('');
  const [cloneUrl, setCloneUrl] = useState('');
  const [cloneTarget, setCloneTarget] = useState('');
  const [initPath, setInitPath] = useState('');
  const [initBranch, setInitBranch] = useState('');
  // 克隆任务进行中：等待 job:done，期间三个表单都禁用（同一份仓库记录表）
  const [cloneRunning, setCloneRunning] = useState(false);

  // 事件订阅只挂一次；回调里要的最新值经 ref 读取（与 useSyncJobs 同一模式）
  const doneRef = useRef<((recordId: number | null) => void) | null>(null);
  useEffect(() => {
    doneRef.current = (recordId) => {
      setCloneRunning(false);
      void queryClient.invalidateQueries({ queryKey: [RECENT_REPOS_QUERY_KEY] });
      if (recordId !== null) {
        openSucceeded(recordId);
      }
    };
  });

  useEffect(() => {
    let disposed = false;
    let unlistenDone: (() => void) | null = null;
    let unlistenFailed: (() => void) | null = null;
    const unlisteners: (() => void)[] = [];
    void onJobDone((payload) => {
      // 只关心克隆任务：同步任务的 result 形状不同，recordId 读取要隔离
      if (payload.jobId !== cloneJobIdRef.current) {
        return;
      }
      const result = payload.result as CloneJobResult | null;
      doneRef.current?.(typeof result?.recordId === 'number' ? result.recordId : null);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        unlistenDone = unlisten;
        unlisteners.push(unlisten);
      }
    });
    void onJobProgress((payload) => {
      if (payload.jobId !== cloneJobIdRef.current) {
        return;
      }
      // 克隆有总量（对象数），百分比可直接换算；缺总量时保持不确定态
      if (payload.total !== null && payload.total > 0) {
        useJobStore
          .getState()
          .setJobProgress(payload.jobId, Math.min(1, (payload.current ?? 0) / payload.total));
      }
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        unlisteners.push(unlisten);
      }
    });
    void onJobFailed((payload) => {
      if (payload.jobId !== cloneJobIdRef.current) {
        return;
      }
      cloneJobIdRef.current = null;
      doneRef.current?.(null);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        unlistenFailed = unlisten;
        unlisteners.push(unlisten);
      }
    });
    return () => {
      disposed = true;
      unlistenDone?.();
      unlistenFailed?.();
    };
  }, []);

  const invalidateRecent = () => {
    void queryClient.invalidateQueries({ queryKey: [RECENT_REPOS_QUERY_KEY] });
  };

  const open = useMutation({
    mutationFn: () => repoOpen(path.trim()),
    onSuccess: (opened) => {
      setPath('');
      invalidateRecent();
      openSucceeded(opened.recordId);
    },
    onError: (error) => show(error),
  });

  const clone = useMutation({
    mutationFn: () => {
      const url = cloneUrl.trim();
      const into = cloneTarget.trim();
      return repoClone({ url, into });
    },
    onSuccess: (jobRef) => {
      // 走全局 jobStore 投影（仪表盘任务卡片）；细节进度由本组件的事件订阅消费
      enqueueJob({
        id: jobRef.jobId,
        kind: 'clone',
        label: t('dashboard.addRepo.mode.clone'),
      });
      setJobState(jobRef.jobId, 'running');
      cloneJobIdRef.current = jobRef.jobId;
      setCloneRunning(true);
    },
    onError: (error) => show(error),
  });

  const init = useMutation({
    mutationFn: () =>
      repoInit({
        path: initPath.trim(),
        // 初始分支留空走后端缺省（init.defaultBranch / main）
        ...(initBranch.trim() === '' ? {} : { initialBranch: initBranch.trim() }),
      }),
    onSuccess: (opened) => {
      setInitPath('');
      setInitBranch('');
      invalidateRecent();
      openSucceeded(opened.recordId);
    },
    onError: (error) => show(error),
  });

  const busy = open.isPending || init.isPending || cloneRunning;

  /** 浏览目录：拿不到选择器（非宿主环境）时按钮不渲染，手输是唯一路径。 */
  const browse = async (apply: (folder: string) => void): Promise<void> => {
    const folder = await pickFolder(t('dashboard.addRepo.pickTitle'));
    if (folder !== null) {
      apply(folder);
    }
  };

  const canBrowse = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  return (
    <article className="rounded-lg border border-line bg-surface p-4" data-testid="add-repo-card">
      <h2 className="text-14 font-medium">{t('dashboard.addRepo.title')}</h2>

      <div className="mt-2">
        <ToggleGroup
          label={t('dashboard.addRepo.modeLabel')}
          value={mode}
          options={MODES.map((item) => ({
            value: item,
            label: t(`dashboard.addRepo.mode.${item}`),
          }))}
          onValueChange={(next) => setMode(next as Mode)}
        />
      </div>

      {mode === 'open' ? (
        <div className="mt-3 flex flex-col gap-2" data-testid="add-repo-open">
          <div className="flex gap-2">
            <Input
              value={path}
              placeholder={t('dashboard.addRepo.pathPlaceholder')}
              onChange={(event) => setPath(event.target.value)}
              data-testid="add-repo-path"
            />
            {canBrowse ? (
              <Button
                variant="ghost"
                onClick={() => void browse(setPath)}
                data-testid="add-repo-browse"
              >
                {t('dashboard.addRepo.browse')}
              </Button>
            ) : null}
          </div>
          <Button
            disabled={busy || path.trim() === ''}
            onClick={() => open.mutate()}
            data-testid="add-repo-open-submit"
          >
            {t('dashboard.addRepo.openSubmit')}
          </Button>
        </div>
      ) : null}

      {mode === 'clone' ? (
        <div className="mt-3 flex flex-col gap-2" data-testid="add-repo-clone">
          <Input
            value={cloneUrl}
            placeholder={t('dashboard.addRepo.urlPlaceholder')}
            onChange={(event) => setCloneUrl(event.target.value)}
            data-testid="add-repo-url"
          />
          <div className="flex gap-2">
            <Input
              value={cloneTarget}
              placeholder={t('dashboard.addRepo.targetPlaceholder')}
              onChange={(event) => setCloneTarget(event.target.value)}
              data-testid="add-repo-clone-target"
            />
            {canBrowse ? (
              <Button
                variant="ghost"
                onClick={() => void browse(setCloneTarget)}
                data-testid="add-repo-clone-browse"
              >
                {t('dashboard.addRepo.browse')}
              </Button>
            ) : null}
          </div>
          <Button
            disabled={busy || cloneUrl.trim() === '' || cloneTarget.trim() === ''}
            onClick={() => clone.mutate()}
            data-testid="add-repo-clone-submit"
          >
            {t('dashboard.addRepo.cloneSubmit')}
          </Button>
          {cloneRunning ? (
            <p className="text-12 text-fg-subtle" role="status" data-testid="add-repo-clone-status">
              {t('dashboard.addRepo.cloneRunning')}
            </p>
          ) : null}
        </div>
      ) : null}

      {mode === 'init' ? (
        <div className="mt-3 flex flex-col gap-2" data-testid="add-repo-init">
          <div className="flex gap-2">
            <Input
              value={initPath}
              placeholder={t('dashboard.addRepo.initPathPlaceholder')}
              onChange={(event) => setInitPath(event.target.value)}
              data-testid="add-repo-init-path"
            />
            {canBrowse ? (
              <Button
                variant="ghost"
                onClick={() => void browse(setInitPath)}
                data-testid="add-repo-init-browse"
              >
                {t('dashboard.addRepo.browse')}
              </Button>
            ) : null}
          </div>
          <Input
            value={initBranch}
            placeholder={t('dashboard.addRepo.branchPlaceholder')}
            onChange={(event) => setInitBranch(event.target.value)}
            data-testid="add-repo-init-branch"
          />
          <Button
            disabled={busy || initPath.trim() === ''}
            onClick={() => init.mutate()}
            data-testid="add-repo-init-submit"
          >
            {t('dashboard.addRepo.initSubmit')}
          </Button>
        </div>
      ) : null}
    </article>
  );
}
