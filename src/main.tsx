import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from '@/app/App';
import { applyThemeMode, readThemeMode } from '@/app/theme';
import { installFrontendErrorHooks } from '@/lib/frontendErrors';
// 副作用导入：初始化 i18n（必须在首次渲染前完成，否则会先渲染出 key 原文）
import '@/lib/i18n';

import '@/styles/index.css';

// 在挂载 React 之前把主题写到 <html>：暗色用户不会看到一帧白屏（闪白）
applyThemeMode(readThemeMode());

/**
 * 未捕获错误的钩子（PLAN §10 的 DoD 必过项）。
 *
 * 为什么在渲染前安装：`createRoot` 之后的渲染错误只有装了 error boundary 才会被接住，
 * 而这个钩子必须比第一行业务代码更早存在。被 React/TanStack Query 捕获的错误
 * 不会进这里（那是"已处理"）；进来的只有真正逃逸的错误。
 *
 * 去路按构建类型分开：开发与 E2E 收进 `window.__errs`（测试结束断言为空），
 * 生产写本地日志——生产里没人看那个数组，只有日志能把"偶发白屏"变成证据。
 */
installFrontendErrorHooks({ collect: import.meta.env.DEV });

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
