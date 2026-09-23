/**
 * 是否处于"窄窗口"（< 1100px）。
 *
 * 用途：T0.4 的验收要求「窗口宽度 < 1100px 时侧栏自动折叠为图标模式」。
 * 自动折叠与用户手工折叠是两件事，因此这里只回答"窗口窄不窄"，
 * 最终是否折叠由 AppShell 把两者取或（见 AppShell 中的说明）。
 *
 * 实现细节：
 *   - 初始值用惰性初始化计算，保证首帧就是正确布局（避免渲染后跳动）。
 *   - effect 内**不同步** setState（React 官方不推荐，且被 react-hooks/set-state-in-effect 拦截）；
 *     只在 change 事件中更新。极端情况下（渲染与 effect 之间窗口尺寸恰好变化）
 *     会沿用首帧值，随下一次尺寸变化自动纠正。
 *   - jsdom 未实现 matchMedia 时降级为 false（宽屏布局），测试里由 setup.ts 补齐桩实现。
 */
import { useEffect, useState } from 'react';

export const NARROW_LAYOUT_QUERY = '(max-width: 1099px)';

function matchesNarrowLayout(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return false;
  }
  return window.matchMedia(NARROW_LAYOUT_QUERY).matches;
}

export function useIsNarrowViewport(): boolean {
  const [narrow, setNarrow] = useState<boolean>(matchesNarrowLayout);

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
      return undefined;
    }
    const media = window.matchMedia(NARROW_LAYOUT_QUERY);
    const handler = (event: MediaQueryListEvent): void => {
      setNarrow(event.matches);
    };
    media.addEventListener('change', handler);
    return () => {
      media.removeEventListener('change', handler);
    };
  }, []);

  return narrow;
}
