/**
 * 目录 / 文件选择对话框（GIT-01/02/03 的路径来源，以及 T7.6 的导出另存为）。
 *
 * # 为什么封装在 lib 层
 *
 * `@tauri-apps/plugin-dialog` 是 JS 插件（底层走 `plugin:dialog|open|save` 命令），
 * 与 IPC 客户端同属"宿主能力"：直接散在组件里会让 e2e 与单测难以拦截，
 * 也违背"调用点可静态检索"的既有纪律（见 `src/lib/ipc/client.ts` 的说明）。
 *
 * # 非 Tauri 环境的行为
 *
 * `pnpm dev` 直接在浏览器里打开（无宿主）或 e2e mock 没有实现 dialog 命令时，
 * `pickFolder` / `pickSavePath` 返回 `null`——调用方把"浏览"按钮禁用、以手输路径兜底，
 * 或把"取消"与"环境不支持"当作同一件事（不写文件），而不是让一次点击变成未捕获异常。
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

/**
 * 打开"保存文件"对话框；用户取消或环境不支持时返回 `null`。
 *
 * `defaultFileName` 只是**建议名**（含扩展名）：系统把它拼成默认路径，用户仍可改目录与文件名。
 * 因此调用方必须按返回的路径去写，**不要**自己再拼一份——否则用户改了名字，
 * 文件却仍然落在旧名字上（而界面上显示的又是新名字）。
 */
export async function pickSavePath(title: string, defaultFileName: string): Promise<string | null> {
  if (!isTauriRuntime()) {
    return null;
  }
  const { save } = await import('@tauri-apps/plugin-dialog');
  const selection = await save({ title, defaultPath: defaultFileName });
  return typeof selection === 'string' ? selection : null;
}
