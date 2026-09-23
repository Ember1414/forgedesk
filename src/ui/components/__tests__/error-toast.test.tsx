import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { ErrorToastContent } from '@/ui/components/error-toast';

/**
 * 错误提示内容的测试重点：
 *   - **详情默认折叠**：技术细节不该淹没"发生了什么"，但必须能展开（否则用户无法自助排查）；
 *   - **修复动作可点击**：按钮要真的能按，且点击把动作 id 交回给调用方；
 *   - **纯文本渲染**：detail 只能作为文本出现，绝不能被当成 HTML 注入
 *     （红线 R8 的另一面：脱敏之后再谈渲染安全，两层都不能少）。
 */
describe('ErrorToastContent', () => {
  it('渲染标题与建议', () => {
    render(<ErrorToastContent title="网络请求失败" hint="检查网络或代理设置后重试" />);

    expect(screen.getByText('网络请求失败')).toBeInTheDocument();
    expect(screen.getByText('检查网络或代理设置后重试')).toBeInTheDocument();
  });

  it('详情默认折叠，展开后可见原文', () => {
    const { container } = render(
      <ErrorToastContent
        title="推送被拒绝"
        detail="remote: rejected\nstatus: non-fast-forward"
        detailsLabel="详情"
      />,
    );

    const details = container.querySelector('details');
    expect(details).not.toBeNull();
    expect(details).not.toHaveAttribute('open');
    expect(screen.getByText('详情')).toBeInTheDocument();
    expect(screen.getByText(/non-fast-forward/)).toBeInTheDocument();
  });

  it('没有详情时不渲染折叠区（避免出现空的"详情"入口）', () => {
    const { container } = render(<ErrorToastContent title="权限不足" />);
    expect(container.querySelector('details')).toBeNull();
  });

  it('详情按纯文本渲染，脚本不会被当作 HTML 执行', () => {
    const payload = '<img src=x onerror="alert(1)">';
    render(<ErrorToastContent title="内部错误" detail={payload} detailsLabel="详情" />);

    // 文本被原样呈现，且 DOM 里没有注入出来的元素
    expect(screen.getByText(payload)).toBeInTheDocument();
    expect(document.querySelector('img')).toBeNull();
  });

  it('动作按钮可点击，并把动作 id 交回调用方', () => {
    const onAction = vi.fn();
    render(
      <ErrorToastContent
        title="状态已过期"
        actions={[
          { id: 'refresh', label: '刷新状态' },
          { id: 'copy', label: '复制等价命令', disabled: true },
        ]}
        onAction={onAction}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: '刷新状态' }));
    expect(onAction).toHaveBeenCalledWith('refresh');

    const disabled = screen.getByRole('button', { name: '复制等价命令' });
    expect(disabled).toBeDisabled();
    fireEvent.click(disabled);
    expect(onAction).toHaveBeenCalledTimes(1);
  });

  it('没有动作时不渲染动作区', () => {
    render(<ErrorToastContent title="内部错误" actions={[]} />);
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });
});
