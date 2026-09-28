/**
 * 提交详情面板（T2.2）。
 *
 * # 它挂在哪里、数据从哪来
 *
 * 面板挂在 `RepoLayout` 既有的详情位（右侧 / 底部 / 隐藏由 `uiStore` 控制），
 * 而提交数据是 `HistoryPage` 通过 `useGraphQuery` 拉取的——两者在组件树上并不相邻
 * （面板在 `Outlet` 的**父级**，查询在 `Outlet` 的**子级**）。后端没有"按 oid 取单个
 * 提交"的命令，因此这里不另起查询，而是**读 TanStack Query 缓存**：
 * 用 `getQueriesData({ queryKey: logKeyPrefix(repoId) })` 把本仓库已加载的历史页
 * 全捞出来，按 oid 找到那条 `Commit`。缓存订阅用 `useSyncExternalStore`，
 * 于是"历史页又拉了一页""用户选了另一个提交"都能让面板自动更新。
 *
 * # 为什么不在面板里直接 fetch
 *
 * 那会绕过 `useGraphQuery` 的分页 / 去重 / `keepPreviousData`，等于把一个
 * 已经存在的真相源又复制了一份（AGENTS.md §6：服务端状态归 TanStack Query）。
 * 读缓存让"谁拥有数据"这件事只有一个答案。
 *
 * # 关于"查看变更"按钮（T2.4）
 *
 * `src/features/diff/DiffView` 只接受 `target: 'staged' | 'unstaged'`，
 * 底层是 `workspaceDiff`（工作区 vs HEAD / 索引），**没有**"以单个提交为 target"
 * 的 IPC 命令。低成本复用的前提不存在，因此这里只渲染元数据，
 * 差异视图留待 T2.4（届时应新增 `commit_diff` 之类的命令，而不是硬凑工作区通道）。
 */
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import type { ReactNode } from 'react';

import { useQueryClient } from '@tanstack/react-query';
import { Copy, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { normalizeError, useAppError } from '@/lib/errors';
import type { Commit, CommitSignature, GraphRow, HistoryPage } from '@/lib/ipc/history';
import { logKeyPrefix } from '@/lib/queryKeys';
import { cn } from '@/lib/utils';

import { IconButton } from '@/ui/components/icon-button';

import {
  absoluteTime,
  authorDisplay,
  shortOid,
  signatureKeySuffix,
} from '@/features/history/commitMeta';
import { useGraphSelectionStore } from '@/features/history/graphSelectionStore';

/** 详情面板要渲染的一条提交（提交本体 + 它在图上的行，行可能缺失）。 */
export interface CommitDetailEntry {
  readonly commit: Commit;
  readonly row: GraphRow | null;
}

/**
 * 从若干页历史里按 oid 找出提交（纯函数，导出供单测）。
 *
 * `row` 允许为 `null`：分页边界上可能先拿到提交、后拿到布局（或反之），
 * 缺行只是少显示"折叠了几个分支"，不该让整个面板消失。
 */
export function findEntryInPages(
  pages: readonly (HistoryPage | null | undefined)[],
  oid: string,
): CommitDetailEntry | null {
  for (const page of pages) {
    const commit = page?.commits.find((candidate) => candidate.oid === oid);
    if (commit !== undefined) {
      const row = page?.layout.rows.find((candidate) => candidate.oid === oid) ?? null;
      return { commit, row };
    }
  }
  return null;
}

/** 身份显示（`Name <email>`；与 `authorDisplay` 同一口径，但作用于任意签名）。 */
function personDisplay(signature: CommitSignature): string {
  const name = signature.name.trim();
  const email = signature.email.trim();
  if (email === '') {
    return name;
  }
  return name === '' ? email : `${name} <${email}>`;
}

export interface CommitDetailPanelProps {
  /** 仓库记录 id（非有限数时直接退回 `fallback`）。 */
  readonly repoId: number;
  /** 没有选中提交（或提交不在缓存里）时渲染的内容。 */
  readonly fallback?: ReactNode;
  readonly className?: string;
}

export function CommitDetailPanel({ repoId, fallback, className }: CommitDetailPanelProps) {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const queryClient = useQueryClient();

  const detailOid = useGraphSelectionStore((state) => state.detailOid);
  const setDetailOid = useGraphSelectionStore((state) => state.setDetailOid);
  const compareBaseOid = useGraphSelectionStore((state) => state.compareBaseOid);
  const setCompareBase = useGraphSelectionStore((state) => state.setCompareBase);

  const cache = queryClient.getQueryCache();
  const subscribe = useCallback(
    (onStoreChange: () => void) => cache.subscribe(onStoreChange),
    [cache],
  );
  // getSnapshot 必须返回**引用稳定**的值（useSyncExternalStore 的硬性要求，
  // 与 useGraphTheme.ts 的缓存约定一致）：findEntryInPages 每次都会新建
  // `{ commit, row }` 包装对象，直接返回会让 React 认为"存储每次都在变"，
  // 陷入无限重渲染（Maximum update depth exceeded）。
  // 因此把上一次发出去的包装对象存在 ref 里：只有当缓存里的 Commit / GraphRow
  // **对象身份**真正变化（翻页、refetch、切换 detailOid、提交被移出缓存）时
  // 才分配新包装；未变则原样交还上一次的引用。
  const entryRef = useRef<CommitDetailEntry | null>(null);
  const getSnapshot = useCallback((): CommitDetailEntry | null => {
    if (detailOid === null || !Number.isFinite(repoId)) {
      entryRef.current = null;
      return null;
    }
    const pages = queryClient
      .getQueriesData<HistoryPage>({ queryKey: logKeyPrefix(repoId) })
      .map(([, data]) => data);
    const found = findEntryInPages(pages, detailOid);
    const previous = entryRef.current;
    if (previous !== null && found !== null) {
      if (previous.commit === found.commit && previous.row === found.row) {
        return previous;
      }
    } else if (previous === found) {
      // 双 null：条目不存在且之前也不存在
      return previous;
    }
    entryRef.current = found;
    return found;
  }, [detailOid, queryClient, repoId]);

  const entry = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  // 复制成功的短暂反馈；切换提交时复位，避免"上一条的已复制"粘在这一条上。
  const [copied, setCopied] = useCopiedFlag(entry?.commit.oid ?? null);

  if (entry === null) {
    return <>{fallback}</>;
  }

  const { commit, row } = entry;
  const isCompareBase = compareBaseOid === commit.oid;
  const collapsedCount = row?.collapsed.length ?? 0;
  const authoredAt = absoluteTime(commit.author.time);
  const committedAt = absoluteTime(commit.committer.time);
  const body = commit.body?.trim() ?? '';

  return (
    <div className={cn('flex h-full min-h-0 flex-col gap-2 overflow-y-auto', className)}>
      <header className="flex items-start justify-between gap-2">
        <h2 className="text-13 font-medium">{t('history.detail.title')}</h2>
        <IconButton
          label={t('history.detail.close')}
          size="sm"
          onClick={() => {
            setDetailOid(null);
          }}
        >
          <X aria-hidden="true" className="size-3.5" />
        </IconButton>
      </header>

      {/* oid（可复制） */}
      <div className="flex flex-col gap-1">
        <span className="text-11 text-fg-subtle">{t('history.detail.oid')}</span>
        <div className="flex items-center gap-1">
          <code className="min-w-0 flex-1 truncate font-mono text-12" title={commit.oid}>
            {commit.oid}
          </code>
          <IconButton
            label={copied ? t('history.detail.copied') : t('history.detail.copyOid')}
            size="sm"
            onClick={() => {
              void navigator.clipboard.writeText(commit.oid).then(
                () => {
                  setCopied(true);
                },
                (error: unknown) => {
                  show(normalizeError(error));
                },
              );
            }}
          >
            <Copy aria-hidden="true" className="size-3.5" />
          </IconButton>
        </div>
      </div>

      {/* 提交信息：subject + body */}
      <section className="flex flex-col gap-1">
        <span className="text-11 text-fg-subtle">{t('history.detail.message')}</span>
        <p className="text-13 font-medium break-words">{commit.subject}</p>
        {body === '' ? (
          <p className="text-12 text-fg-subtle">{t('history.detail.noBody')}</p>
        ) : (
          <pre className="whitespace-pre-wrap break-words font-sans text-12 text-fg-muted">
            {body}
          </pre>
        )}
      </section>

      {/* 作者 / 提交者与时间 */}
      <dl className="flex flex-col gap-1.5 text-12">
        <Field label={t('history.detail.author')} value={authorDisplay(commit)} />
        <Field label={t('history.detail.authoredAt')} value={authoredAt ?? t('history.pending')} />
        <Field label={t('history.detail.committer')} value={personDisplay(commit.committer)} />
        <Field
          label={t('history.detail.committedAt')}
          value={committedAt ?? t('history.pending')}
        />
        <Field
          label={t('history.detail.signature')}
          value={t(`history.signature.${signatureKeySuffix(commit.signature)}`)}
        />
      </dl>

      {/* 父提交 */}
      <section className="flex flex-col gap-1">
        <span className="text-11 text-fg-subtle">{t('history.detail.parents')}</span>
        {commit.parents.length === 0 ? (
          <p className="text-12 text-fg-muted">{t('history.detail.noParents')}</p>
        ) : (
          <ul className="flex flex-wrap gap-1">
            {commit.parents.map((parent) => (
              <li
                key={parent}
                className="rounded-sm bg-surface-sunken px-1.5 py-0.5 font-mono text-11 text-fg-muted"
                title={parent}
              >
                {shortOid(parent)}
              </li>
            ))}
          </ul>
        )}
      </section>

      {/* 折叠摘要 */}
      {collapsedCount > 0 ? (
        <p className="rounded-md border border-line bg-surface-sunken px-2 py-1 text-11 text-fg-subtle">
          {t('history.detail.collapsed', { count: collapsedCount })}
        </p>
      ) : null}

      {/* 比较基准状态（T2.4 的差异视图会消费它） */}
      <section className="mt-auto flex flex-col gap-1 border-t border-line pt-2">
        <span className="text-11 text-fg-subtle">{t('history.detail.compareBase')}</span>
        {isCompareBase ? (
          <div className="flex items-center justify-between gap-2">
            <span className="text-12 text-brand">{t('history.detail.compareBaseSet')}</span>
            <IconButton
              label={t('history.detail.clearCompareBase')}
              size="sm"
              onClick={() => {
                setCompareBase(null);
              }}
            >
              <X aria-hidden="true" className="size-3.5" />
            </IconButton>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => {
              setCompareBase(commit.oid);
            }}
            className="fd-transition self-start rounded-md border border-line px-2 py-1 text-12 hover:bg-surface-sunken"
          >
            {t('history.menu.compareBaseSet')}
          </button>
        )}
      </section>
    </div>
  );
}

/** 一行"标签 + 值"（详情面板的元数据统一用它，避免每处各写一遍样式）。 */
function Field({ label, value }: { readonly label: string; readonly value: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-11 text-fg-subtle">{label}</dt>
      <dd className="break-words text-fg">{value}</dd>
    </div>
  );
}

/**
 * "已复制"的短暂反馈（1.5 秒后自动复位；切换提交时立即复位）。
 *
 * 单独抽出来是因为它带一个定时器 effect，混在主组件里会让"复制"这一段
 * 的状态机淹没在元数据渲染里。
 */
function useCopiedFlag(oid: string | null): [boolean, (value: boolean) => void] {
  // 派生而非 effect 重置：切换提交时 copied 自动变 false（react-hooks/set-state-in-effect）
  const [copiedOid, setCopiedOid] = useState<string | null>(null);
  const copied = copiedOid !== null && copiedOid === oid;

  useEffect(() => {
    if (copiedOid === null) {
      return;
    }
    const timer = setTimeout(() => {
      setCopiedOid(null);
    }, 1_500);
    return () => {
      clearTimeout(timer);
    };
  }, [copiedOid]);

  const setCopied = useCallback(
    (value: boolean) => {
      setCopiedOid(value ? oid : null);
    },
    [oid],
  );

  return [copied, setCopied];
}
