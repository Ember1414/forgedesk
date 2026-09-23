/**
 * Vitest 全局测试环境初始化。
 *
 * 通过 vitest 的 `setupFiles` 在每个测试文件执行前加载：
 *   - 注册 @testing-library/jest-dom 的自定义断言（toBeInTheDocument 等）
 *   - 补齐 jsdom 缺失的浏览器 API（Radix 原语会用到，缺了会直接抛错）
 *   - 初始化 i18n 并固定为简体中文：断言文案时不依赖开发机的系统语言
 */
import '@testing-library/jest-dom/vitest';
import { beforeEach } from 'vitest';

import i18n, { i18nReady } from '@/lib/i18n';

await i18nReady;
// 固定语言，避免 CI（通常为 en-US）与本地（zh-CN）出现不同的断言结果
await i18n.changeLanguage('zh-CN');

/**
 * jsdom 未实现的 API 补齐。
 *
 * 为什么必须补：Radix 的浮层组件依赖 ResizeObserver 测量锚点、依赖
 * Pointer Capture 实现"拖出元素后仍能收到事件"、依赖 scrollIntoView 滚动到选中项。
 * 缺少它们时抛出的异常往往指向内部实现（例如 `setPointerCapture is not a function`），
 * 排查成本很高；一次性补齐比在每个测试里各补一次更可靠。
 */
if (typeof window !== 'undefined') {
  if (!('ResizeObserver' in window)) {
    class ResizeObserverStub {
      observe(): void {
        /* 测试环境不需要真实测量 */
      }
      unobserve(): void {
        /* noop */
      }
      disconnect(): void {
        /* noop */
      }
    }
    Object.defineProperty(window, 'ResizeObserver', {
      writable: true,
      value: ResizeObserverStub,
    });
  }

  Element.prototype.scrollIntoView = () => undefined;
  Element.prototype.scrollTo = () => undefined;
  Element.prototype.hasPointerCapture = () => false;
  Element.prototype.setPointerCapture = () => undefined;
  Element.prototype.releasePointerCapture = () => undefined;

  // jsdom 没有 PointerEvent 构造器；用 MouseEvent 兜底，使 fireEvent.pointerDown 可派发
  if (!('PointerEvent' in window)) {
    class PointerEventStub extends MouseEvent {
      readonly pointerId: number;
      readonly pointerType: string;

      constructor(type: string, init: PointerEventInit = {}) {
        super(type, init);
        this.pointerId = init.pointerId ?? 1;
        this.pointerType = init.pointerType ?? 'mouse';
      }
    }
    Object.defineProperty(window, 'PointerEvent', { writable: true, value: PointerEventStub });
  }
}

beforeEach(() => {
  // 主题与语言都持久化在 localStorage：每个用例前清空，避免用例之间互相影响
  window.localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
});
