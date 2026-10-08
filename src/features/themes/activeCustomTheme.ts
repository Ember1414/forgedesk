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

import { findBuiltinTheme } from './builtinThemes';
import {
  applyCustomThemeColors,
  readCachedCustomTheme,
  writeCachedCustomTheme,
} from './themeApply';
import type { ThemeDefinition } from './themeModel';

let activeTheme: ThemeDefinition | null = null;

/**
 * 当前激活的"带色主题"（null = 内置的两套基础配色）。
 *
 * 注意语义：内置的 Sandstone Dawn / Pine Nocturne **也**从这里进出——它们与
 * 导入主题的差别只是"随应用分发"，落到 DOM 上完全一样（都是 CSS 变量覆盖）。
 * 只有 ForgeDesk Light/Dark 用 null 表示，因为它们的"覆盖"就是空集。
 */
export function currentActiveCustomTheme(): ThemeDefinition | null {
  return activeTheme;
}

/** 重新套用当前激活主题（外观变化时由订阅回调触发）。 */
function reapply(): void {
  applyCustomThemeColors(activeTheme, currentResolvedTheme());
}

type ActiveThemeListener = () => void;
const activeThemeListeners = new Set<ActiveThemeListener>();

/**
 * 订阅"当前激活主题"的变化（设置页的画廊据此移动"使用中"标记）。
 *
 * 为什么需要它：激活动作只改模块级变量与 DOM 变量，**不会**触发任何 React 更新，
 * 于是页面上的"使用中"徽标纹丝不动——用户点了"使用此主题"却看不到任何反馈，
 * 与"这个按钮没接上"无法区分（2026-10-08 反馈的"四种主题只能用两种"里，
 * 有一半正是这个：色板生效了，但界面拒绝承认）。
 *
 * 返回退订函数；与 `app/theme.ts` 的 resolved 订阅各管一件事，刻意不合并——
 * 那个管"亮/暗解析结果"，这个管"用户选了哪个主题"。
 */
export function subscribeActiveTheme(listener: ActiveThemeListener): () => void {
  activeThemeListeners.add(listener);
  return () => {
    activeThemeListeners.delete(listener);
  };
}

function notifyActiveTheme(): void {
  for (const listener of activeThemeListeners) {
    listener();
  }
}

/** 激活（或取消激活）自定义主题：同步缓存 + 立即上色 + 通知订阅者。 */
export function setActiveCustomTheme(theme: ThemeDefinition | null): void {
  activeTheme = theme;
  writeCachedCustomTheme(theme);
  reapply();
  notifyActiveTheme();
}

/**
 * 启动路径：同步读取首帧缓存并套用（不等 SQLite）。
 * 返回缓存中的主题（调用方可在设置加载后用它做 reconcile）。
 */
export function applyCachedCustomTheme(): ThemeDefinition | null {
  activeTheme = readCachedCustomTheme();
  reapply();
  notifyActiveTheme();
  return activeTheme;
}

/**
 * 设置存储加载后的对账：缓存指向的主题若既不在自定义列表、也不是内置主题
 * （被删除 / 被清空 / id 拼错），自动取消激活。
 *
 * 内置主题必须一并认账：Sandstone Dawn / Pine Nocturne 走的就是这条上色路径，
 * 若把它们当作"不在列表里"清掉，用户选了内置配色重启后会静默回落到
 * ForgeDesk Light（2026-10-08：四种内置主题里两种"点了没反应、重启就丢"）。
 */
export function reconcileActiveCustomTheme(customs: readonly ThemeDefinition[]): void {
  if (activeTheme === null) {
    return;
  }
  const known =
    findBuiltinTheme(activeTheme.id) !== undefined ||
    customs.some((theme) => theme.id === activeTheme?.id);
  if (!known) {
    setActiveCustomTheme(null);
  }
}

// 模块级注册：任何 light/dark 解析结果变化（含 system 自动切换）都重新套用。
// 重复 import 只会注册一次（ESM 模块单例）。
onResolvedThemeChange(() => {
  reapply();
});
