/**
 * Vitest 全局测试环境初始化。
 *
 * 通过 vitest 的 `setupFiles` 在每个测试文件执行前加载：
 *   - 注册 @testing-library/jest-dom 的自定义断言（toBeInTheDocument 等）
 *   - 补充 jsdom 缺失的浏览器 API（如 matchMedia），避免组件测试因环境报错而假失败
 */
import '@testing-library/jest-dom/vitest';

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
