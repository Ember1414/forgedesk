/**
 * 顶层「插件」页（路由 `/plugins`，侧栏「集成 → 插件」与命令面板「打开插件」的目标）。
 *
 * 历史上这里是 M0 的 `PlaceholderPage` 骨架；M6 把真正的插件管理器做成了
 * `PluginSettingsPage`（T6.4，位于设置区 `/settings/plugins`）。结果侧栏的
 * 「插件」条目一直指向「页面还没有实现」的占位页——一个**可见但打不开**的
 * 入口，属于界面显示未随里程碑更新的回归。
 *
 * 现在按仓库既有约定（见 `src/features/repo/RepoStatusPage.tsx` 等转发页），
 * 直接把真实管理页转发出来：插件只有一个实现，避免"骨架页与实现页"两份漂移。
 */
export { PluginSettingsPage as PluginsPage } from '@/features/settings/PluginSettingsPage';
