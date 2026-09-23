import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Button } from '@/ui/components/button';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/ui/components/context-menu';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/ui/components/dropdown-menu';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/ui/components/tabs';
import { ToggleGroup } from '@/ui/components/toggle-group';

/**
 * 菜单与切换类组件的测试重点：**键盘能否完整操作**。
 * 桌面 Git 工具的高频操作都在键盘上，菜单如果只能鼠标点，
 * 体验会比命令行更差——这正是这类组件最值得测的地方。
 */
function openMenu(trigger: HTMLElement): void {
  // Radix 菜单在 pointerdown 时打开（真实浏览器行为），jsdom 需要显式派发
  fireEvent.pointerDown(trigger, { pointerId: 1, pointerType: 'mouse', button: 0 });
  fireEvent.click(trigger);
}

/**
 * 关于菜单的无障碍名称：Radix 会把 `aria-labelledby` 指向**触发器**，
 * 因此菜单的可访问名称就是触发器的名称（"分支操作"），而调用方传的 aria-label 会被覆盖。
 * 这是符合 ARIA 菜单模式的做法（菜单以它的按钮命名），测试与使用都不应依赖 aria-label。
 */
describe('DropdownMenu', () => {
  function renderMenu(onSelect: () => void) {
    return render(
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button>分支操作</Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuLabel>分支</DropdownMenuLabel>
          <DropdownMenuItem onSelect={onSelect}>检出</DropdownMenuItem>
          <DropdownMenuItem disabled>删除</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>,
    );
  }

  it('点击触发器打开菜单', async () => {
    renderMenu(() => undefined);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();

    openMenu(screen.getByRole('button', { name: '分支操作' }));

    expect(await screen.findByRole('menu')).toBeInTheDocument();
  });

  it('键盘 Enter 打开菜单并可选中项', async () => {
    const onSelect = vi.fn();
    renderMenu(onSelect);

    const trigger = screen.getByRole('button', { name: '分支操作' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });

    const item = await screen.findByRole('menuitem', { name: '检出' });
    fireEvent.click(item);

    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it('禁用项不可选中', async () => {
    renderMenu(() => undefined);
    openMenu(screen.getByRole('button', { name: '分支操作' }));

    const disabledItem = await screen.findByRole('menuitem', { name: '删除' });
    expect(disabledItem).toHaveAttribute('aria-disabled', 'true');
  });

  it('Esc 关闭菜单', async () => {
    renderMenu(() => undefined);
    openMenu(screen.getByRole('button', { name: '分支操作' }));

    const menu = await screen.findByRole('menu');
    fireEvent.keyDown(menu, { key: 'Escape' });

    await waitFor(() => {
      expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    });
  });
});

describe('ContextMenu', () => {
  it('右键打开菜单并执行操作', async () => {
    const onSelect = vi.fn();
    render(
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div>文件行</div>
        </ContextMenuTrigger>
        <ContextMenuContent aria-label="文件菜单">
          <ContextMenuItem onSelect={onSelect}>复制路径</ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>,
    );

    fireEvent.contextMenu(screen.getByText('文件行'));

    const item = await screen.findByRole('menuitem', { name: '复制路径' });
    fireEvent.click(item);
    expect(onSelect).toHaveBeenCalledTimes(1);
  });
});

describe('Tabs', () => {
  function renderTabs() {
    return render(
      <Tabs defaultValue="changes">
        <TabsList aria-label="工作区视图">
          <TabsTrigger value="changes">变更</TabsTrigger>
          <TabsTrigger value="staged">已暂存</TabsTrigger>
        </TabsList>
        <TabsContent value="changes">变更内容</TabsContent>
        <TabsContent value="staged">已暂存内容</TabsContent>
      </Tabs>,
    );
  }

  it('默认显示第一个面板，其余内容不在 DOM 中', () => {
    renderTabs();
    expect(screen.getByRole('tabpanel')).toHaveTextContent('变更内容');
    expect(screen.queryByText('已暂存内容')).not.toBeInTheDocument();
  });

  it('点击标签切换面板并同步 aria-selected', async () => {
    renderTabs();
    // Radix Tabs 的激活发生在 mousedown（与浏览器原生标签行为一致），而不是 click
    fireEvent.mouseDown(screen.getByRole('tab', { name: '已暂存' }));
    fireEvent.click(screen.getByRole('tab', { name: '已暂存' }));

    await waitFor(() => {
      expect(screen.getByRole('tab', { name: '已暂存' })).toHaveAttribute('aria-selected', 'true');
    });
    expect(screen.getByRole('tabpanel')).toHaveTextContent('已暂存内容');
  });

  it('方向键在标签间移动（roving tabindex）', async () => {
    renderTabs();
    const first = screen.getByRole('tab', { name: '变更' });
    first.focus();
    fireEvent.keyDown(first, { key: 'ArrowRight' });

    await waitFor(() => {
      expect(screen.getByRole('tab', { name: '已暂存' })).toHaveFocus();
    });
  });
});

describe('ToggleGroup', () => {
  function renderGroup(onValueChange: (value: string) => void, value = 'light') {
    return render(
      <ToggleGroup
        label="主题"
        value={value}
        onValueChange={onValueChange}
        options={[
          { value: 'light', label: '亮色' },
          { value: 'dark', label: '暗色' },
          { value: 'legacy', label: '旧版', disabled: true },
        ]}
      />,
    );
  }

  it('点击切换选中项', () => {
    const onValueChange = vi.fn();
    renderGroup(onValueChange);

    fireEvent.click(screen.getByText('暗色'));
    expect(onValueChange).toHaveBeenCalledWith('dark');
  });

  it('再次点击当前项不会传出空值（避免出现"全不选"）', () => {
    const onValueChange = vi.fn();
    renderGroup(onValueChange, 'dark');

    fireEvent.click(screen.getByText('暗色'));
    expect(onValueChange).not.toHaveBeenCalledWith('');
  });

  it('禁用项不可点击', () => {
    const onValueChange = vi.fn();
    renderGroup(onValueChange);

    const disabled = screen.getByText('旧版');
    expect(disabled).toBeDisabled();

    fireEvent.click(disabled);
    expect(onValueChange).not.toHaveBeenCalled();
  });

  it('控件有可读的名称（aria-label）', () => {
    renderGroup(() => undefined);
    expect(screen.getByLabelText('主题')).toBeInTheDocument();
  });
});
