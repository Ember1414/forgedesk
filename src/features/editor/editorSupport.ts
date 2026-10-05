/**
 * 编辑器支撑（T5.7）：打开文件的装配与外部变更订阅。
 *
 * 打开 = fs_read → 装配标签（元数据 + 磁盘基线）；外部变更 = 订阅
 * `repo:changed`（workspace 类别，来自文件监听 T1.10），路径命中已打开
 * 文件时重读磁盘并与基线比对——比对结果驱动三选一提示（绝不静默覆盖）。
 */
import { fsRead } from '@/lib/ipc/fs';
import { onRepoChanged } from '@/lib/ipc/workspace';
import type { Unlisten } from '@/lib/ipc/client';
import type { EditorTab } from '@/stores/editorStore';

/** 打开一个文件并装配成编辑器标签。 */
export async function openEditorFile(repoId: number, path: string): Promise<EditorTab> {
  const content = await fsRead(repoId, path);
  return {
    path,
    name: path.split('/').pop() ?? path,
    eol: content.eol,
    hasBom: content.hasBom,
    isBinary: content.isBinary,
    baseline: content.content ?? null,
    dirtyDisk: false,
  };
}

/**
 * 订阅仓库变化并对指定文件做"外部修改"检测。
 *
 * 只在路径命中时回调；组件卸载时退订。返回退订函数的 Promise 形态
 * 与 Tauri 事件订阅一致（调用方在 cleanup 里 await）。
 */
export function watchExternalChanges(
  repoId: number,
  path: string,
  onExternal: () => void,
): Promise<Unlisten> {
  return onRepoChanged((payload) => {
    if (payload.repoId !== repoId) {
      return;
    }
    if (payload.kind === 'workspace' || payload.kind === 'large') {
      if (payload.kind === 'large' || payload.paths.some((changed) => changed === path)) {
        onExternal();
      }
    }
  });
}
