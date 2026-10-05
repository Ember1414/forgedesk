/**
 * 文件历史与 Blame（T5.8）的前端封装。
 */
import { invokeCommand } from './client';

/** 一行 blame（逐行归属）。 */
export interface BlameLine {
  readonly oid: string;
  readonly shortOid: string;
  readonly lineNo: number;
  readonly author: string;
  readonly authorMail: string;
  readonly authorTime: number;
  readonly summary: string;
  readonly isUncommitted: boolean;
  readonly previousPath: string | null;
}

/** blame 选项。 */
export interface BlameOptions {
  readonly ignoreWhitespace?: boolean;
  readonly detectMoves?: boolean;
  /** `-L <start>,<end>`：降级模式只 blame 可见区间。 */
  readonly range?: string;
}

/** 文件历史条目（变更类型 A/M/D/R）。 */
export interface FileHistoryEntry {
  readonly oid: string;
  readonly author: string;
  readonly authorTime: number;
  readonly subject: string;
  readonly changeKind: string;
  readonly oldPath: string | null;
}

/** 文件历史分页。 */
export interface FileHistoryPage {
  readonly items: readonly FileHistoryEntry[];
  readonly nextCursor: number | null;
}

/** 逐行 blame（后端 `--line-porcelain` 解析）。 */
export function gitBlame(
  repoId: number,
  path: string,
  options?: BlameOptions,
): Promise<BlameLine[]> {
  return invokeCommand<BlameLine[]>('git_blame', {
    repoId,
    path,
    ...(options === undefined ? {} : options),
  });
}

/** 文件历史（--follow 跟随重命名）。 */
export function gitFileHistory(
  repoId: number,
  path: string,
  options?: { readonly follow?: boolean; readonly limit?: number; readonly cursor?: number },
): Promise<FileHistoryPage> {
  return invokeCommand<FileHistoryPage>('git_file_history', {
    repoId,
    path,
    ...(options?.follow === undefined ? {} : { follow: options.follow }),
    ...(options?.limit === undefined ? {} : { limit: options.limit }),
    ...(options?.cursor === undefined ? {} : { cursor: options.cursor }),
  });
}

/** 历史版本的文件内容（base64）。 */
export function gitFileAt(
  repoId: number,
  path: string,
  rev: string,
): Promise<{ readonly contentBase64: string; readonly isBinary: boolean; readonly size: number }> {
  return invokeCommand('git_file_at', { repoId, path, rev });
}
