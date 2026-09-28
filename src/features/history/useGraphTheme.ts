/**
 * 订阅 Canvas 配色（T2.2）。
 *
 * # 为什么不能直接在组件里调 `readGraphThemeFromDocument()`
 *
 * 那样每次渲染都会重新走一遍 `getComputedStyle`（一次强制样式计算，
 * 在大 DOM 上可达数毫秒），而且拿到的是**新对象**——任何以它为依赖的
 * `useMemo` 都会失效，等于每帧重画。
 *
 * # 为什么用 `useSyncExternalStore` 而不是 `useState` + `useEffect`
 *
 * 主题的真相在 DOM 上（`<html data-theme>`），是一个外部存储。
 * 用 `useEffect` 里 `setState` 同步它会有一帧是旧色（Canvas 先按亮色画完，
 * 下一帧才纠正成暗色），并且撞上 `react-hooks/set-state-in-effect`（error 级）。
 * `useSyncExternalStore` 的 `getSnapshot` 在渲染期同步读取，天然没有这一帧。
 *
 * 额外的好处：`theme.ts` 在"跟随系统"模式下由 `matchMedia` 回调改写
 * `data-theme`，React 完全不知道这件事发生了。MutationObserver 是唯一
 * 能同时覆盖"用户改设置"和"系统改外观"两条路径的观察点。
 */
import { useSyncExternalStore } from 'react';

import { readGraphThemeFromDocument } from '@/features/history/graphTheme';
import type { GraphTheme } from '@/features/history/graphTheme';

/** 快照缓存的键与值（模块级：所有订阅者共享同一份，切主题时一起失效）。 */
let cacheKey: string | null = null;
let cacheValue: GraphTheme | null = null;

/** 当前主题标识；没有 DOM 或没有 `data-theme` 时用固定字符串。 */
function currentThemeKey(): string {
  if (typeof document === 'undefined') {
    return 'none';
  }
  return document.documentElement.getAttribute('data-theme') ?? 'none';
}

/**
 * 读取（并按主题缓存）Canvas 配色。
 *
 * 缓存是必需的：`useSyncExternalStore` 要求 `getSnapshot` 在数据未变时
 * **返回同一个引用**，否则 React 会认为存储每帧都在变，陷入无限重渲染
 * （并在控制台打出 "The result of getSnapshot should be cached" 警告）。
 *
 * 用 `data-theme` 做键的取舍：若将来有人在运行时改 token 值而不改主题，
 * 这里会读到旧色。这种用法与 tokens.css 的约定（只有 light/dark 两套）冲突，
 * 因此不值得为它付出"每帧强制样式计算"的代价。
 */
function snapshot(): GraphTheme {
  const key = currentThemeKey();
  if (cacheValue !== null && cacheKey === key) {
    return cacheValue;
  }
  const theme = readGraphThemeFromDocument();
  cacheKey = key;
  cacheValue = theme;
  return theme;
}

/**
 * 订阅 `<html data-theme>` 的变化。
 *
 * 只观察这一个属性（`attributeFilter`）：不限制的话，任何祖先属性变动
 * 都会回调一次，而回调会触发所有订阅组件重渲染。
 */
function subscribe(onStoreChange: () => void): () => void {
  if (typeof document === 'undefined' || typeof MutationObserver === 'undefined') {
    return () => undefined;
  }
  const observer = new MutationObserver(() => {
    onStoreChange();
  });
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme'],
  });
  return () => {
    observer.disconnect();
  };
}

/**
 * 当前主题下的 Canvas 配色。
 *
 * 返回值在主题不变时引用稳定，可以直接当 `useMemo` / 绘制调度的依赖。
 */
export function useGraphTheme(): GraphTheme {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/** 让缓存失效（单测里改完 token 后强制重读用）。 */
export function invalidateGraphThemeCache(): void {
  cacheKey = null;
  cacheValue = null;
}
