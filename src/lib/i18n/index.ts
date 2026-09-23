/**
 * i18n 骨架（M0 / T0.4 首次落地，T0.6 扩展错误文案）。
 *
 * 为什么 M0 就要有它：PLAN.md 的风险表把「i18n 后期补做成本高」列为高概率风险，
 * 对策是「M0 即建立骨架，所有文案从第一天起走 key」。因此外壳的每一句文案
 * 都写在 locales/*.json 里，组件只引用 key —— 后期新增语言不需要改组件。
 *
 * 约定：
 *  1. 命名空间（namespace）按功能域划分，M0 只有两个：
 *       - common：与业务无关的通用词（应用名、主题、语言、空态）
 *       - shell ：应用外壳（顶栏、侧栏、状态栏、骨架页面标题与说明）
 *     后续按 PLAN §M6.2 增加 errors / repo / history / github 等。
 *  2. zh-CN 与 en-US 的 key 必须完全一致，由 i18n.test.ts 自动断言。
 *  3. 语言选择持久化到 localStorage（key: forgedesk.language）；未选择时跟随系统，
 *     非中文系统回落到 en-US。
 */
import i18n from 'i18next';
import type { Resource } from 'i18next';
import { initReactI18next } from 'react-i18next';

import enUsCommon from './locales/en-US/common.json';
import enUsShell from './locales/en-US/shell.json';
import zhCnCommon from './locales/zh-CN/common.json';
import zhCnShell from './locales/zh-CN/shell.json';

/** 首发语言（PLAN PF-03：中英双语）。 */
export const SUPPORTED_LANGUAGES = ['zh-CN', 'en-US'] as const;
export type AppLanguage = (typeof SUPPORTED_LANGUAGES)[number];

export const FALLBACK_LANGUAGE: AppLanguage = 'en-US';
export const DEFAULT_NAMESPACE = 'common';
export const NAMESPACES = ['common', 'shell'] as const;

/** 语言持久化用的存储键。 */
export const LANGUAGE_STORAGE_KEY = 'forgedesk.language';

export const resources: Resource = {
  'zh-CN': { common: zhCnCommon, shell: zhCnShell },
  'en-US': { common: enUsCommon, shell: enUsShell },
};

function isSupportedLanguage(value: string): value is AppLanguage {
  return (SUPPORTED_LANGUAGES as readonly string[]).includes(value);
}

/** 读取用户显式选择的语言（未选择或不合法时返回 undefined）。 */
export function readStoredLanguage(): AppLanguage | undefined {
  try {
    const stored = window.localStorage.getItem(LANGUAGE_STORAGE_KEY);
    return stored !== null && isSupportedLanguage(stored) ? stored : undefined;
  } catch {
    // localStorage 在隐私模式或被策略禁用时会抛错：此时退化为「跟随系统」
    return undefined;
  }
}

/** 初始语言：用户选择 > 系统语言（zh* → 简体中文，其余 → en-US）。 */
export function resolveInitialLanguage(): AppLanguage {
  const stored = readStoredLanguage();
  if (stored !== undefined) {
    return stored;
  }
  const system = typeof navigator === 'undefined' ? '' : navigator.language;
  return system.toLowerCase().startsWith('zh') ? 'zh-CN' : FALLBACK_LANGUAGE;
}

export function writeStoredLanguage(language: AppLanguage): void {
  try {
    window.localStorage.setItem(LANGUAGE_STORAGE_KEY, language);
  } catch {
    /* 存储不可用时忽略：仅影响下次启动的初始语言 */
  }
}

/**
 * 初始化 promise。
 *
 * 资源是内联打包进产物的，理论上同步可用；测试与入口仍显式 await 一次，
 * 避免依赖 i18next 内部的时序细节（否则会出现"偶发渲染出 key 原文"的假失败）。
 */
export const i18nReady = i18n.use(initReactI18next).init({
  resources,
  lng: resolveInitialLanguage(),
  fallbackLng: FALLBACK_LANGUAGE,
  ns: [...NAMESPACES],
  defaultNS: DEFAULT_NAMESPACE,
  interpolation: { escapeValue: false },
  returnNull: false,
});

/**
 * 把 i18next 当前的语言标识归一化为受支持的 AppLanguage。
 *
 * i18next 的 `resolvedLanguage` 可能是 'zh-CN'、'zh'、'en-US'… 甚至回退值，
 * 界面上需要一个稳定值来决定哪个选项处于选中态，因此在这里统一收敛。
 */
export function resolveActiveLanguage(current: string | undefined): AppLanguage {
  if (current === undefined) {
    return FALLBACK_LANGUAGE;
  }
  const normalized = current.toLowerCase();
  const matched = SUPPORTED_LANGUAGES.find((language) =>
    normalized.startsWith(language.slice(0, 2).toLowerCase()),
  );
  return matched ?? FALLBACK_LANGUAGE;
}

/** 切换语言并持久化。 */
export async function changeLanguage(language: AppLanguage): Promise<void> {
  writeStoredLanguage(language);
  await i18n.changeLanguage(language);
}

export default i18n;
