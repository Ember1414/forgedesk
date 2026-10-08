import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { CommandDictionaryPage } from '@/features/terminal/CommandDictionaryPage';

/**
 * 命令字典页的测试重点（2026-10-08 的用户反馈直接对应三条断言）：
 *
 *   1. **命令名必须可见**——此前 `EXPLAIN_DICTIONARY` 只有 `command.name`（而
 *      命令级条目的 name 恒为 null，它是"子命令名"字段），于是整页只有分类、
 *      风险与说明，没有一条命令名。用户看到的就是"词典里没有命令"。
 *   2. **分类标题只出现一次**——此前是"每个条目各带一个分类标题"，同一个分类名
 *      在页面上重复几十遍，看起来像排版坏了。
 *   3. **风险筛选按命令自身的风险**——此前用 `subs[0].risk` 顶替，选"危险"会
 *      把 `git reflog`（安全，只是第一个子命令危险）也筛出来。
 */
describe('CommandDictionaryPage', () => {
  it('每条命令都显示命令名（不再是"只有解释没有命令"）', () => {
    render(<CommandDictionaryPage />);

    expect(screen.getByText('git init')).toBeInTheDocument();
    // 2026-10-08 补齐的条目：词典从 36 条命令扩到 69 条，至少抽查一条新条目
    expect(screen.getByText('git grep')).toBeInTheDocument();
    expect(screen.getByText('git sparse-checkout')).toBeInTheDocument();
  });

  it('分类标题只渲染一次（不再每个条目重复一遍）', () => {
    render(<CommandDictionaryPage />);

    expect(screen.getAllByRole('heading', { name: '仓库' })).toHaveLength(1);
    expect(screen.getAllByRole('heading', { name: '暂存与工作区' })).toHaveLength(1);
  });

  it('风险筛选只看命令自身的风险', () => {
    render(<CommandDictionaryPage />);

    fireEvent.click(screen.getByRole('radio', { name: '危险' }));

    // 真正的危险命令仍在
    expect(screen.getByText('git clean')).toBeInTheDocument();
    // 安全命令被筛掉
    expect(screen.queryByText('git init')).not.toBeInTheDocument();
    // 回归点：reflog 自身是 safe，只有第一个子命令（expire）危险——
    // 旧实现会把它当成"危险命令"留在列表里
    expect(screen.queryByText('git reflog')).not.toBeInTheDocument();
  });

  it('关键字搜索命中命令名', () => {
    render(<CommandDictionaryPage />);

    fireEvent.change(screen.getByLabelText('搜索命令或解释…'), { target: { value: 'grep' } });

    expect(screen.getByText('git grep')).toBeInTheDocument();
    expect(screen.queryByText('git init')).not.toBeInTheDocument();
  });
});
