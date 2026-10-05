import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { TerminalPage } from '@/features/terminal/TerminalPage';
import { initialTerminalState, useTerminalStore } from '@/stores/terminalStore';

/**
 * 终端页面的组件测试（T5.2）。
 *
 * TerminalView（xterm 实例挂在 jsdom 打不开的 canvas 上）被替换为桩：
 * 这里的断言对象是**页面的编排行为**——创建会话、标签生命周期、重命名；
 * xterm 本体的行为由 E2E 覆盖（e2e/terminal.spec.ts）。
 */
vi.mock('@/lib/ipc', () => ({
  termCreate: vi.fn(),
  termClose: vi.fn(),
  termShellList: vi.fn(),
  termResize: vi.fn(),
  termWrite: vi.fn(),
  systemOpenUrl: vi.fn(),
  isTauriRuntime: vi.fn(() => false),
  listenTermOutput: vi.fn(),
  listenTermExit: vi.fn(),
}));

vi.mock('@/features/terminal/TerminalView', () => ({
  TerminalView: ({ tab, active }: { tab: { readonly termId: string }; active: boolean }) => (
    <div data-testid={`view-${tab.termId}`} data-active={String(active)} />
  ),
}));

import { termClose, termCreate, termShellList } from '@/lib/ipc';

const termCreateMock = vi.mocked(termCreate);
const termCloseMock = vi.mocked(termClose);
const termShellListMock = vi.mocked(termShellList);

function renderPage(): void {
  render(
    <MemoryRouter initialEntries={['/repo/7/terminal']}>
      <Routes>
        <Route path="/repo/:repoId/terminal" element={<TerminalPage />} />
      </Routes>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useTerminalStore.setState(initialTerminalState);
  termShellListMock.mockResolvedValue([
    { id: 'default', program: '' },
    { id: 'pwsh', program: 'pwsh' },
  ]);
  termCreateMock.mockResolvedValue({ termId: 'term-1', program: 'pwsh' });
  termCloseMock.mockResolvedValue(undefined);
});

afterEach(() => {
  useTerminalStore.setState(initialTerminalState);
});

async function createTerminalViaMenu(shellName: string): Promise<void> {
  renderPage();
  const plus = screen.getByRole('button', { name: '新建终端' });
  // Radix 在 pointerdown 打开菜单（DEV-ENV 陷阱 9）
  fireEvent.pointerDown(plus, { pointerId: 1, pointerType: 'mouse', button: 0 });
  fireEvent.click(plus);
  const item = await screen.findByRole('menuitem', { name: shellName });
  fireEvent.click(item);
  await waitFor(() => {
    expect(useTerminalStore.getState().tabs.length).toBe(1);
  });
}

describe('TerminalPage（终端页面编排）', () => {
  it('没有标签时展示空态引导', () => {
    renderPage();
    expect(screen.getByText('还没有终端。点上面的 + 新建一个。')).toBeInTheDocument();
  });

  it('从 shell 菜单创建会话：后端参数带仓库 id，标签立即激活并渲染视图', async () => {
    await createTerminalViaMenu('默认 Shell');

    expect(termCreateMock).toHaveBeenCalledWith({
      repoId: 7,
      cols: 80,
      rows: 24,
    });
    const state = useTerminalStore.getState();
    expect(state.activeTermId).toBe('term-1');
    expect(screen.getByTestId('view-term-1')).toHaveAttribute('data-active', 'true');
  });

  it('关闭标签时同步调用后端 term_close 并移除视图', async () => {
    await createTerminalViaMenu('默认 Shell');

    fireEvent.click(screen.getByRole('button', { name: '关闭终端 默认 Shell' }));

    await waitFor(() => {
      expect(termCloseMock).toHaveBeenCalledWith('term-1');
    });
    expect(useTerminalStore.getState().tabs.length).toBe(0);
    expect(screen.queryByTestId('view-term-1')).not.toBeInTheDocument();
  });

  it('双击标题进入重命名，Enter 提交后 OSC 不再覆盖', async () => {
    await createTerminalViaMenu('默认 Shell');

    fireEvent.doubleClick(screen.getByRole('tab', { name: '默认 Shell' }));
    const input = screen.getByRole('textbox', { name: '重命名终端标签' });
    fireEvent.change(input, { target: { value: '构建' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    const tab = useTerminalStore.getState().tabs[0];
    expect(tab?.title).toBe('构建');
    expect(tab?.renamed).toBe(true);
  });

  it('选择非默认 shell 时把 shell id 传给后端', async () => {
    await createTerminalViaMenu('PowerShell 7');

    expect(termCreateMock).toHaveBeenCalledWith({
      repoId: 7,
      shell: 'pwsh',
      cols: 80,
      rows: 24,
    });
    expect(useTerminalStore.getState().tabs[0]?.title).toBe('PowerShell 7');
  });
});
