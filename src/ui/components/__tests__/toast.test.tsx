import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { Toaster } from '@/ui/components/toast';
import { initialToastState, pushToast, useToastStore } from '@/stores/toastStore';

/**
 * 轻提示的测试重点：
 *   - 队列上限（提示不能无限堆叠盖住界面）；
 *   - 错误提示不会被自动关掉（一闪而过的错误等于没提示）；
 *   - 关闭按钮有无障碍名称、动作按钮可执行。
 */
beforeEach(() => {
  useToastStore.setState(initialToastState);
});

afterEach(() => {
  useToastStore.setState(initialToastState);
});

describe('toastStore', () => {
  it('push 后按顺序入队并返回 id', () => {
    const id = pushToast({ tone: 'success', title: '推送完成' });

    expect(id).toMatch(/^toast-/);
    expect(useToastStore.getState().toasts).toHaveLength(1);
    expect(useToastStore.getState().toasts[0]?.title).toBe('推送完成');
  });

  it('默认 tone 为 info，默认会自动消失', () => {
    pushToast({ title: '获取中' });
    const [toast] = useToastStore.getState().toasts;

    expect(toast?.tone).toBe('info');
    expect(toast?.duration).toBeGreaterThan(0);
  });

  it('duration 可显式设为 0（错误提示需要用户处理）', () => {
    pushToast({ tone: 'danger', title: '推送被拒绝', duration: 0 });
    expect(useToastStore.getState().toasts[0]?.duration).toBe(0);
  });

  it('超出上限时丢弃最早的一条', () => {
    for (let index = 0; index < 6; index += 1) {
      pushToast({ title: `提示 ${String(index)}` });
    }

    const titles = useToastStore.getState().toasts.map((toast) => toast.title);
    expect(titles).toHaveLength(4);
    expect(titles).not.toContain('提示 0');
    expect(titles).toContain('提示 5');
  });

  it('dismiss 只移除指定的一条，clear 清空全部', () => {
    const first = pushToast({ title: 'A' });
    pushToast({ title: 'B' });

    useToastStore.getState().dismissToast(first);
    expect(useToastStore.getState().toasts.map((toast) => toast.title)).toEqual(['B']);

    useToastStore.getState().clearToasts();
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });
});

describe('Toaster', () => {
  it('渲染标题、说明与关闭按钮（关闭按钮有无障碍名称）', async () => {
    // duration: 0 —— 测试里不引入自动消失定时器，避免定时器在用例结束后触发更新
    pushToast({
      tone: 'success',
      title: '推送完成',
      description: 'main → origin/main',
      duration: 0,
    });
    render(<Toaster closeLabel="关闭提示" />);

    expect(await screen.findByText('推送完成')).toBeInTheDocument();
    expect(screen.getByText('main → origin/main')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '关闭提示' })).toBeInTheDocument();
  });

  it('点击关闭按钮把提示移出队列', async () => {
    pushToast({ title: '获取中', duration: 0 });
    render(<Toaster closeLabel="关闭提示" />);

    fireEvent.click(await screen.findByRole('button', { name: '关闭提示' }));

    await waitFor(() => {
      expect(useToastStore.getState().toasts).toHaveLength(0);
    });
  });

  it('动作按钮可执行前端回调', async () => {
    const onClick = vi.fn();
    pushToast({
      tone: 'danger',
      title: '推送被拒绝',
      duration: 0,
      actions: [{ id: 'retry', label: '重试', onClick }],
    });
    render(<Toaster closeLabel="关闭提示" />);

    fireEvent.click(await screen.findByRole('button', { name: '重试' }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('命令型动作经 IPC 执行，成功后给出反馈', async () => {
    // invokeCommand 在 jsdom 里没有 Tauri 宿主，会抛错；
    // 这里只断言"点击不会静默"——真实链路由桌面宿主下的手工验收覆盖。
    const error = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    pushToast({
      tone: 'danger',
      title: '状态已过期',
      duration: 0,
      actions: [{ id: 'refresh', label: '刷新状态', command: 'app_version' }],
    });
    render(<Toaster closeLabel="关闭提示" />);

    fireEvent.click(await screen.findByRole('button', { name: '刷新状态' }));

    // 失败时应产生新的错误提示（而不是毫无反应）
    await waitFor(() => {
      expect(useToastStore.getState().toasts.length).toBeGreaterThan(1);
    });
    error.mockRestore();
  });

  it('无提示时不渲染任何内容', () => {
    const { container } = render(<Toaster closeLabel="关闭提示" />);
    expect(container.querySelectorAll('[role="status"]')).toHaveLength(0);
  });
});
