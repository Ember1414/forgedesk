/**
 * 解释卡片（T5.4）：非模态浮层，用人话解释终端里的 git 命令。
 *
 * 触发：Enter 时命中的条目风险 ≠ safe（自动），或 Ctrl+/（显式，无论风险）。
 * 内容全部来自本地知识库（assets/git-explains.yaml，构建期打包）；
 * 文档链接经 systemOpenUrl 打开（后端有 http(s) 白名单校验）。
 */
import { useCallback } from 'react';

import { BookOpen, CircleAlert, CircleCheck, X, TriangleAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { systemOpenUrl } from '@/lib/ipc';
import { IconButton } from '@/ui/components/icon-button';
import { cn } from '@/lib/utils';

import type { ExplainEntry, ExplainRisk, GitExplain } from '@/features/terminal/explainer';

export interface ExplainCardProps {
  readonly explain: GitExplain;
  readonly repoId: number;
  /** 插入典型用法到当前行。 */
  readonly onInsert: (example: string) => void;
  readonly onClose: () => void;
}

/** 风险徽标的图标与配色（图标 + 文字双重表达）。 */
function RiskBadge({ risk }: { readonly risk: ExplainRisk }) {
  const { t } = useTranslation('shell');
  const map = {
    safe: { icon: CircleCheck, className: 'text-success' },
    caution: { icon: CircleAlert, className: 'text-warning' },
    dangerous: { icon: TriangleAlert, className: 'text-danger' },
  } as const;
  const { icon: Icon, className } = map[risk];
  return (
    <span className={cn('flex items-center gap-1 text-12 font-medium', className)}>
      <Icon aria-hidden="true" className="size-3.5" />
      {t(`terminal.explain.risk.${risk}`)}
    </span>
  );
}

/** 官方文档按钮（文案走 i18n）。 */
function DocsButton({ docsUrl }: { readonly docsUrl: string }) {
  const { t } = useTranslation('shell');
  return (
    <IconButton
      label={t('terminal.explain.docs')}
      tooltip={t('terminal.explain.docs')}
      size="sm"
      onClick={() => void systemOpenUrl(docsUrl).catch(() => {})}
    >
      <BookOpen aria-hidden="true" className="size-3.5" />
    </IconButton>
  );
}

/** 卡片展示一个条目（命令级或命中的子级）。 */
function EntryBody({ entry }: { readonly entry: ExplainEntry }) {
  return (
    <>
      <div className="flex items-center justify-between gap-2">
        <RiskBadge risk={entry.risk} />
        {entry.docsUrl !== undefined ? <DocsButton docsUrl={entry.docsUrl} /> : null}
      </div>
      <p className="text-13 leading-relaxed">{entry.summary}</p>
    </>
  );
}

export function ExplainCard({ explain, repoId, onInsert, onClose }: ExplainCardProps) {
  const { t } = useTranslation('shell');
  // 子级命中时以子级为主解释（更精确），命令级作为上下文副标题
  const primary = explain.sub ?? explain.command;
  const route = primary.equivalentUiRoute ?? explain.command.equivalentUiRoute;
  const example = primary.example ?? explain.command.example;

  const openEquivalent = useCallback(() => {
    if (route !== undefined) {
      window.location.hash = `/repo/${repoId}${route}`;
    }
  }, [repoId, route]);

  return (
    <div
      role="dialog"
      aria-label={t('terminal.explain.title')}
      className="border-line bg-surface-raised absolute inset-x-3 top-3 z-30 rounded-md border p-3 shadow-lg"
    >
      <div className="flex items-start justify-between gap-2">
        <p className="text-fg-muted font-mono text-12">
          git {explain.commandName}
          {explain.sub ? ` ${explain.sub.name}` : ''}
        </p>
        <IconButton
          label={t('terminal.explain.close')}
          tooltip={t('terminal.explain.close')}
          size="sm"
          onClick={onClose}
        >
          <X aria-hidden="true" className="size-3.5" />
        </IconButton>
      </div>

      <EntryBody entry={primary} />

      <div className="mt-2 flex flex-wrap items-center gap-2">
        {route !== undefined ? (
          <button
            type="button"
            className="border-line bg-surface hover:bg-surface-sunken fd-transition rounded-md border px-2.5 py-1 text-12"
            onClick={openEquivalent}
          >
            {t('terminal.explain.openEquivalent')}
          </button>
        ) : null}
        {example !== undefined ? (
          <button
            type="button"
            className="border-line bg-surface hover:bg-surface-sunken fd-transition rounded-md border px-2.5 py-1 text-12"
            onClick={() => onInsert(example)}
          >
            {t('terminal.explain.insertExample')}
          </button>
        ) : null}
      </div>

      <p className="text-fg-subtle mt-2 text-11">
        {t('terminal.explain.snapshotNote')}{' '}
        <button
          type="button"
          className="text-brand hover:underline"
          onClick={() =>
            void systemOpenUrl(
              'https://github.com/Ember1414/forgedesk/issues/new?labels=git-explains',
            ).catch(() => {})
          }
        >
          {t('terminal.explain.feedback')}
        </button>
      </p>
    </div>
  );
}
