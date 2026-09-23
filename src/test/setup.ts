/**
 * Vitest 全局测试环境初始化。
 *
 * 通过 vitest 的 `setupFiles` 在每个测试文件执行前加载：
 *   - 注册 @testing-library/jest-dom 的自定义断言（toBeInTheDocument 等）
 *   - 补充 jsdom 缺失的浏览器 API（如 matchMedia），避免组件测试因环境报错而假失败
 *   - 初始化 i18n 并固定为简体中文：断言文案时不依赖开发机的系统语言
 */
import '@testing-library/jest-dom/vitest';
import { beforeEach } from 'vitest';

import i18n, { i18nReady } from '@/lib/i18n';

await i18nReady;
// 固定语言，避免 CI（通常为 en-US）与本地（zh-CN）出现不同的断言结果
await i18n.changeLanguage('zh-CN');

beforeEach(() => {
  // 主题与语言都持久化在 localStorage：每个用例前清空，避免用例之间互相影响
  window.localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
});

// jsdom 未实现 matchMedia；主题相关的代码会用到它
if (typeof window !== 'undefined' && !window.matchMedia) {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => undefined,
      removeListener: () => undefined,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      dispatchEvent: () => false,
    }),
  });
}
