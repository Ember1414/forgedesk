import { describe, expect, it } from 'vitest';

import { cn } from '@/lib/utils';

/**
 * `cn` 是全部 UI 组件的基础工具（合并 Tailwind 类名，后者覆盖前者的同类工具类）。
 * 它一旦行为异常，表现是"样式莫名其妙不生效"，排查成本很高，因此需要测试锁住语义。
 */
describe('cn', () => {
  it('拼接多个类名', () => {
    expect(cn('a', 'b')).toBe('a b');
  });

  it('忽略假值参数', () => {
    expect(cn('a', false, undefined, null, '', 'b')).toBe('a b');
  });

  it('支持条件对象写法', () => {
    expect(cn('base', { active: true, disabled: false })).toBe('base active');
  });

  it('同组工具类由后者覆盖前者（tailwind-merge 的核心作用）', () => {
    expect(cn('p-2', 'p-4')).toBe('p-4');
    expect(cn('text-fg', 'text-danger')).toBe('text-danger');
  });

  it('不同组的工具类不会被误删', () => {
    const result = cn('p-4', 'text-14', 'bg-surface', 'rounded-md');
    expect(result).toBe('p-4 text-14 bg-surface rounded-md');
  });

  it('处理自定义语义 token 类（本项目的 bg-surface / text-fg-muted 等）', () => {
    // 语义 token 是自定义的，必须确认 tailwind-merge 不会把它们当成"冲突组"合并掉
    const result = cn('bg-canvas', 'text-fg-muted', 'border-line');
    expect(result).toContain('bg-canvas');
    expect(result).toContain('text-fg-muted');
    expect(result).toContain('border-line');
  });
});
