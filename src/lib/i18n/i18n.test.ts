import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import i18n, {
  FALLBACK_LANGUAGE,
  NAMESPACES,
  SUPPORTED_LANGUAGES,
  resolveActiveLanguage,
} from '@/lib/i18n';
import enUsCommon from '@/lib/i18n/locales/en-US/common.json';
import enUsErrors from '@/lib/i18n/locales/en-US/errors.json';
import enUsShell from '@/lib/i18n/locales/en-US/shell.json';
import zhCnCommon from '@/lib/i18n/locales/zh-CN/common.json';
import zhCnErrors from '@/lib/i18n/locales/zh-CN/errors.json';
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
  'zh-CN': { common: zhCnCommon, shell: zhCnShell, errors: zhCnErrors },
  'en-US': { common: enUsCommon, shell: enUsShell, errors: enUsErrors },
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

/**
 * 源码里引用的 key 必须真的存在。
 *
 * 为什么需要：中英 key 对齐（上面的用例）只能保证"两边一样"，保证不了
 * "界面用的那个 key 存在"。T1.6 引入的 `t('common:actions.cancel')` 就漏了定义，
 * 结果是确认框上印着 `common:actions.cancel` 原文——中英对齐检查、硬编码检查、
 * 类型检查全都拦不住它（`t` 的签名接受任意字符串），一直到 E2E 里按按钮名
 * 找不到元素才暴露。
 *
 * 判定口径（务实优先）：
 *   - 只认**字符串字面量**调用：`t('a.b')`、`t("ns:a.b")`。模板字符串与
 *     拼接（`t(\`errors:${code}.title\`)`）无法静态求值，跳过；
 *   - 带命名空间前缀时按前缀查；不带前缀时**在任一命名空间里存在即可**
 *     （文件里的 `useTranslation('errors')` 之类绑定关系用正则推断太脆，
 *     宁可用宽口径：少一个假阳性，就少一次"把门禁关掉"的冲动）；
 *   - 测试文件自身不参与扫描。
 */
function collectSourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const full = join(directory, entry.name);
    if (entry.isDirectory()) {
      return entry.name === '__tests__' || entry.name === '__mocks__'
        ? []
        : collectSourceFiles(full);
    }
    if (!/\.tsx?$/.test(entry.name) || /\.(test|spec)\.tsx?$/.test(entry.name)) {
      return [];
    }
    return [full];
  });
}

/** 从源码里抽出所有 `t('<字面量>'` 形式的 key 引用。 */
function collectReferencedKeys(source: string): string[] {
  const pattern = /\bt\(\s*(?:'([^'\\\n]+)'|"([^"\\\n]+)")\s*[,)]/g;
  const keys: string[] = [];
  for (const match of source.matchAll(pattern)) {
    const key = match[1] ?? match[2];
    if (key !== undefined) {
      keys.push(key);
    }
  }
  return keys;
}

describe('i18n key 引用完整性', () => {
  const allKeys = new Set(
    NAMESPACES.flatMap((namespace) =>
      SUPPORTED_LANGUAGES.flatMap((language) =>
        flattenKeys(BUNDLES[language][namespace]).map((key) => `${namespace}:${key}`),
      ),
    ),
  );

  it('源码里 t() 引用的 key 全部存在', () => {
    const srcDir = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
    const missing: string[] = [];

    for (const file of collectSourceFiles(srcDir)) {
      const source = readFileSync(file, 'utf8');
      const relativePath = relative(srcDir, file).replace(/\\/g, '/');
      for (const key of collectReferencedKeys(source)) {
        const separator = key.indexOf(':');
        const qualified =
          separator > 0 && (NAMESPACES as readonly string[]).includes(key.slice(0, separator))
            ? key
            : null;
        if (qualified !== null) {
          if (!allKeys.has(qualified)) {
            missing.push(`${relativePath}: ${qualified}`);
          }
          continue;
        }
        // 无前缀：任一命名空间里有就算存在
        const found = NAMESPACES.some((namespace) => allKeys.has(`${namespace}:${key}`));
        if (!found) {
          missing.push(`${relativePath}: ${key}`);
        }
      }
    }

    expect(missing).toEqual([]);
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
