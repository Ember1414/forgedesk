/**
 * 提交前的计划预览（T1.7）。
 *
 * 这是红线 R7「计划预览 → 快照 → 执行 → 可回滚」在界面上的那一屏：用户在这里看到的
 * 文件清单、等价命令与钩子列表，全部来自后端的**同一份计划**（`commit_prepare` 的返回值），
 * 点击确认后执行的也是它——不存在"界面显示一份、实际执行另一份"的可能。
 *
 * # 为什么用 Dialog 而不是 AlertDialog
 *
 * AlertDialog 的语义是"你已经决定了，确认一下后果"（它的 `impact` 是必填项，且焦点
 * 默认落在取消上）。这里要做的是让用户**阅读**（命令、钩子、文件清单）之后再决定，
 * 属于"计划预览"。两者混用会让用户对"哪个框是危险操作"失去判断。
 *
 * # 关于快照的诚实提示
 *
 * M3 之前没有快照，界面上如实写明"本次不会创建快照"。假装有安全网比没有安全网更危险。
 */
import { useState } from 'react';

import { Copy } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { NormalizedError } from '@/lib/errors';
import type { CommitPlan, PlannedFile } from '@/lib/ipc/commit';
import { Button } from '@/ui/components/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';
import { ErrorState } from '@/ui/components/error-state';

/** 索引状态字符 → i18n 文案（复用工作区面板的同一批文案，避免同义两译）。 */
const STATUS_LABEL_KEYS: Readonly<Record<string, string>> = {
  A: 'workspace.status.added',
  M: 'workspace.status.modified',
  D: 'workspace.status.deleted',
  R: 'workspace.status.renamed',
  C: 'workspace.status.copied',
  T: 'workspace.status.typeChanged',
  U: 'workspace.status.conflict',
};

/** 每组最多列出的文件数；超出只报数量（几十行的清单没人会读）。 */
const MAX_FILES_PER_GROUP = 50;

export interface CommitPreviewDialogProps {
  readonly open: boolean;
  /** 待确认的计划；为 `null` 时不渲染内容。 */
  readonly plan: CommitPlan | null;
  /** 正在执行（按钮进入 loading 并禁用重复提交）。 */
  readonly busy: boolean;
  /** 上一次执行的失败（例如钩子拒绝），展示在对话框里而不是只弹一个提示。 */
  readonly failure: NormalizedError | null;
  readonly onOpenChange: (open: boolean) => void;
  readonly onConfirm: () => void;
}

/** 按索引状态分组（顺序稳定，便于核对）。 */
function groupFiles(files: readonly PlannedFile[]): readonly (readonly [string, string[]])[] {
  const groups = new Map<string, string[]>();
  for (const file of files) {
    const key = file.indexStatus.toUpperCase();
    const bucket = groups.get(key) ?? [];
    bucket.push(file.path);
    groups.set(key, bucket);
  }
  return [...groups.entries()].sort(([left], [right]) => left.localeCompare(right));
}

export function CommitPreviewDialog({
  open,
  plan,
  busy,
  failure,
  onOpenChange,
  onConfirm,
}: CommitPreviewDialogProps) {
  const { t } = useTranslation('shell');
  const [copied, setCopied] = useState(false);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        closeLabel={t('commit.cancel')}
        className="max-h-[85vh] overflow-y-auto"
        // Ctrl/Cmd+Enter 在对话框里也能确认：用户的肌肉记忆不该因为"弹了个框"而失效
        onKeyDown={(event) => {
          if ((event.ctrlKey || event.metaKey) && event.key === 'Enter' && !busy) {
            event.preventDefault();
            onConfirm();
          }
        }}
      >
        <DialogHeader>
          <DialogTitle>{t('commit.previewTitle')}</DialogTitle>
          <DialogDescription>{t('commit.previewDescription')}</DialogDescription>
        </DialogHeader>

        {plan !== null ? (
          <div className="mt-4 flex flex-col gap-4">
            <section className="flex flex-col gap-1.5">
              <h3 className="text-12 font-medium text-fg-muted">{t('commit.messageLabel')}</h3>
              <p className="text-14 font-medium text-fg">{plan.subject}</p>
              {plan.description !== null && plan.description !== '' ? (
                <pre className="max-h-40 overflow-y-auto whitespace-pre-wrap break-words rounded-md border border-line bg-surface-sunken px-3 py-2 font-mono text-12 text-fg-muted">
                  {plan.description}
                </pre>
              ) : null}
            </section>

            <section className="flex flex-col gap-1.5">
              <h3 className="text-12 font-medium text-fg-muted">
                {t('commit.previewFiles', { count: plan.files.length })}
              </h3>
              {plan.files.length === 0 ? (
                <p className="text-12 text-fg-subtle">{t('commit.previewFilesEmpty')}</p>
              ) : (
                <div className="flex flex-col gap-2">
                  {groupFiles(plan.files).map(([status, paths]) => (
                    <div key={status} className="flex flex-col gap-1">
                      <span className="text-11 text-fg-subtle">
                        {t(STATUS_LABEL_KEYS[status] ?? 'workspace.status.unknown')} ·{' '}
                        {paths.length}
                      </span>
                      <ul className="flex flex-col gap-0.5">
                        {paths.slice(0, MAX_FILES_PER_GROUP).map((path) => (
                          <li key={path} className="truncate font-mono text-12 text-fg-muted">
                            {path}
                          </li>
                        ))}
                        {paths.length > MAX_FILES_PER_GROUP ? (
                          <li className="text-11 text-fg-subtle">
                            +{paths.length - MAX_FILES_PER_GROUP}
                          </li>
                        ) : null}
                      </ul>
                    </div>
                  ))}
                </div>
              )}
            </section>

            <section className="flex flex-col gap-1.5">
              <div className="flex items-center gap-2">
                <h3 className="text-12 font-medium text-fg-muted">{t('commit.previewCommand')}</h3>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    void navigator.clipboard.writeText(plan.equivalentCommand).then(() => {
                      setCopied(true);
                    });
                  }}
                >
                  <Copy aria-hidden="true" className="size-3.5" />
                  {copied ? t('commit.copied') : t('commit.copyCommand')}
                </Button>
              </div>
              <pre className="overflow-x-auto whitespace-pre-wrap break-all rounded-md border border-line bg-surface-sunken px-3 py-2 font-mono text-12 text-fg-muted">
                {plan.equivalentCommand}
              </pre>
            </section>

            <section className="flex flex-col gap-1.5">
              <h3 className="text-12 font-medium text-fg-muted">{t('commit.previewHooks')}</h3>
              {plan.hooks.length === 0 ? (
                <p className="text-12 text-fg-subtle">{t('commit.previewHooksEmpty')}</p>
              ) : (
                <ul className="flex flex-wrap gap-1.5">
                  {plan.hooks.map((hook) => (
                    <li
                      key={hook}
                      className="rounded-sm bg-surface-sunken px-1.5 py-0.5 font-mono text-11 text-fg-muted"
                    >
                      {hook}
                    </li>
                  ))}
                </ul>
              )}
            </section>

            {plan.author !== null ? (
              <section className="flex flex-col gap-1">
                <h3 className="text-12 font-medium text-fg-muted">{t('commit.previewAuthor')}</h3>
                <p className="font-mono text-12 text-fg-muted">
                  {plan.author.name} &lt;{plan.author.email}&gt;
                </p>
              </section>
            ) : null}

            {plan.amend ? (
              <p className="rounded-md border border-warning bg-surface px-3 py-2 text-12 text-warning">
                {t('commit.previewAmend')}
              </p>
            ) : null}

            <p className="rounded-md border border-line bg-surface-sunken px-3 py-2 text-12 text-fg-subtle">
              {t('commit.previewSnapshot')}
            </p>

            {failure !== null && plan !== null ? (
              <ErrorState
                title={t(`errors:${failure.code}.title`)}
                {...(failure.hint === undefined
                  ? { hint: t(`errors:${failure.code}.hint`) }
                  : { hint: failure.hint })}
                {...(failure.detail === undefined ? {} : { details: failure.detail })}
              />
            ) : null}
          </div>
        ) : null}

        <DialogFooter>
          <Button variant="secondary" onClick={() => onOpenChange(false)} disabled={busy}>
            {t('commit.cancel')}
          </Button>
          <Button loading={busy} onClick={onConfirm}>
            {busy ? t('commit.submitting') : t('commit.confirm')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
