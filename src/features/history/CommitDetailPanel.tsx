/**
 * 提交详情面板（T2.4）。
 *
 * # 它挂在哪里、数据从哪来
 *
 * 面板挂在 `RepoLayout` 既有的详情位（右侧 / 底部 / 隐藏由 `uiStore` 控制）。
 * 数据来自 `git_commit_detail`（TanStack Query，键见 `commitDetailKey`）：
 * 元数据（含正文与签名）、refs、相对所选父提交的统计与文件清单、状态标记。
 * 文件清单的**行级内容**按需经 `workspace_diff`（`between` 目标）拉取并交给
 * `DiffView` 渲染——不新增第二对 diff 命令（见 API.md 的说明）。
 *
 * # 钉住 / 跟随（T2.4）
 *
 * 两种模式的真相源是 `graphSelectionStore.detailPinned`：跟随（缺省）下选中即展示；
 * 钉住后 `select` 不再改写 `detailOid`（store 里的规则），面板冻结在当前提交上，
 * 用户可以放心点别处对照。"定位父提交 / 子提交"是主动导航，会**解除钉住**。
 *
 * # "在历史中定位"的范围边界
 *
 * 定位只对**已加载进 Query 缓存**的历史页生效（`buildNeighbourIndex`）——
 * 没加载到的提交既不在选中序号里、画布上也未必画了。按钮对此如实禁用，
 * 而不是悄悄跳转到错误的位置。子提交靠"谁的 parents 里有它"从缓存反推。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';

import { keepPreviousData, useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, ChevronRight, Copy, ExternalLink, Pin, PinOff, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { normalizeError, useAppError } from '@/lib/errors';
import { gitCommitDetail } from '@/lib/ipc/commitDetail';
import type { CommitDetail, CommitFileChange } from '@/lib/ipc/commitDetail';
import { workspaceDiffPatch } from '@/lib/ipc/workspace';
import type { HistoryPage } from '@/lib/ipc/history';
import { commitDetailKey, logKeyPrefix } from '@/lib/queryKeys';
import { cn } from '@/lib/utils';

import { DiffView } from '@/features/diff/DiffView';
import { IconButton } from '@/ui/components/icon-button';
import { Skeleton } from '@/ui/components/skeleton';
import { ToggleGroup } from '@/ui/components/toggle-group';

import {
  absoluteTime,
  parseRefs,
  shortOid,
  signatureKeySuffix,
} from '@/features/history/commitMeta';
import { refChipClass } from '@/features/history/graphTheme';
import { NO_MODIFIERS, useGraphSelectionStore } from '@/features/history/graphSelectionStore';

/** 详情面板要渲染的一条提交在缓存里的"邻居"：行序 + 反向的子提交索引。 */
export interface NeighbourIndex {
  /** 行序的 oid（按全局行号升序；跨页去重）。 */
  readonly order: readonly string[];
  /** oid → 直接子提交（缓存里谁的 parents 含它）。 */
  readonly childrenOf: ReadonlyMap<string, readonly string[]>;
}

/**
 * 从若干页历史里构建邻居索引（纯函数，导出供单测）。
 *
 * 行序取自 `GraphRow.row`（服务层已平移成全局行号）；同一 oid 出现在多页时
 * （翻页重叠）取最小行号。子索引遍历每条提交的 parents——O(提交数 × 父数)，
 * 对 ≤500 行/页的缓存来说是微秒级。
 */
export function buildNeighbourIndex(
  pages: readonly (HistoryPage | null | undefined)[],
): NeighbourIndex {
  const rowByOid = new Map<string, number>();
  const childrenOf = new Map<string, string[]>();
  for (const page of pages) {
    for (const row of page?.layout.rows ?? []) {
      const existing = rowByOid.get(row.oid);
      if (existing === undefined || row.row < existing) {
        rowByOid.set(row.oid, row.row);
      }
    }
    for (const commit of page?.commits ?? []) {
      for (const parent of commit.parents) {
        const children = childrenOf.get(parent);
        if (children === undefined) {
          childrenOf.set(parent, [commit.oid]);
        } else if (!children.includes(commit.oid)) {
          children.push(commit.oid);
        }
      }
    }
  }
  const order = [...rowByOid.entries()]
    .sort((left, right) => left[1] - right[1])
    .map(([oid]) => oid);
  return { order, childrenOf };
}

/**
 * 由仓库网页根 URL 拼出该提交的页面（纯函数，导出供单测）。
 *
 * 三个托管商的提交页路径不同（GitHub `/commit/`、GitLab `/-/commit/`、
 * Bitbucket `/commits/`）；后端只推断仓库根（见 services::commit_detail 的
 * `infer_web_url`），按 host 分派的收尾工作在这里。
 */
export function commitWebUrl(webUrl: string, oid: string): string {
  const root = webUrl.replace(/\/+$/, '');
  if (root.includes('//gitlab.com/')) {
    return `${root}/-/commit/${oid}`;
  }
  if (root.includes('//bitbucket.org/')) {
    return `${root}/commits/${oid}`;
  }
  return `${root}/commit/${oid}`;
}

/** 身份显示（`Name <email>`；与 `authorDisplay` 同一口径，但作用于任意签名）。 */
function personDisplay(signature: { readonly name: string; readonly email: string }): string {
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
  /** 没有选中提交（或查询未启用）时渲染的内容。 */
  readonly fallback?: ReactNode;
  readonly className?: string;
}

export function CommitDetailPanel({ repoId, fallback, className }: CommitDetailPanelProps) {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const queryClient = useQueryClient();

  const detailOid = useGraphSelectionStore((state) => state.detailOid);
  const detailPinned = useGraphSelectionStore((state) => state.detailPinned);
  const setDetailPinned = useGraphSelectionStore((state) => state.setDetailPinned);
  const setDetailOid = useGraphSelectionStore((state) => state.setDetailOid);
  const select = useGraphSelectionStore((state) => state.select);
  const compareBaseOid = useGraphSelectionStore((state) => state.compareBaseOid);
  const setCompareBase = useGraphSelectionStore((state) => state.setCompareBase);
  const requestRebase = useGraphSelectionStore((state) => state.requestRebase);

  // 双父视图选择：切换提交时自动回到第一父——用"记住上次属于哪个 oid"派生，
  // 而不是 effect 里 setState（react-hooks/set-state-in-effect）。
  const [parentChoice, setParentChoice] = useState<{ oid: string; index: number }>({
    oid: '',
    index: 0,
  });
  const parentIndex = detailOid !== null && parentChoice.oid === detailOid ? parentChoice.index : 0;

  // 展开的文件同样跟随提交：换提交后展开集归零
  const [expandedFor, setExpandedFor] = useState<{ oid: string; paths: ReadonlySet<string> }>({
    oid: '',
    paths: new Set<string>(),
  });
  const expandedPaths =
    detailOid !== null && expandedFor.oid === detailOid ? expandedFor.paths : EMPTY_PATH_SET;

  const query = useQuery({
    queryKey: commitDetailKey(repoId, detailOid ?? 'none', parentIndex),
    queryFn: () => gitCommitDetail(repoId, detailOid ?? '', parentIndex),
    enabled: detailOid !== null && Number.isFinite(repoId),
    placeholderData: keepPreviousData,
    retry: false,
  });
  const detail = detailOid === null ? undefined : query.data;

  // 邻居索引（定位父/子提交用）：详情数据一变就重算（翻页后按钮的可用态会跟上）
  const pagesVersion = query.dataUpdatedAt;
  const neighbours = useMemo(() => {
    // pagesVersion 是缓存内容的版本号（翻页 / 失效后变化）：索引纯函数不消费它，
    // 但必须在回调里建立引用，否则 exhaustive-deps 会把它当多余依赖——
    // 那样"加载更多之后定位按钮的可用态"就不更新了
    void pagesVersion;
    return buildNeighbourIndex(
      queryClient
        .getQueriesData<HistoryPage>({ queryKey: logKeyPrefix(repoId) })
        .map(([, data]) => data),
    );
  }, [queryClient, repoId, pagesVersion]);

  // 复制成功的短暂反馈：记录"复制的是哪一项"（oid/邮箱/消息/补丁各有一枚按钮）
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffectOnCopied(copiedKey, setCopiedKey, copiedTimer);

  const copyText = useCallback(
    (key: string, text: string) => {
      void navigator.clipboard.writeText(text).then(
        () => {
          if (copiedTimer.current !== null) {
            clearTimeout(copiedTimer.current);
          }
          setCopiedKey(key);
        },
        (error: unknown) => {
          show(normalizeError(error));
        },
      );
    },
    [show],
  );

  /** 定位父/子提交：主动导航 → 解除钉住、选中并跟随展示。 */
  const locate = useCallback(
    (target: string) => {
      if (!neighbours.order.includes(target)) {
        return;
      }
      setDetailPinned(false);
      select(target, NO_MODIFIERS, neighbours.order);
    },
    [neighbours, select, setDetailPinned],
  );

  const copyPatch = useCallback(
    (current: CommitDetail) => {
      // 与详情文件清单同一父口径；根提交走 `commit` 目标（引擎内部与空树比较）
      const from = current.meta.parents[current.parentIndex];
      const spec =
        from === undefined
          ? { target: 'commit' as const, revision: current.meta.oid, forceFull: true }
          : {
              target: 'between' as const,
              from,
              to: current.meta.oid,
              forceFull: true,
            };
      void workspaceDiffPatch(repoId, spec).then(
        (bytes) => {
          copyText('patch', new TextDecoder().decode(new Uint8Array(bytes)));
        },
        (error: unknown) => {
          show(normalizeError(error));
        },
      );
    },
    [copyText, repoId, show],
  );

  if (detailOid === null || !Number.isFinite(repoId)) {
    return <>{fallback}</>;
  }

  if (query.isError) {
    const normalized = normalizeError(query.error);
    return (
      <div className={cn('flex h-full min-h-0 flex-col gap-2', className)}>
        <PanelHeader
          pinned={detailPinned}
          onTogglePin={() => {
            setDetailPinned(!detailPinned);
          }}
          onClose={() => {
            setDetailOid(null);
          }}
        />
        <p className="text-12 text-danger">{t('history.detail.error')}</p>
        {normalized.detail === undefined ? null : (
          <p className="break-words font-mono text-11 text-fg-subtle">{normalized.detail}</p>
        )}
      </div>
    );
  }

  if (detail === undefined) {
    return (
      <div className={cn('flex h-full min-h-0 flex-col gap-2', className)}>
        <PanelHeader
          pinned={detailPinned}
          onTogglePin={() => {
            setDetailPinned(!detailPinned);
          }}
          onClose={() => {
            setDetailOid(null);
          }}
        />
        <div aria-busy="true" className="flex flex-col gap-1.5">
          <Skeleton className="h-4 w-3/4" />
          <Skeleton className="h-3 w-full" />
          <Skeleton className="h-3 w-2/3" />
          <Skeleton className="h-3 w-1/2" />
        </div>
      </div>
    );
  }

  const { meta } = detail;
  const children = neighbours.childrenOf.get(meta.oid) ?? [];
  const isCompareBase = compareBaseOid === meta.oid;
  const authoredAt = absoluteTime(meta.author.time);
  const committedAt = absoluteTime(meta.committer.time);
  const body = meta.body?.trim() ?? '';
  const diffParentOid = meta.parents[detail.parentIndex];
  const webUrl = detail.webUrl === null ? null : commitWebUrl(detail.webUrl, meta.oid);

  return (
    <div
      className={cn('flex h-full min-h-0 flex-col gap-2 overflow-y-auto', className)}
      data-testid="commit-detail-panel"
    >
      <PanelHeader
        pinned={detailPinned}
        onTogglePin={() => {
          setDetailPinned(!detailPinned);
        }}
        onClose={() => {
          setDetailOid(null);
        }}
      />

      {/* oid（可复制） */}
      <div className="flex flex-col gap-1">
        <span className="text-11 text-fg-subtle">{t('history.detail.oid')}</span>
        <div className="flex items-center gap-1">
          <code className="min-w-0 flex-1 truncate font-mono text-12" title={meta.oid}>
            {meta.oid}
          </code>
          <CopyButton
            labelKey={copiedKey === 'oid' ? 'history.detail.copied' : 'history.detail.copyOid'}
            onClick={() => {
              copyText('oid', meta.oid);
            }}
          />
          <CopyButton
            labelKey={
              copiedKey === 'shortOid' ? 'history.detail.copied' : 'history.detail.copyShortOid'
            }
            onClick={() => {
              copyText('shortOid', meta.shortOid);
            }}
          />
        </div>
      </div>

      {/* 状态标记：HEAD / 推送状态 / refs 胶囊。文字 + 色彩双重编码（不只靠颜色）。 */}
      <div className="flex flex-wrap items-center gap-1">
        {detail.isHead ? (
          <span className="rounded-full bg-brand-subtle px-1.5 text-10 font-medium text-brand">
            {t('history.detail.headBadge')}
          </span>
        ) : null}
        <span
          className={cn(
            'rounded-full px-1.5 text-10',
            detail.isPushed ? 'bg-success-subtle text-success' : 'bg-surface-sunken text-fg-subtle',
          )}
        >
          {detail.isPushed ? t('history.detail.pushed') : t('history.detail.notPushed')}
        </span>
        {parseRefs(detail.refs).map((ref) => (
          <span
            key={`${ref.kind}:${ref.label}`}
            className={cn(
              'max-w-full truncate rounded-full border px-1.5 text-10',
              refChipClass(ref.kind),
            )}
            title={ref.label}
          >
            {ref.label}
          </span>
        ))}
        {webUrl === null ? null : (
          <button
            type="button"
            className="fd-transition flex min-w-0 items-center gap-1 rounded-full border border-line px-1.5 text-10 text-fg-subtle hover:bg-surface-sunken"
            title={t('history.detail.openInBrowserTitle', { url: webUrl })}
            onClick={() => {
              copyText('webUrl', webUrl);
            }}
          >
            <ExternalLink aria-hidden="true" className="size-3 shrink-0" />
            <span className="truncate">
              {copiedKey === 'webUrl'
                ? t('history.detail.copied')
                : t('history.detail.openInBrowser')}
            </span>
          </button>
        )}
      </div>

      {/* 提交信息：subject + body（整条可复制） */}
      <section className="flex flex-col gap-1">
        <div className="flex items-center justify-between gap-1">
          <span className="text-11 text-fg-subtle">{t('history.detail.message')}</span>
          <CopyButton
            labelKey={
              copiedKey === 'message' ? 'history.detail.copied' : 'history.detail.copyMessage'
            }
            onClick={() => {
              copyText('message', body === '' ? meta.subject : `${meta.subject}\n\n${body}`);
            }}
          />
        </div>
        <p className="text-13 font-medium break-words">{meta.subject}</p>
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
        <Field label={t('history.detail.author')} value={personDisplay(meta.author)} />
        <Field label={t('history.detail.authoredAt')} value={authoredAt ?? t('history.pending')} />
        <Field label={t('history.detail.committer')} value={personDisplay(meta.committer)} />
        <Field
          label={t('history.detail.committedAt')}
          value={committedAt ?? t('history.pending')}
        />
        <Field
          label={t('history.detail.signature')}
          value={t(`history.signature.${signatureKeySuffix(meta.signature)}`)}
        />
        {/* 作者邮箱单独给一枚复制（T2.4：元信息区可复制） */}
        <div className="flex items-center justify-between gap-1">
          <dt className="text-11 text-fg-subtle">{t('history.detail.authorEmail')}</dt>
          <CopyButton
            labelKey={copiedKey === 'email' ? 'history.detail.copied' : 'history.detail.copyEmail'}
            onClick={() => {
              copyText('email', meta.author.email);
            }}
          />
        </div>
      </dl>

      {/* 父提交（可定位）与子提交（由缓存反推） */}
      <section className="flex flex-col gap-1">
        <span className="text-11 text-fg-subtle">{t('history.detail.parents')}</span>
        {meta.parents.length === 0 ? (
          <p className="text-12 text-fg-muted">{t('history.detail.noParents')}</p>
        ) : (
          <ul className="flex flex-wrap gap-1">
            {meta.parents.map((parent, index) => (
              <li key={parent}>
                <button
                  type="button"
                  disabled={!neighbours.order.includes(parent)}
                  title={
                    neighbours.order.includes(parent)
                      ? t('history.detail.locateTitle')
                      : t('history.detail.notLoaded')
                  }
                  onClick={() => {
                    locate(parent);
                  }}
                  className="fd-transition rounded-sm bg-surface-sunken px-1.5 py-0.5 font-mono text-11 text-fg-muted enabled:hover:bg-brand-subtle enabled:hover:text-brand disabled:opacity-50"
                >
                  {t('history.detail.parentChip', {
                    ordinal: index + 1,
                    oid: shortOid(parent),
                  })}
                </button>
              </li>
            ))}
          </ul>
        )}
        {children.length > 0 ? (
          <>
            <span className="mt-1 text-11 text-fg-subtle">{t('history.detail.children')}</span>
            <ul className="flex flex-wrap gap-1">
              {children.map((child) => (
                <li key={child}>
                  <button
                    type="button"
                    title={t('history.detail.locateTitle')}
                    onClick={() => {
                      locate(child);
                    }}
                    className="fd-transition rounded-sm bg-surface-sunken px-1.5 py-0.5 font-mono text-11 text-fg-muted hover:bg-brand-subtle hover:text-brand"
                  >
                    {shortOid(child)}
                  </button>
                </li>
              ))}
            </ul>
          </>
        ) : null}
      </section>

      {/* 统计 + 合并提交的父选择（验收项：双父都能看） */}
      <section className="flex flex-col gap-1.5 border-t border-line pt-2">
        <div className="flex items-center justify-between gap-2">
          <span className="text-11 text-fg-subtle">{t('history.detail.changes')}</span>
          <span className="font-mono text-11 text-fg-muted">
            {t('history.detail.statsLine', {
              files: detail.stats.filesChanged,
              insertions: detail.stats.insertions,
              deletions: detail.stats.deletions,
            })}
          </span>
        </div>
        {detail.isMerge ? (
          <ToggleGroup
            label={t('history.detail.parentToggle')}
            value={String(detail.parentIndex)}
            options={meta.parents.slice(0, 2).map((parent, index) => ({
              value: String(index),
              label: t(index === 0 ? 'history.detail.firstParent' : 'history.detail.secondParent', {
                oid: shortOid(parent),
              }),
            }))}
            onValueChange={(next) => {
              setParentChoice({ oid: meta.oid, index: Number(next) || 0 });
            }}
          />
        ) : null}
      </section>

      {/* 文件清单：点击展开单个文件的 diff（复用 T1.5 的 DiffView，between 模式） */}
      <section className="flex flex-col gap-1">
        <span className="text-11 text-fg-subtle">{t('history.detail.files')}</span>
        {detail.files.length === 0 ? (
          <p className="text-12 text-fg-muted">{t('history.detail.noFiles')}</p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {detail.files.map((file) => (
              <FileRow
                key={`${file.kind}:${file.path}`}
                repoId={repoId}
                file={file}
                expanded={expandedPaths.has(file.path)}
                diffSource={
                  diffParentOid === undefined ? undefined : { from: diffParentOid, to: meta.oid }
                }
                onToggle={() => {
                  setExpandedFor((previous) => {
                    const current =
                      previous.oid === meta.oid ? new Set(previous.paths) : new Set<string>();
                    if (current.has(file.path)) {
                      current.delete(file.path);
                    } else {
                      current.add(file.path);
                    }
                    return { oid: meta.oid, paths: current };
                  });
                }}
              />
            ))}
          </ul>
        )}
      </section>

      {/* 比较基准（T2.4 的差异视图会消费它） */}
      <section className="flex flex-col gap-1 border-t border-line pt-2">
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
              setCompareBase(meta.oid);
            }}
            className="fd-transition self-start rounded-md border border-line px-2 py-1 text-12 hover:bg-surface-sunken"
          >
            {t('history.menu.compareBaseSet')}
          </button>
        )}
      </section>

      {/* 底部操作条：复制为 patch 与整理历史可用；拣选/反转/重置在历史页的
          操作面板（HistoryOpsPanel），建标签/分支在分支页——这里不再放禁用占位 */}
      <footer className="mt-auto flex flex-wrap gap-1 border-t border-line pt-2">
        <button
          type="button"
          onClick={() => {
            copyPatch(detail);
          }}
          className="fd-transition rounded-md border border-line px-2 py-1 text-11 hover:bg-surface-sunken"
        >
          {copiedKey === 'patch' ? t('history.detail.copied') : t('history.detail.copyPatch')}
        </button>
        {/* 整理历史（T3.6）：以当前提交为区间右端打开拖拽面板（R7：写操作经预览与快照） */}
        <button
          type="button"
          onClick={() => {
            requestRebase({ oids: [meta.oid] });
          }}
          className="fd-transition rounded-md border border-line px-2 py-1 text-11 hover:bg-surface-sunken"
          data-testid="detail-organize-history"
        >
          {t('history.detail.organizeHistory')}
        </button>
      </footer>
    </div>
  );
}

/** 空展开集（模块级：每次切换提交都要一个空集，不该每次新建）。 */
const EMPTY_PATH_SET: ReadonlySet<string> = new Set<string>();

/** 面板头部：标题 + 钉住开关 + 关闭。三个分支（正常/加载/错误）共用。 */
function PanelHeader({
  pinned,
  onTogglePin,
  onClose,
}: {
  readonly pinned: boolean;
  readonly onTogglePin: () => void;
  readonly onClose: () => void;
}) {
  const { t } = useTranslation('shell');
  return (
    <header className="flex items-start justify-between gap-2">
      <h2 className="text-13 font-medium">{t('history.detail.title')}</h2>
      <div className="flex items-center gap-0.5">
        <IconButton
          label={pinned ? t('history.detail.unpin') : t('history.detail.pin')}
          size="sm"
          aria-pressed={pinned}
          data-testid="commit-detail-pin"
          onClick={onTogglePin}
        >
          {pinned ? (
            <PinOff aria-hidden="true" className="size-3.5" />
          ) : (
            <Pin aria-hidden="true" className="size-3.5" />
          )}
        </IconButton>
        <IconButton label={t('history.detail.close')} size="sm" onClick={onClose}>
          <X aria-hidden="true" className="size-3.5" />
        </IconButton>
      </div>
    </header>
  );
}

/** 一枚复制按钮（文案由调用方给 key：默认"复制"或"已复制"）。 */
function CopyButton({
  labelKey,
  onClick,
}: {
  readonly labelKey: string;
  readonly onClick: () => void;
}) {
  const { t } = useTranslation('shell');
  return (
    <IconButton label={t(labelKey)} size="sm" onClick={onClick}>
      <Copy aria-hidden="true" className="size-3.5" />
    </IconButton>
  );
}

/** 文件清单的一行：点击展开（展开时在行内渲染 DiffView，between 模式）。 */
function FileRow({
  repoId,
  file,
  expanded,
  diffSource,
  onToggle,
}: {
  readonly repoId: number;
  readonly file: CommitFileChange;
  readonly expanded: boolean;
  readonly diffSource: { readonly from: string; readonly to: string } | undefined;
  readonly onToggle: () => void;
}) {
  const { t } = useTranslation('shell');
  return (
    <li className="min-w-0">
      <button
        type="button"
        aria-expanded={expanded}
        onClick={onToggle}
        className="fd-transition flex w-full min-w-0 items-center gap-1.5 rounded-sm px-1 py-0.5 text-left text-12 hover:bg-surface-sunken"
        title={file.oldPath === null ? file.path : `${file.oldPath} → ${file.path}`}
      >
        {expanded ? (
          <ChevronDown aria-hidden="true" className="size-3 shrink-0 text-fg-subtle" />
        ) : (
          <ChevronRight aria-hidden="true" className="size-3 shrink-0 text-fg-subtle" />
        )}
        <span className="min-w-0 flex-1 truncate font-mono text-11">{file.path}</span>
        {file.oldPath !== null ? (
          <span className="shrink-0 rounded-full border border-line px-1 text-10 text-fg-subtle">
            {t('history.detail.renamedBadge')}
          </span>
        ) : null}
        <span className="shrink-0 rounded-full bg-surface-sunken px-1 text-10 text-fg-subtle">
          {t(`history.detail.kind.${file.kind}`)}
        </span>
        {file.binary ? (
          <span className="shrink-0 font-mono text-10 text-fg-subtle">
            {t('history.detail.binaryBadge')}
          </span>
        ) : (
          <>
            <span className="shrink-0 font-mono text-10 text-success">+{file.additions}</span>
            <span className="shrink-0 font-mono text-10 text-danger">−{file.deletions}</span>
          </>
        )}
      </button>
      {/* 展开的 diff 容器用固定高而不是 max-h：VirtualList 量的就是这个容器的
          contentRect，max-h 不参与布局，量出来是 0，列表一行都画不出 */}
      {expanded && diffSource !== undefined ? (
        <div className="mt-1 h-80 overflow-y-auto rounded-md border border-line">
          {/* 行级内容复用 T1.5 的 DiffView：source 触发 between 模式（无暂存动作）。
              DiffView 根容器不带 h-full（在 Sheet 里靠父级拉伸），这里必须显式给，
              否则 VirtualList 量到的高度是 0、一行都画不出 */}
          <DiffView repoId={repoId} path={file.path} source={diffSource} className="h-full" />
        </div>
      ) : null}
    </li>
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
 * "已复制"的 1.5 秒复位定时器（抽出来让主组件少一段定时器状态机）。
 *
 * 独立 effect 的原因：定时器只与 copiedKey 相关，与渲染无关；
 * 卸载时清掉，避免对已卸载组件 setState。
 */
function useEffectOnCopied(
  copiedKey: string | null,
  setCopiedKey: (key: string | null) => void,
  timerRef: { current: ReturnType<typeof setTimeout> | null },
): void {
  useEffect(() => {
    if (copiedKey === null) {
      return;
    }
    timerRef.current = setTimeout(() => {
      setCopiedKey(null);
    }, 1_500);
    return () => {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [copiedKey, setCopiedKey, timerRef]);
}
