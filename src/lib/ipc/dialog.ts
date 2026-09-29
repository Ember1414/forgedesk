/**
 * 目录选择对话框（GIT-01/02/03：打开 / 克隆 / 初始化仓库的路径来源）。
 *
 * # 为什么封装在 lib 层
 *
 * `@tauri-apps/plugin-dialog` 是 JS 插件（底层走 `plugin:dialog|open` 命令），
 * 与 IPC 客户端同属"宿主能力"：直接散在组件里会让 e2e 与单测难以拦截，
 * 也违背"调用点可静态检索"的既有纪律（见 `src/lib/ipc/client.ts` 的说明）。
 *
 * # 非 Tauri 环境的行为
 *
 * `pnpm dev` 直接在浏览器里打开（无宿主）或 e2e mock 没有实现 dialog 命令时，
 * `pickFolder` 返回 `null`——调用方把"浏览"按钮禁用或以手输路径兜底，
 * 而不是让一次点击变成一个未捕获异常。
 */
import { isTauriRuntime } from '@/lib/ipc/client';

/** 打开目录选择器；用户取消或环境不支持时返回 `null`。 */
export async function pickFolder(title: string): Promise<string | null> {
  if (!isTauriRuntime()) {
    return null;
  }
  // 动态 import：插件 JS 只有在 Tauri 宿主里才有意义，普通浏览器构建里
  // 让它留在异步分片，避免拖住首屏（插件本身很轻，这里更多是语义表达）。
  const { open } = await import('@tauri-apps/plugin-dialog');
  const selection = await open({ directory: true, multiple: false, title });
  // 用户取消时插件返回 null；multiple=false 时返回值是单字符串
  return typeof selection === 'string' ? selection : null;
}
