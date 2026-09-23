import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from '@/app/App';
import { applyThemeMode, readThemeMode } from '@/app/theme';
// 副作用导入：初始化 i18n（必须在首次渲染前完成，否则会先渲染出 key 原文）
import '@/lib/i18n';

import '@/styles/index.css';

// 在挂载 React 之前把主题写到 <html>：暗色用户不会看到一帧白屏（闪白）
applyThemeMode(readThemeMode());

/**
 * 未捕获错误集合（`window.__errs`，PLAN §10 的 DoD 必过项）。
 *
 * 用途：E2E 与人工验收的**最后一道闸**——所有交互结束后断言它为空。
 * 被 React/TanStack Query 捕获的错误不会进这里（那是"已处理"）；
 * 进来的只有真正逃逸的错误：未捕获异常与没人接的 Promise 拒绝。
 *
 * 为什么在渲染前安装：`createRoot` 之后的渲染错误只有装了 error boundary 才会被接住，
 * 而 M0 还没有 boundary——这个钩子必须比第一行业务代码更早存在。
 * 只保留最近 50 条：错误的数量不该成为内存问题。
 */
declare global {
  interface Window {
    /** 未捕获错误集合（E2E 断言其为空；仅保留最近 50 条）。 */
    __errs?: unknown[];
  }
}

window.__errs = [];
const collectError = (error: unknown): void => {
  const errors = window.__errs ?? (window.__errs = []);
  errors.push(error);
  if (errors.length > 50) {
    errors.splice(0, errors.length - 50);
  }
};
window.addEventListener('error', (event) => {
  collectError(event.error ?? event.message);
});
window.addEventListener('unhandledrejection', (event) => {
  collectError(event.reason);
});

const container = document.getElementById('root');
if (!container) {
  // 启动引导期的致命错误：此时 React 与 i18n 都还没初始化，只能抛给开发者看。
  throw new Error('未找到根容器 #root，index.html 可能被修改。'); // i18n-ignore 引导期错误，仅开发者可见
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
