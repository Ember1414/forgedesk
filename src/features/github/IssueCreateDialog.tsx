/**
 * 新建 Issue 对话框（T4.8 UI）：标题必填 + 描述（Markdown）可选。
 *
 * 创建成功返回消毒后的详情并回调 `onCreated(number)`——调用方据此刷新
 * 列表；本组件不持有列表状态。
 */
import { useState } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import { repoIssueCreate } from '@/lib/ipc';
import { pushToast } from '@/stores/toastStore';

import { Button } from '@/ui/components/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';

/** 与其他 GitHub 页一致的站点常量。 */
const HOST = 'github.com';

export interface IssueCreateDialogProps {
  /** `null` 表示关闭。 */
  readonly target: { readonly owner: string; readonly repo: string } | null;
  readonly onOpenChange: (open: boolean) => void;
  /** 创建成功（参数是新的 Issue 编号）。 */
  readonly onCreated: (number: number) => void;
}

export function IssueCreateDialog({ target, onOpenChange, onCreated }: IssueCreateDialogProps) {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [creating, setCreating] = useState(false);

  const create = async () => {
    if (target === null || title.trim() === '') {
      return;
    }
    setCreating(true);
    try {
      const created = await repoIssueCreate({
        host: HOST,
        owner: target.owner,
        repo: target.repo,
        title,
        ...(body.trim() === '' ? {} : { body }),
      });
      pushToast({ tone: 'success', title: t('github.issues.createdToast') });
      setTitle('');
      setBody('');
      onOpenChange(false);
      onCreated(created.number);
    } catch (raw) {
      show(raw);
    } finally {
      setCreating(false);
    }
  };

  return (
    <Dialog open={target !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg" closeLabel={t('common:actions.close')}>
        <DialogHeader>
          <DialogTitle>{t('github.issues.createTitle')}</DialogTitle>
          <DialogDescription>{target ? `${target.owner}/${target.repo}` : ''}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-2" data-testid="issue-create-form">
          <input
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            placeholder={t('github.issues.titleLabel')}
            aria-label={t('github.issues.titleLabel')}
            className="fd-transition rounded-md border border-line bg-surface px-2.5 py-1.5 text-13 focus:border-brand"
            data-testid="issue-create-title"
          />
          <textarea
            value={body}
            onChange={(event) => setBody(event.target.value)}
            placeholder={t('github.issues.bodyLabel')}
            aria-label={t('github.issues.bodyLabel')}
            rows={5}
            className="fd-transition rounded-md border border-line bg-surface px-2.5 py-1.5 text-13 focus:border-brand"
            data-testid="issue-create-body"
          />
        </div>
        <div className="flex justify-end gap-2">
          <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
            {t('common:actions.cancel')}
          </Button>
          <Button
            type="button"
            disabled={creating || title.trim() === ''}
            onClick={() => void create()}
            data-testid="issue-create-submit"
          >
            {t('github.issues.create')}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
