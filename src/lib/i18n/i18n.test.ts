import { describe, expect, it } from 'vitest';

import i18n, {
  FALLBACK_LANGUAGE,
  NAMESPACES,
  SUPPORTED_LANGUAGES,
  resolveActiveLanguage,
} from '@/lib/i18n';
import enUsCommon from '@/lib/i18n/locales/en-US/common.json';
import enUsShell from '@/lib/i18n/locales/en-US/shell.json';
import zhCnCommon from '@/lib/i18n/locales/zh-CN/common.json';
import zhCnShell from '@/lib/i18n/locales/zh-CN/shell.json';

/**
 * i18n 骨架的守护测试。
 *
 * 背景：PLAN.md 把「i18n 后期补做成本高」列为高风险项，对策是 M0 起所有文案走 key。
 * 但"走 key"如果没有守护就会退化成：中文加了 key、英文忘了加，
 * 结果英文界面直接显示 key 原文（比硬编码中文更糟，因为用户看不懂）。
 * 因此这里断言两件事：**中英 key 集合完全一致**、**没有空文案**。
 */
type Namespace = (typeof NAMESPACES)[number];
type Bundle = Record<string, unknown>;

/**
 * 直接引入 JSON 资源（而不是读 i18n 实例）：这样校验的是**仓库里的文件**，
 * 而不是运行时可能已被回退逻辑掩盖的结果。
 */
const BUNDLES = {
  'zh-CN': { common: zhCnCommon, shell: zhCnShell },
  'en-US': { common: enUsCommon, shell: enUsShell },
} satisfies Record<(typeof SUPPORTED_LANGUAGES)[number], Record<Namespace, Bundle>>;

/** 把嵌套的文案对象拍平成 `a.b.c` 形式的 key 列表。 */
function flattenKeys(value: unknown, prefix = ''): string[] {
  if (typeof value !== 'object' || value === null) {
    return [prefix];
  }
  return Object.entries(value).flatMap(([key, child]) =>
    flattenKeys(child, prefix === '' ? key : `${prefix}.${key}`),
  );
}

/** 收集值为空字符串的 key（漏翻译最常见的形态）。 */
function collectEmptyValues(value: unknown, prefix = ''): string[] {
  if (typeof value === 'string') {
    return value.trim() === '' ? [prefix] : [];
  }
  if (typeof value !== 'object' || value === null) {
    return [`${prefix}（不是字符串也不是对象）`];
  }
  return Object.entries(value).flatMap(([key, child]) =>
    collectEmptyValues(child, prefix === '' ? key : `${prefix}.${key}`),
  );
}

describe('i18n 资源完整性', () => {
  it('首发语言包含中英两种', () => {
    expect([...SUPPORTED_LANGUAGES]).toEqual(['zh-CN', 'en-US']);
  });

  it.each([...NAMESPACES])('命名空间 %s 的中英 key 完全一致', (namespace) => {
    const zhKeys = flattenKeys(BUNDLES['zh-CN'][namespace]).sort();
    const enKeys = flattenKeys(BUNDLES['en-US'][namespace]).sort();
    expect(enKeys).toEqual(zhKeys);
  });

  it.each([...NAMESPACES])('命名空间 %s 没有空文案', (namespace) => {
    for (const language of SUPPORTED_LANGUAGES) {
      expect(collectEmptyValues(BUNDLES[language][namespace])).toEqual([]);
    }
  });
});

describe('i18n 运行时', () => {
  it('已初始化为测试设定的语言', () => {
    expect(i18n.isInitialized).toBe(true);
    expect(i18n.language).toBe('zh-CN');
  });

  it('默认命名空间可用', () => {
    expect(i18n.t('app.name')).toBe('ForgeDesk');
  });

  it('shell 命名空间的 key 可用', () => {
    expect(i18n.t('items.dashboard', { ns: 'shell' })).toBe('仪表盘');
  });

  it('归一化 i18next 的语言标识', () => {
    expect(resolveActiveLanguage('zh-CN')).toBe('zh-CN');
    expect(resolveActiveLanguage('zh')).toBe('zh-CN');
    expect(resolveActiveLanguage('zh-Hans-CN')).toBe('zh-CN');
    expect(resolveActiveLanguage('en-GB')).toBe('en-US');
    // 不支持的语言必须回落到 fallback，而不是把 'fr' 当成有效语言
    expect(resolveActiveLanguage('fr')).toBe(FALLBACK_LANGUAGE);
    expect(resolveActiveLanguage(undefined)).toBe(FALLBACK_LANGUAGE);
  });
});
