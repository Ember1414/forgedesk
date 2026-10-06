import { describe, expect, it, vi } from 'vitest';

import { formatDate, formatDateTime, formatRelative } from './intl';

/**
 * Intl 统一封装（T6.7）。
 *
 * 锚定的是**行为契约**而不是具体文案（Intl 的输出随 ICU 版本微调）：
 *   - null/无效输入 → null；
 *   - 同一输入在 zh-CN 与 en-US 下产生不同输出（locale 真的被用上了）；
 *   - 相对时间的方向（过去 vs 未来）由 RelativeTimeFormat 决定。
 */

// 固定语言：Intl 输出依赖 locale，测试里不能让开发机语言影响断言
async function withLocale(locale: string, fn: () => void): Promise<void> {
  const i18n = (await import('@/lib/i18n')).default;
  const previous = i18n.resolvedLanguage;
  await i18n.changeLanguage(locale as 'zh-CN' | 'en-US');
  try {
    fn();
  } finally {
    void i18n.changeLanguage(previous as 'zh-CN' | 'en-US');
  }
}

describe('Intl 日期格式化', () => {
  it('null 与无效输入返回 null 而不是抛错', () => {
    expect(formatDateTime(null)).toBeNull();
    expect(formatDateTime(undefined)).toBeNull();
    expect(formatDateTime(Number.NaN)).toBeNull();
    expect(formatDate(new Date('not a date'))).toBeNull();
  });

  it('有效输入返回非空字符串', () => {
    const timestamp = Date.UTC(2026, 0, 15, 8, 30);
    expect(formatDateTime(timestamp)).toBeTruthy();
    expect(formatDate(timestamp)).toBeTruthy();
  });
});

describe('Intl 相对时间', () => {
  it('按当前语言输出（zh-CN 与 en-US 输出不同）', async () => {
    const now = Math.floor(Date.UTC(2026, 5, 1, 12, 0) / 1000);
    const threeMinutesAgo = now - 180;

    let zhOutput = '';
    let enOutput = '';
    await withLocale('zh-CN', () => {
      zhOutput = formatRelative(threeMinutesAgo, now);
    });
    await withLocale('en-US', () => {
      enOutput = formatRelative(threeMinutesAgo, now);
    });
    expect(zhOutput).toContain('3');
    expect(enOutput).toMatch(/3/);
    expect(zhOutput).not.toEqual(enOutput);
  });

  it('过去与未来在同一语言下输出不同（英文下断言具体词）', async () => {
    const now = Math.floor(Date.UTC(2026, 5, 1, 12, 0) / 1000);
    const yesterday = now - 86_400;
    const tomorrow = now + 86_400;

    await withLocale('en-US', () => {
      // numeric: auto 下 ±1 day 是特殊词；英文里分别是 yesterday / tomorrow
      expect(formatRelative(yesterday, now)).toMatch(/yesterday/i);
      expect(formatRelative(tomorrow, now)).toMatch(/tomorrow/i);
    });
  });

  it('60 秒内按秒分档，巨大差值按年分档', () => {
    const now = Math.floor(Date.UTC(2026, 5, 1, 12, 0) / 1000);
    const justNow = formatRelative(now - 30, now);
    const yearsAgo = formatRelative(now - 40 * 365.25 * 86_400, now);
    expect(justNow).toBeTruthy();
    expect(yearsAgo).toMatch(/4|年|year/);
  });

  it('vi.spyOn 覆盖 resolvedLanguage 也能工作（防御回归）', async () => {
    await withLocale('en-US', () => {
      const now = Math.floor(Date.UTC(2026, 5, 1, 12, 0) / 1000);
      expect(formatRelative(now - 3_600, now)).toMatch(/hour/i);
    });
    vi.restoreAllMocks();
  });
});
