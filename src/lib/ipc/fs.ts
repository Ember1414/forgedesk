/**
 * 工作区文件系统（T5.7）：fs_* 命令族的前端封装。
 *
 * 后端契约见 docs/API.md（`fs_tree` 等）：路径一律相对仓库根（POSIX 分隔符），
 * 安全（逃逸拒绝、软链检查）在后端 `resolve_within` 完成。
 */
import { invokeCommand } from './client';

/** 树节点种类。 */
export type FsKind = 'file' | 'dir';

/** 文件树节点（懒加载一层）。 */
export interface FsNode {
  readonly name: string;
  /** 相对仓库根的路径（POSIX 分隔符）——前端唯一寻址方式。 */
  readonly relPath: string;
  readonly kind: FsKind;
  readonly size: number;
}

/** 换行符形态（保存时按它恢复，绝不静默改写）。 */
export type FsEol = 'lf' | 'crlf' | 'cr' | 'mixed';

/** `fs_read` 的返回。 */
export interface FsFileContent {
  readonly content: string | null;
  readonly eol: FsEol;
  readonly hasBom: boolean;
  readonly size: number;
  readonly isBinary: boolean;
  readonly truncated: boolean;
}

/** 列出目录一层节点。 */
export function fsTree(
  repoId: number,
  path: string,
  options?: { readonly showHidden?: boolean; readonly showIgnored?: boolean },
): Promise<FsNode[]> {
  return invokeCommand<FsNode[]>('fs_tree', {
    repoId,
    path,
    ...(options?.showHidden === undefined ? {} : { showHidden: options.showHidden }),
    ...(options?.showIgnored === undefined ? {} : { showIgnored: options.showIgnored }),
  });
}

/** 读取文件（≤5MB；二进制 content 为 null）。 */
export function fsRead(repoId: number, path: string): Promise<FsFileContent> {
  return invokeCommand<FsFileContent>('fs_read', { repoId, path });
}

/** 写入文件（EOL/BOM 按打开时的元数据恢复）。 */
export function fsWrite(
  repoId: number,
  path: string,
  content: string,
  eol: FsEol,
  hasBom: boolean,
): Promise<{ readonly writtenBytes: number }> {
  return invokeCommand<{ readonly writtenBytes: number }>('fs_write', {
    repoId,
    path,
    content,
    eol,
    hasBom,
  });
}

/** 创建文件 / 目录。 */
export function fsCreate(repoId: number, path: string, isDir: boolean): Promise<void> {
  return invokeCommand<void>('fs_create', { repoId, path, isDir });
}

/** 重命名 / 移动。 */
export function fsRename(repoId: number, path: string, newPath: string): Promise<void> {
  return invokeCommand<void>('fs_rename', { repoId, path, newPath });
}

/** 删除（移入回收站）。 */
export function fsDelete(repoId: number, path: string): Promise<void> {
  return invokeCommand<void>('fs_delete', { repoId, path });
}
