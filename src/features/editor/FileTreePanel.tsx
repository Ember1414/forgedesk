/**
 * 文件树面板（T5.7）：懒加载、Git 状态标记、常用操作。
 *
 * - 懒加载一层（点目录展开）；`fs_tree` 后端做 check-ignore 过滤；
 * - Git 状态标记来自 workspace_status 的文件清单（修改/新增/冲突 → 黄点）；
 * - 操作：新建文件/目录、重命名、删除（确认后进回收站）；
 * - 过滤器：仅显示已修改文件 / 显示隐藏文件。
 */
import { useCallback, useState } from 'react';

import { FilePlus2, FolderPlus, Pencil, RefreshCw, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useQuery, useQueryClient } from '@tanstack/react-query';

import { fsCreate, fsDelete, fsRename, fsTree } from '@/lib/ipc/fs';
import { workspaceStatus } from '@/lib/ipc/workspace';
import { STATUS_QUERY_KEY } from '@/lib/queryKeys';
import type { FsNode } from '@/lib/ipc/fs';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/ui/components/alert-dialog';
import { Button } from '@/ui/components/button';
import { Checkbox } from '@/ui/components/checkbox';
import { Input } from '@/ui/components/input';
import { cn } from '@/lib/utils';

export interface FileTreePanelProps {
  readonly repoId: number;
  /** 打开一个文件（父组件负责装进编辑器标签）。 */
  readonly onOpenFile: (path: string) => void;
}

type Prompt =
  | { readonly kind: 'newFile' | 'newDir'; readonly dir: string }
  | { readonly kind: 'rename'; readonly path: string; readonly name: string }
  | { readonly kind: 'delete'; readonly path: string; readonly name: string }
  | null;

function joinPath(dir: string, name: string): string {
  return dir === '' ? name : `${dir}/${name}`;
}

function parentOf(path: string): string {
  const parts = path.split('/');
  parts.pop();
  return parts.join('/');
}

export function FileTreePanel({ repoId, onOpenFile }: FileTreePanelProps) {
  const { t } = useTranslation('shell');
  const queryClient = useQueryClient();
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [showHidden, setShowHidden] = useState(false);
  const [onlyChanged, setOnlyChanged] = useState(false);
  const [prompt, setPrompt] = useState<Prompt>(null);
  const [promptText, setPromptText] = useState('');
  const [reloadTick, setReloadTick] = useState(0);

  const status = useQuery({
    queryKey: [STATUS_QUERY_KEY, repoId, false],
    queryFn: () => workspaceStatus(repoId, false),
  });
  const changedPaths = new Set(
    [
      ...(status.data?.staged ?? []),
      ...(status.data?.unstaged ?? []),
      ...(status.data?.untracked ?? []),
      ...(status.data?.conflicted ?? []),
    ].map((file) => file.path),
  );

  const tree = useQuery({
    queryKey: ['fs-tree', repoId, '', showHidden, reloadTick],
    queryFn: () => fsTree(repoId, '', { showHidden }),
  });

  const refresh = useCallback(() => {
    setReloadTick((tick) => tick + 1);
    void queryClient.invalidateQueries({ queryKey: [STATUS_QUERY_KEY, repoId] });
  }, [queryClient, repoId]);

  const toggle = useCallback((relPath: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(relPath)) {
        next.delete(relPath);
      } else {
        next.add(relPath);
      }
      return next;
    });
  }, []);

  const runPrompt = useCallback(async () => {
    if (prompt === null) {
      return;
    }
    try {
      switch (prompt.kind) {
        case 'newFile':
          await fsCreate(repoId, joinPath(prompt.dir, promptText), false);
          break;
        case 'newDir':
          await fsCreate(repoId, joinPath(prompt.dir, promptText), true);
          break;
        case 'rename':
          await fsRename(repoId, prompt.path, joinPath(parentOf(prompt.path), promptText));
          break;
        case 'delete':
          await fsDelete(repoId, prompt.path);
          break;
      }
      refresh();
    } finally {
      setPrompt(null);
      setPromptText('');
    }
  }, [prompt, promptText, refresh, repoId]);

  const visible = (tree.data ?? []).filter(
    (node) => !(onlyChanged && node.kind === 'file' && !changedPaths.has(node.relPath)),
  );

  return (
    <div className="border-line flex h-full min-h-0 flex-col border-e">
      <div className="border-line flex items-center gap-1 border-b px-2 py-1.5">
        <Button size="sm" variant="ghost" onClick={refresh} aria-label={t('editor.tree.refresh')}>
          <RefreshCw aria-hidden="true" className="size-3.5" />
        </Button>
        <Button
          size="sm"
          variant="ghost"
          aria-label={t('editor.tree.newFile')}
          onClick={() => setPrompt({ kind: 'newFile', dir: '' })}
        >
          <FilePlus2 aria-hidden="true" className="size-3.5" />
        </Button>
        <Button
          size="sm"
          variant="ghost"
          aria-label={t('editor.tree.newDir')}
          onClick={() => setPrompt({ kind: 'newDir', dir: '' })}
        >
          <FolderPlus aria-hidden="true" className="size-3.5" />
        </Button>
        <span className="flex-1" />
        <label className="text-fg-muted flex items-center gap-1 text-11">
          <Checkbox checked={showHidden} onCheckedChange={(c) => setShowHidden(c === true)} />
          {t('editor.tree.showHidden')}
        </label>
        <label className="text-fg-muted flex items-center gap-1 text-11">
          <Checkbox checked={onlyChanged} onCheckedChange={(c) => setOnlyChanged(c === true)} />
          {t('editor.tree.onlyChanged')}
        </label>
      </div>

      <div className="min-h-0 flex-1 overflow-auto p-1">
        {visible.map((node) => (
          <TreeRow
            key={node.relPath}
            node={node}
            repoId={repoId}
            depth={0}
            expanded={expanded}
            changedPaths={changedPaths}
            onToggle={toggle}
            onOpenFile={onOpenFile}
            refresh={refresh}
            onPrompt={setPrompt}
          />
        ))}
      </div>

      {prompt !== null ? (
        <AlertDialog open>
          <AlertDialogContent
            tone={prompt.kind === 'delete' ? 'danger' : undefined}
            impact={
              prompt.kind === 'delete'
                ? t('editor.tree.deleteImpact', { name: prompt.name })
                : t('editor.tree.promptImpact')
            }
          >
            {/* 标题/描述与按钮必须各自成组：`AlertDialogHeader` / `Footer` 提供
                垂直间距与右对齐（Footer 的 mt-5 就是输入框与按钮之间那道缝）。
                此前直接平铺在 Content 里，输入框与"确认/取消"贴在一起——
                用户反馈的"确认取消框与名称框有细微重叠"（2026-10-08）。 */}
            <AlertDialogHeader>
              <AlertDialogTitle>
                {prompt.kind === 'newFile'
                  ? t('editor.tree.newFile')
                  : prompt.kind === 'newDir'
                    ? t('editor.tree.newDir')
                    : prompt.kind === 'rename'
                      ? t('editor.tree.rename')
                      : t('editor.tree.deleteTitle')}
              </AlertDialogTitle>
              <AlertDialogDescription>
                {prompt.kind === 'delete' ? prompt.path : t('editor.tree.promptName')}
              </AlertDialogDescription>
            </AlertDialogHeader>
            {prompt.kind !== 'delete' ? (
              <Input
                srLabel={t('editor.tree.promptName')}
                value={promptText}
                autoFocus
                onChange={(event) => setPromptText(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' && promptText.trim() !== '') {
                    void runPrompt();
                  }
                }}
              />
            ) : null}
            <AlertDialogFooter>
              {/* 取消在左、确认在右（与全站其它确认框一致）；焦点默认落在取消上 ——
                  删除是不可撤销的，默认动作不该是它。 */}
              <AlertDialogCancel
                onClick={() => {
                  setPrompt(null);
                  setPromptText('');
                }}
              >
                {t('editor.tree.promptCancel')}
              </AlertDialogCancel>
              <AlertDialogAction
                // 只有删除是破坏性操作：新建/重命名的确认按钮不该长成危险红色
                destructive={prompt.kind === 'delete'}
                onClick={() => void runPrompt()}
                disabled={prompt.kind !== 'delete' && promptText.trim() === ''}
              >
                {t('editor.tree.promptConfirm')}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </div>
  );
}

function TreeRow({
  node,
  repoId,
  depth,
  expanded,
  changedPaths,
  onToggle,
  onOpenFile,
  refresh,
  onPrompt,
}: {
  readonly node: FsNode;
  readonly repoId: number;
  readonly depth: number;
  readonly expanded: ReadonlySet<string>;
  readonly changedPaths: ReadonlySet<string>;
  readonly onToggle: (relPath: string) => void;
  readonly onOpenFile: (path: string) => void;
  readonly refresh: () => void;
  readonly onPrompt: (prompt: Prompt) => void;
}) {
  const { t } = useTranslation('shell');
  const isDir = node.kind === 'dir';
  const isExpanded = expanded.has(node.relPath);
  const children = useQuery({
    queryKey: ['fs-tree', repoId, node.relPath, isExpanded],
    queryFn: () => fsTree(repoId, node.relPath, { showHidden: false }),
    enabled: isDir && isExpanded,
  });
  const changed = changedPaths.has(node.relPath);

  return (
    <div>
      <div
        className={cn(
          'fd-transition group flex items-center gap-1 rounded-md px-1 py-0.5 text-13 hover:bg-surface-sunken',
          changed && 'text-warning',
        )}
        style={{ paddingInlineStart: `${depth * 14 + 4}px` }}
      >
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center gap-1 text-start"
          onClick={() => (isDir ? onToggle(node.relPath) : onOpenFile(node.relPath))}
          aria-expanded={isDir ? isExpanded : undefined}
        >
          <span aria-hidden="true" className="w-3 text-center">
            {isDir ? (isExpanded ? '▾' : '▸') : ''}
          </span>
          <span className="truncate">{node.name}</span>
        </button>
        {changed ? <span className="text-11">●</span> : null}
        <span className="opacity-0 group-hover:opacity-100 focus-within:opacity-100">
          <button
            type="button"
            className="text-fg-muted hover:text-fg p-0.5"
            aria-label={t('editor.tree.renameAria', { name: node.name })}
            onClick={() => onPrompt({ kind: 'rename', path: node.relPath, name: node.name })}
          >
            <Pencil aria-hidden="true" className="size-3" />
          </button>
          <button
            type="button"
            className="text-fg-muted hover:text-danger p-0.5"
            aria-label={t('editor.tree.deleteAria', { name: node.name })}
            onClick={() => onPrompt({ kind: 'delete', path: node.relPath, name: node.name })}
          >
            <Trash2 aria-hidden="true" className="size-3" />
          </button>
        </span>
      </div>
      {isDir && isExpanded
        ? (children.data ?? []).map((child) => (
            <TreeRow
              key={child.relPath}
              node={child}
              repoId={repoId}
              depth={depth + 1}
              expanded={expanded}
              changedPaths={changedPaths}
              onToggle={onToggle}
              onOpenFile={onOpenFile}
              refresh={refresh}
              onPrompt={onPrompt}
            />
          ))
        : null}
    </div>
  );
}
