import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ReadmeDialog } from '@/features/github/ReadmeDialog';
import { repoRemoteReadme } from '@/lib/ipc';
import type { RemoteRepo } from '@/lib/ipc';
import { initialToastState, useToastStore } from '@/stores/toastStore';

/**
 * README 对话框（T4.6）。
 *
 * XSS 的主战场在 Rust 侧（services::readme 的白名单用例）；这里钉住
 * 前端契约：后端给什么就渲染什么（不再二次解析）、404 有明确文案、
 * 外链点击被拦截成复制而不是 webview 导航。
 */
vi.mock('@/lib/ipc', () => ({
  repoRemoteReadme: vi.fn(),
}));

const readmeMock = vi.mocked(repoRemoteReadme);

const repo: RemoteRepo = {
  id: 1,
  owner: 'octocat',
  name: 'Hello-World',
  fullName: 'octocat/Hello-World',
  htmlUrl: 'https://github.com/octocat/Hello-World',
  private: false,
  fork: false,
  stars: 3,
};

function renderDialog(target: RemoteRepo | null = repo): void {
  render(<ReadmeDialog repo={target} onOpenChange={() => undefined} />);
}

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
});

afterEach(() => {
  useToastStore.setState(initialToastState);
});

describe('ReadmeDialog — 安全渲染', () => {
  it('渲染后端给定的消毒 HTML（标题与正文可见）', async () => {
    readmeMock.mockResolvedValue(
      '<h1>ForgeDesk</h1><p>一段安全的正文</p><a href="https://example.com">链接</a>',
    );
    renderDialog();

    await waitFor(() => {
      expect(screen.getByTestId('readme-content')).toBeVisible();
    });
    expect(screen.getByRole('heading', { name: 'ForgeDesk' })).toBeVisible();
    expect(screen.getByText('一段安全的正文')).toBeVisible();
  });

  it('即使后端产物含活动内容，注入也不会执行（纵深防御断言）', async () => {
    const alert = vi.fn();
    // jsdom 下 innerHTML 注入的 script 不执行、事件属性不绑定——
    // 这正是"后端消毒 + innerHTML 渲染"组合的安全模型；本用例把这个
    // 模型钉进测试，防止将来有人换成会执行脚本的渲染方式
    vi.stubGlobal('alert', alert);
    readmeMock.mockResolvedValue(
      '<p>ok</p><script>window.__pwned = true; alert(1)</script><img src="x" onerror="alert(1)">',
    );
    renderDialog();

    await waitFor(() => {
      expect(screen.getByTestId('readme-content')).toBeVisible();
    });
    await Promise.resolve();
    expect(alert).not.toHaveBeenCalled();
    expect((window as { __pwned?: boolean }).__pwned).toBeUndefined();
    vi.unstubAllGlobals();
  });

  it('NOT_FOUND 时显示"没有 README"', async () => {
    readmeMock.mockRejectedValue({ code: 'NOT_FOUND', message: 'Not Found' });
    renderDialog();

    await waitFor(() => {
      expect(screen.getByText('该仓库没有 README。')).toBeVisible();
    });
  });

  it('点击正文里的链接被拦截为复制，不发生 webview 导航', async () => {
    readmeMock.mockResolvedValue('<p><a href="https://example.com/doc">文档</a></p>');
    renderDialog();

    await waitFor(() => {
      expect(screen.getByText('文档')).toBeVisible();
    });
    const click = fireEvent.click(screen.getByText('文档'), { cancelable: true });
    expect(click).toBe(false);
    await waitFor(() => {
      expect(useToastStore.getState().toasts.length).toBeGreaterThan(0);
    });
  });
});
