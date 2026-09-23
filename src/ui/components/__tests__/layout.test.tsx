import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Resizable } from '@/ui/components/resizable';
import { SplitPane } from '@/ui/components/split-pane';

/**
 * 布局类组件的测试重点：**键盘可调**与 ARIA 数值同步。
 *
 * 拖拽是最容易实现、也最容易忘记键盘替代的交互：
 * 只用鼠标能做、键盘做不了的"调整宽度"，对键盘用户等于功能不存在。
 * 因此这里同时覆盖"拖拽"与"方向键"两条路径。
 */
describe('Resizable', () => {
  function renderResizable(onSizeChange: (size: number) => void, size = 240) {
    return render(
      <Resizable handleLabel="调整详情面板宽度" size={size} onSizeChange={onSizeChange}>
        <div>内容</div>
      </Resizable>,
    );
  }

  it('分隔条暴露 current/min/max 与方向（读屏软件据此播报）', () => {
    renderResizable(() => undefined);

    const handle = screen.getByRole('separator', { name: '调整详情面板宽度' });
    expect(handle).toHaveAttribute('aria-valuenow', '240');
    expect(handle).toHaveAttribute('aria-valuemin', '160');
    expect(handle).toHaveAttribute('aria-valuemax', '720');
    expect(handle).toHaveAttribute('aria-orientation', 'vertical');
    expect(handle).toHaveAttribute('tabindex', '0');
  });

  it('方向键按步长调整，Shift 加速', () => {
    const onSizeChange = vi.fn();
    renderResizable(onSizeChange);
    const handle = screen.getByRole('separator');

    fireEvent.keyDown(handle, { key: 'ArrowRight' });
    expect(onSizeChange).toHaveBeenLastCalledWith(256);

    fireEvent.keyDown(handle, { key: 'ArrowRight', shiftKey: true });
    expect(onSizeChange).toHaveBeenLastCalledWith(304);

    fireEvent.keyDown(handle, { key: 'ArrowLeft' });
    expect(onSizeChange).toHaveBeenLastCalledWith(224);
  });

  it('其它按键不改变尺寸', () => {
    const onSizeChange = vi.fn();
    renderResizable(onSizeChange);

    fireEvent.keyDown(screen.getByRole('separator'), { key: 'a' });
    expect(onSizeChange).not.toHaveBeenCalled();
  });

  it('拖拽按位移调整尺寸，且不会超过上界', () => {
    const onSizeChange = vi.fn();
    renderResizable(onSizeChange);
    const handle = screen.getByRole('separator');

    fireEvent.pointerDown(handle, { pointerId: 1, clientX: 300 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 340 });
    expect(onSizeChange).toHaveBeenLastCalledWith(280);

    // 超过 maxSize(720)：应被夹紧到上界而不是继续增长
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 3000 });
    expect(onSizeChange).toHaveBeenLastCalledWith(720);

    fireEvent.pointerUp(handle, { pointerId: 1 });
    // 抬起之后继续移动不再改变尺寸（避免"松手后还在跟手"）
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 100 });
    expect(onSizeChange).toHaveBeenLastCalledWith(720);
  });

  it('垂直方向用上下键调整', () => {
    const onSizeChange = vi.fn();
    render(
      <Resizable
        handleLabel="调整高度"
        orientation="vertical"
        size={200}
        onSizeChange={onSizeChange}
      >
        <div>内容</div>
      </Resizable>,
    );

    fireEvent.keyDown(screen.getByRole('separator'), { key: 'ArrowDown' });
    expect(onSizeChange).toHaveBeenLastCalledWith(216);
  });
});

describe('SplitPane', () => {
  function renderSplit(onPrimarySizeChange?: (size: number) => void) {
    return render(
      <SplitPane
        separatorLabel="调整左侧宽度"
        defaultPrimarySize={300}
        {...(onPrimarySizeChange === undefined ? {} : { onPrimarySizeChange })}
        primary={<div>主区</div>}
        secondary={<div>副区</div>}
      />,
    );
  }

  it('同时渲染主区与副区，并提供可调分隔条', () => {
    renderSplit();

    expect(screen.getByText('主区')).toBeInTheDocument();
    expect(screen.getByText('副区')).toBeInTheDocument();
    expect(screen.getByRole('separator', { name: '调整左侧宽度' })).toHaveAttribute(
      'aria-valuenow',
      '300',
    );
  });

  it('非受控模式下方向键调整后 aria-valuenow 跟着更新', () => {
    renderSplit();

    fireEvent.keyDown(screen.getByRole('separator'), { key: 'ArrowRight' });
    expect(screen.getByRole('separator')).toHaveAttribute('aria-valuenow', '316');
  });

  it('受控模式下把变更交给调用方，自己不擅自改', () => {
    const onPrimarySizeChange = vi.fn();
    render(
      <SplitPane
        separatorLabel="调整左侧宽度"
        primarySize={300}
        onPrimarySizeChange={onPrimarySizeChange}
        primary={<div>主区</div>}
        secondary={<div>副区</div>}
      />,
    );

    fireEvent.keyDown(screen.getByRole('separator'), { key: 'ArrowRight' });

    expect(onPrimarySizeChange).toHaveBeenCalledWith(316);
    // 受控值未变，界面就应保持 300（尺寸由调用方说的算）
    expect(screen.getByRole('separator')).toHaveAttribute('aria-valuenow', '300');
  });
});
