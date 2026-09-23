import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { THEME_STORAGE_KEY } from '@/app/theme';
import { initialUiState, useUiStore } from '@/stores/uiStore';

/**
 * uiStore 测试。
 *
 * 注意：Zustand 的 store 是**模块级单例**，用例之间必须复位，
 * 否则会出现"单独跑通过、一起跑失败"的顺序依赖（典型的假绿来源）。
 */
beforeEach(() => {
  window.localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
  useUiStore.setState(initialUiState);
});

afterEach(() => {
  useUiStore.setState(initialUiState);
});

describe('侧栏', () => {
  it('默认展开', () => {
    expect(useUiStore.getState().sidebarCollapsed).toBe(false);
  });

  it('可以切换与显式设置', () => {
    useUiStore.getState().toggleSidebar();
    expect(useUiStore.getState().sidebarCollapsed).toBe(true);

    useUiStore.getState().toggleSidebar();
    expect(useUiStore.getState().sidebarCollapsed).toBe(false);

    useUiStore.getState().setSidebarCollapsed(true);
    expect(useUiStore.getState().sidebarCollapsed).toBe(true);
  });
});

describe('详情面板位置', () => {
  it('默认在右侧，可切换到底部或隐藏', () => {
    expect(useUiStore.getState().detailPanel).toBe('right');
    useUiStore.getState().setDetailPanel('bottom');
    expect(useUiStore.getState().detailPanel).toBe('bottom');
    useUiStore.getState().setDetailPanel('hidden');
    expect(useUiStore.getState().detailPanel).toBe('hidden');
  });
});

describe('当前仓库', () => {
  it('默认未打开仓库', () => {
    expect(useUiStore.getState().currentRepoId).toBeNull();
  });

  it('可设置与清空', () => {
    useUiStore.getState().setCurrentRepoId('example-forgedesk');
    expect(useUiStore.getState().currentRepoId).toBe('example-forgedesk');
    useUiStore.getState().setCurrentRepoId(null);
    expect(useUiStore.getState().currentRepoId).toBeNull();
  });
});

describe('主题', () => {
  it('设置主题时同时落盘并写入 <html>（不依赖 React 渲染）', () => {
    useUiStore.getState().setThemeMode('dark');
    expect(useUiStore.getState().themeMode).toBe('dark');
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark');
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
  });
});
