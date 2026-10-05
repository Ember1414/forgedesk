import { TerminalPage } from '@/features/terminal/TerminalPage';

/**
 * 终端页面（仓库级路由的挂载点）。
 *
 * 实现在 `src/features/terminal/`（T5.1 PTY 会话 + T5.2 xterm 前端）；
 * 本文件只是路由兼容的转发层——路由表（`src/app/routes.tsx`）是共享文件，
 * 保持原路径不动，让 M5 的实现落在自己的功能目录里。
 */
export function RepoTerminalPage() {
  return <TerminalPage />;
}
