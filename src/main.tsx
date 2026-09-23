import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from '@/app/App';
import { applyThemeMode, readThemeMode } from '@/app/theme';
// 副作用导入：初始化 i18n（必须在首次渲染前完成，否则会先渲染出 key 原文）
import '@/lib/i18n';

import '@/styles/index.css';

// 在挂载 React 之前把主题写到 <html>：暗色用户不会看到一帧白屏（闪白）
applyThemeMode(readThemeMode());

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
