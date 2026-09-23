import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Button } from '@/ui/components/button';
import { IconButton } from '@/ui/components/icon-button';

/**
 * 按钮类组件的测试重点（T0.5 要求：渲染 + 键盘交互 + 禁用态）：
 *   - loading 必须是"既禁用又告知"（aria-busy），否则屏幕阅读器会把点不动误解为故障；
 *   - asChild 渲染时不能把 disabled 透传到子元素（会变成非法 HTML 属性）；
 *   - IconButton 的无障碍名称来自 label，而不是图标。
 */
describe('Button', () => {
  it('默认渲染为原生按钮并响应点击', () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>提交</Button>);

    const button = screen.getByRole('button', { name: '提交' });
    fireEvent.click(button);

    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('键盘 Enter 触发点击（原生按钮语义）', () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>提交</Button>);

    const button = screen.getByRole('button', { name: '提交' });
    button.focus();
    fireEvent.keyDown(button, { key: 'Enter' });
    fireEvent.click(button);

    expect(button).toHaveFocus();
    expect(onClick).toHaveBeenCalled();
  });

  it('loading 时同时置 aria-busy 与 disabled，防止重复提交', () => {
    const onClick = vi.fn();
    render(
      <Button loading onClick={onClick}>
        推送中
      </Button>,
    );

    const button = screen.getByRole('button');
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute('aria-busy', 'true');
    expect(button).toHaveAttribute('data-loading');

    fireEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });

  it('disabled 时不触发点击', () => {
    const onClick = vi.fn();
    render(
      <Button disabled onClick={onClick}>
        不可用
      </Button>,
    );

    fireEvent.click(screen.getByRole('button'));
    expect(onClick).not.toHaveBeenCalled();
  });

  it('asChild 时把样式交给子元素，且不注入 disabled 属性', () => {
    render(
      <Button asChild>
        <a href="https://example.com">文档</a>
      </Button>,
    );

    const link = screen.getByRole('link', { name: '文档' });
    expect(link).not.toHaveAttribute('disabled');
    // 变体类名已透传到子元素（样式不因 asChild 而丢失）
    expect(link.className).toContain('bg-brand');
  });
});

describe('IconButton', () => {
  it('用 label 作为无障碍名称', () => {
    render(
      <IconButton label="删除分支">
        <span aria-hidden="true">×</span>
      </IconButton>,
    );

    expect(screen.getByRole('button', { name: '删除分支' })).toBeInTheDocument();
  });

  it('tooltip 缺省时以 label 作为 title', () => {
    render(
      <IconButton label="新增" tooltip="新增一个标签">
        <span aria-hidden="true">+</span>
      </IconButton>,
    );

    const button = screen.getByRole('button', { name: '新增' });
    expect(button).toHaveAttribute('title', '新增一个标签');
  });

  it('disabled 时不触发点击', () => {
    const onClick = vi.fn();
    render(
      <IconButton label="删除" disabled onClick={onClick}>
        <span aria-hidden="true">×</span>
      </IconButton>,
    );

    fireEvent.click(screen.getByRole('button', { name: '删除' }));
    expect(onClick).not.toHaveBeenCalled();
  });
});
