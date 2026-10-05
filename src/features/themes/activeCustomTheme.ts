/**
 * 激活的自定义主题协调（T6.6）。
 *
 * # 为什么需要这个模块
 *
 * 主题有三条写入路径：启动首帧（localStorage 缓存，同步）、设置页操作（SQLite +
 * 缓存）、"跟随系统"下的外观自动切换（applyThemeMode 回调）。三条路径都要落在
 * 同一处 DOM 状态上，否则会出现"改了激活主题但系统切暗色后变量没跟着换"的漂移。
 * 这里集中持有激活态，并把自己注册为 resolved 外观变化的订阅者。
 */

import { currentResolvedTheme, onResolvedThemeChange } from '@/app/theme';

import {
  applyCustomThemeColors,
  readCachedCustomTheme,
  writeCachedCustomTheme,
} from './themeApply';
import type { ThemeDefinition } from './themeModel';

let activeTheme: ThemeDefinition | null = null;

/** 当前激活的自定义主题（null = 使用内置主题）。 */
export function currentActiveCustomTheme(): ThemeDefinition | null {
  return activeTheme;
}

/** 重新套用当前激活主题（外观变化时由订阅回调触发）。 */
function reapply(): void {
  applyCustomThemeColors(activeTheme, currentResolvedTheme());
}

/** 激活（或取消激活）自定义主题：同步缓存 + 立即上色。 */
export function setActiveCustomTheme(theme: ThemeDefinition | null): void {
  activeTheme = theme;
  writeCachedCustomTheme(theme);
  reapply();
}

/**
 * 启动路径：同步读取首帧缓存并套用（不等 SQLite）。
 * 返回缓存中的主题（调用方可在设置加载后用它做 reconcile）。
 */
export function applyCachedCustomTheme(): ThemeDefinition | null {
  activeTheme = readCachedCustomTheme();
  reapply();
  return activeTheme;
}

/**
 * 设置存储加载后的对账：缓存指向的主题若已不在自定义列表中（被删除/清空），
 * 自动取消激活。内置主题不经过这里（它们没有自定义变量可套用）。
 */
export function reconcileActiveCustomTheme(customs: readonly ThemeDefinition[]): void {
  if (activeTheme !== null && !customs.some((theme) => theme.id === activeTheme?.id)) {
    setActiveCustomTheme(null);
  }
}

// 模块级注册：任何 light/dark 解析结果变化（含 system 自动切换）都重新套用。
// 重复 import 只会注册一次（ESM 模块单例）。
onResolvedThemeChange(() => {
  reapply();
});
