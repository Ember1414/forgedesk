import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { parsePanelDsl } from '@/features/plugins/panelDsl';
import { PanelRenderer } from '@/features/plugins/PanelRenderer';

/**
 * 面板渲染器（T6.3 验收）：
 *   - 七种块全部可渲染；
 *   - 坏 DSL（未知块/非数组/坏 JSON）渲染兜底错误卡片而不是崩溃；
 *   - button 点击把命令 id 交给上层回调。
 * DSL 规则的细节由 Rust 侧 panel_dsl.rs 的测试钉住，这里只测渲染契约。
 */

const SAMPLE_DSL = JSON.stringify([
  { type: 'heading', text: '仓库统计' },
  { type: 'text', text: '最近 30 天', tone: 'muted' },
  {
    type: 'keyValue',
    entries: [
      ['分支', 'main'],
      ['领先', '3'],
    ],
  },
  { type: 'table', columns: ['作者', '提交数'], rows: [['a', '12']] },
  { type: 'list', items: ['大文件: a.bin', '疑似密钥: .env'] },
  { type: 'progress', label: '配额', value: 42 },
  { type: 'button', command: 'com.example.x.refresh', label: '刷新' },
]);

describe('插件面板渲染器', () => {
  it('渲染全部七种块', () => {
    render(<PanelRenderer dslJson={SAMPLE_DSL} />);

    expect(screen.getByRole('heading', { name: '仓库统计' })).toBeInTheDocument();
    expect(screen.getByText('最近 30 天')).toBeInTheDocument();
    expect(screen.getByText('分支')).toBeInTheDocument();
    expect(screen.getByText('作者')).toBeInTheDocument();
    expect(screen.getByText('大文件: a.bin')).toBeInTheDocument();
    expect(screen.getByText('42%')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '刷新' })).toBeInTheDocument();
  });

  it('button 点击把命令 id 交给上层回调', () => {
    const onCommand = vi.fn();
    render(<PanelRenderer dslJson={SAMPLE_DSL} onCommand={onCommand} />);

    fireEvent.click(screen.getByRole('button', { name: '刷新' }));

    expect(onCommand).toHaveBeenCalledWith('com.example.x.refresh');
  });

  it('未传 onCommand 时按钮仍渲染（置灰交给后续接线）', () => {
    render(<PanelRenderer dslJson={SAMPLE_DSL} />);
    expect(screen.getByRole('button', { name: '刷新' })).toBeInTheDocument();
  });

  it.each([
    ['未知块类型', JSON.stringify([{ type: 'iframe', src: 'https://evil' }])],
    ['根不是数组', JSON.stringify({ type: 'text' })],
    ['坏 JSON', '{broken'],
    ['进度越界', JSON.stringify([{ type: 'progress', label: 'x', value: 101 }])],
  ])('%s 时渲染兜底错误卡片而不是崩溃', (_name, badDsl) => {
    render(<PanelRenderer dslJson={badDsl} />);

    const fallback = screen.getByRole('alert');
    expect(fallback).toBeInTheDocument();
    expect(fallback).toHaveTextContent('插件面板暂时无法显示');
    // 正常内容不存在
    expect(screen.queryByTestId('plugin-panel-content')).not.toBeInTheDocument();
  });

  it('解析器拒绝未知块并保留类型名（与宿主校验同规则）', () => {
    expect(() => parsePanelDsl('[{"type": "iframe"}]')).toThrow(/iframe/);
  });
});
