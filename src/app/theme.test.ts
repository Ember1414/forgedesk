import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  THEME_STORAGE_KEY,
  applyThemeMode,
  isThemeMode,
  readThemeMode,
  resolveTheme,
  watchSystemTheme,
  writeThemeMode,
} from '@/app/theme';

/**
 * 主题模块的守护测试。
 *
 * 为什么值得测：主题有三处容易出错的地方——
 *   1. 'system' 的解析（多一次系统偏好判断，写错就会在暗色系统上显示亮色）；
 *   2. localStorage 的读写（脏数据不能把界面带崩）；
 *   3. 系统外观变化的订阅（忘记取消订阅会泄漏，切换设置后表现错乱）。
 */
interface MediaQueryStub {
  /** 模拟系统外观变化。 */
  emit(matches: boolean): void;
  listenerCount(): number;
}

function stubMatchMedia(initialMatches: boolean): MediaQueryStub {
  const listeners = new Set<(event: MediaQueryListEvent) => void>();
  let matches = initialMatches;

  window.matchMedia = ((query: string) => ({
    get matches() {
      return matches;
    },
    media: query,
    onchange: null,
    addEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => {
      listeners.add(listener);
    },
    removeEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => {
      listeners.delete(listener);
    },
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;

  return {
    emit(nextMatches) {
      matches = nextMatches;
      for (const listener of listeners) {
        listener({ matches: nextMatches } as MediaQueryListEvent);
      }
    },
    listenerCount: () => listeners.size,
  };
}

afterEach(() => {
  window.localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
  vi.restoreAllMocks();
});

describe('resolveTheme', () => {
  it('显式模式直接返回自身', () => {
    expect(resolveTheme('light')).toBe('light');
    expect(resolveTheme('dark')).toBe('dark');
  });

  it("'system' 跟随系统偏好", () => {
    stubMatchMedia(true);
    expect(resolveTheme('system')).toBe('dark');
    stubMatchMedia(false);
    expect(resolveTheme('system')).toBe('light');
  });
});

describe('applyThemeMode', () => {
  it('把解析后的主题写到 <html> 并同步 color-scheme', () => {
    stubMatchMedia(false);
    expect(applyThemeMode('dark')).toBe('dark');
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    expect(document.documentElement.style.colorScheme).toBe('dark');
  });

  it('system 模式写入的是解析后的确定值，而不是 "system" 字面量', () => {
    stubMatchMedia(true);
    applyThemeMode('system');
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
  });
});

describe('主题持久化', () => {
  it('默认是 system', () => {
    expect(readThemeMode()).toBe('system');
  });

  it('写入后能读回', () => {
    writeThemeMode('light');
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('light');
    expect(readThemeMode()).toBe('light');
  });

  it('脏数据回落到 system，而不是返回非法值', () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, 'midnight');
    expect(readThemeMode()).toBe('system');
    expect(isThemeMode('midnight')).toBe(false);
  });
});

describe('watchSystemTheme', () => {
  it('系统外观变化时回调解析后的主题', () => {
    const media = stubMatchMedia(false);
    const onChange = vi.fn();
    watchSystemTheme(onChange);

    media.emit(true);
    expect(onChange).toHaveBeenCalledWith('dark');

    media.emit(false);
    expect(onChange).toHaveBeenLastCalledWith('light');
  });

  it('取消订阅后不再收到回调（避免监听器泄漏与切换设置后的错乱）', () => {
    const media = stubMatchMedia(false);
    const onChange = vi.fn();
    const stop = watchSystemTheme(onChange);
    expect(media.listenerCount()).toBe(1);

    stop();
    expect(media.listenerCount()).toBe(0);

    media.emit(true);
    expect(onChange).not.toHaveBeenCalled();
  });
});
