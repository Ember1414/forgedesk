import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from '@/app/App';

import '@/styles/index.css';

const container = document.getElementById('root');
if (!container) {
  throw new Error('未找到根容器 #root，index.html 可能被修改。');
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
