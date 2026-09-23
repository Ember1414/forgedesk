/**
 * IPC 封装的契约测试。
 *
 * 为什么值得测这些"薄封装"：命令名与参数名是**跨语言契约**——前端写错一个字母，
 * TypeScript 完全看不出来（`invoke` 的签名是 `(command: string, args?: …)`），
 * 只有运行时才会以 "Command xxx not found" 或"参数反序列化失败"的形式暴露。
 * 这里把每个封装的命令名与参数形状钉住，并顺带断言 DTO 字段用 camelCase
 * （与 Rust 侧 `serde(rename_all = "camelCase")` 对应）。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>();
const listen =
  vi.fn<(event: string, handler: (message: { payload: unknown }) => void) => Promise<() => void>>();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (message: { payload: unknown }) => void) =>
    listen(event, handler),
}));

const {
  cancelJob,
  invokeCommand,
  isTauriRuntime,
  listenEvent,
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
} = await import('@/lib/ipc');

beforeEach(() => {
  invoke.mockReset();
  listen.mockReset();
  invoke.mockResolvedValue(null);
  listen.mockResolvedValue(() => undefined);
  delete (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'];
});

describe('仓库命令', () => {
  it('repo_discover 只传 path', async () => {
    await repoDiscover('E:\\Projects\\ForgeDesk');
    expect(invoke).toHaveBeenCalledWith('repo_discover', { path: 'E:\\Projects\\ForgeDesk' });
  });

  it('repo_open 只传 path', async () => {
    await repoOpen('/tmp/repo');
    expect(invoke).toHaveBeenCalledWith('repo_open', { path: '/tmp/repo' });
  });

  it('repo_clone 把整份 spec 作为单个参数传下去', async () => {
    await repoClone({ url: 'https://example.com/a.git', into: '/tmp/a', depth: 1 });
    expect(invoke).toHaveBeenCalledWith('repo_clone', {
      spec: { url: 'https://example.com/a.git', into: '/tmp/a', depth: 1 },
    });
  });

  it('repo_init 把整份 spec 作为单个参数传下去', async () => {
    await repoInit({ path: '/tmp/new', gitignore: 'rust', license: 'MIT', licenseHolder: 'Ada' });
    expect(invoke).toHaveBeenCalledWith('repo_init', {
      spec: { path: '/tmp/new', gitignore: 'rust', license: 'MIT', licenseHolder: 'Ada' },
    });
  });

  it('repo_recent_list 缺省不带 limit，显式传值时才带', async () => {
    await repoRecentList();
    expect(invoke).toHaveBeenLastCalledWith('repo_recent_list', {});

    await repoRecentList(10);
    expect(invoke).toHaveBeenLastCalledWith('repo_recent_list', { limit: 10 });
  });

  it('repo_forget 与 repo_close 用 repoId（camelCase）', async () => {
    await repoForget(3);
    expect(invoke).toHaveBeenLastCalledWith('repo_forget', { repoId: 3 });

    await repoClose(4);
    expect(invoke).toHaveBeenLastCalledWith('repo_close', { repoId: 4 });
  });
});

describe('长任务', () => {
  it('cancelJob 用 jobId（camelCase）并返回后端给的可取消结果', async () => {
    invoke.mockResolvedValue(true);
    await expect(cancelJob('job-1')).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith('job_cancel', { jobId: 'job-1' });
  });

  it('订阅使用文档里的三个事件名', async () => {
    await onJobProgress(() => undefined);
    expect(listen.mock.calls[0]?.[0]).toBe('job:progress');

    await onJobDone(() => undefined);
    expect(listen.mock.calls[1]?.[0]).toBe('job:done');

    await onJobFailed(() => undefined);
    expect(listen.mock.calls[2]?.[0]).toBe('job:failed');
  });

  it('listenEvent 只把载荷交给回调，并原样返回 unlisten', async () => {
    const unlisten = (): void => undefined;
    listen.mockResolvedValue(unlisten);
    const received: unknown[] = [];

    const result = await listenEvent<{ jobId: string }>('job:progress', (payload) => {
      received.push(payload);
    });

    expect(result).toBe(unlisten);
    // 模拟 Tauri 投递一次事件：回调应拿到 payload 而不是整个 Event 包装
    const handler = listen.mock.calls[0]?.[1];
    handler?.({ payload: { jobId: 'job-1' } });
    expect(received).toEqual([{ jobId: 'job-1' }]);
  });

  it('progressPercent 在总量未知或为零时返回 null（渲染不确定进度条）', () => {
    expect(progressPercent({ jobId: 'a', phase: 'counting', current: 3, total: null })).toBeNull();
    expect(progressPercent({ jobId: 'a', phase: 'counting', current: null, total: 9 })).toBeNull();
    expect(progressPercent({ jobId: 'a', phase: 'counting', current: 1, total: 0 })).toBeNull();
  });

  it('progressPercent 计算并夹在 0–100', () => {
    expect(progressPercent({ jobId: 'a', phase: 'receiving', current: 3, total: 12 })).toBe(25);
    expect(progressPercent({ jobId: 'a', phase: 'receiving', current: 99, total: 12 })).toBe(100);
    expect(progressPercent({ jobId: 'a', phase: 'receiving', current: -1, total: 12 })).toBe(0);
  });
});

describe('客户端', () => {
  it('invokeCommand 直接透传命令名与参数', async () => {
    await invokeCommand('app_version');
    expect(invoke).toHaveBeenCalledWith('app_version', undefined);
  });

  it('isTauriRuntime 依据宿主注入的全局对象判断', () => {
    expect(isTauriRuntime()).toBe(false);
    (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'] = {};
    expect(isTauriRuntime()).toBe(true);
  });
});
