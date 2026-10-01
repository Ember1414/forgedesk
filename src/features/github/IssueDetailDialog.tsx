/**
 * Issue 详情对话框（T4.8 UI）：评论 + 编辑 + 关开 + 指派。
 *
 * # 描述 HTML 已消毒
 *
 * `bodyHtml` 来自后端白名单渲染（与 README/PR 描述同一规则），这里不再
 * 二次解析；链接点击委托为复制（无 opener 插件，与全应用一致）。
 *
 * # 评论是纯文本渲染
 *
 * 评论正文是 Markdown 原文（来自 API），React 文本节点展示（默认转义），
 * 与 PR 时间线评论同一规则。
 *
 * # 动作后本地更新
 *
 * 关开/编辑/指派的命令都返回**消毒后的最新详情**，直接替换本地状态，
 * 不重新拉取（少一次请求，UI 也不会闪回加载态）；评论列表只在发表后
 * 本地追加。列表页的刷新由 `onChanged` 通知（open 视图里被关闭的
 * Issue 要消失）。
 *
 * # 指派面板按需加载
 *
 * 可指派人列表是独立的端点；只在用户展开指派面板时拉取一次，
 * 不给打开详情这件事增加固定成本。
 */
import { useEffect, useState } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import {
  repoIssueAssignees,
  repoIssueAssigneesSet,
  repoIssueBody,
  repoIssueCommentCreate,
  repoIssueCommentsList,
  repoIssueEdit,
  repoIssueGet,
  repoIssueStateSet,
} from '@/lib/ipc';
import type { Assignee, IssueComment, IssueDetail } from '@/lib/ipc';
import { pushToast } from '@/stores/toastStore';

import { Button } from '@/ui/components/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';
import { ErrorState } from '@/ui/components/error-state';

/** 与其他 GitHub 页一致的站点常量。 */
const HOST = 'github.com';

/** 详情对话框的目标（仓库 + Issue 号）。 */
export interface IssueTarget {
  readonly owner: string;
  readonly repo: string;
  readonly number: number;
}

interface IssueDetailDialogProps {
  /** 目标；`null` 表示关闭。 */
  readonly target: IssueTarget | null;
  readonly onOpenChange: (open: boolean) => void;
  /** Issue 状态变化后回调（列表刷新用）。 */
  readonly onChanged: () => void;
}

export function IssueDetailDialog({ target, onOpenChange, onChanged }: IssueDetailDialogProps) {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const [detail, setDetail] = useState<IssueDetail | null>(null);
  const [comments, setComments] = useState<readonly IssueComment[]>([]);
  const [commentInput, setCommentInput] = useState('');
  const [postingComment, setPostingComment] = useState(false);
  const [failed, setFailed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editTitle, setEditTitle] = useState('');
  const [editBody, setEditBody] = useState('');
  /** 编辑器打开时的描述原文（Markdown）：未改动的比对基准。 */
  const [editBaseBody, setEditBaseBody] = useState('');
  const [savingEdit, setSavingEdit] = useState(false);
  const [assignOpen, setAssignOpen] = useState(false);
  const [assignees, setAssignees] = useState<readonly Assignee[]>([]);
  const [assignLoaded, setAssignLoaded] = useState(false);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [savingAssign, setSavingAssign] = useState(false);

  useEffect(() => {
    if (target === null) {
      void Promise.resolve().then(() => {
        setDetail(null);
        setComments([]);
        setFailed(false);
        setEditing(false);
        setAssignOpen(false);
        setAssignLoaded(false);
      });
      return;
    }
    const cancelled = { value: false };
    void Promise.resolve()
      .then(() => {
        setLoading(true);
        setFailed(false);
        return Promise.all([
          repoIssueGet(HOST, target.owner, target.repo, target.number),
          repoIssueCommentsList(HOST, target.owner, target.repo, target.number).catch(
            () => [] as IssueComment[],
          ),
        ]);
      })
      .then(([issue, commentList]) => {
        if (cancelled.value) {
          return;
        }
        setDetail(issue);
        setComments(commentList);
      })
      .catch((raw: unknown) => {
        if (!cancelled.value) {
          setFailed(true);
          show(raw);
        }
      })
      .finally(() => {
        if (!cancelled.value) {
          setLoading(false);
        }
      });
    return () => {
      cancelled.value = true;
    };
  }, [target, show]);

  const openAssignees = () => {
    setAssignOpen(!assignOpen);
    if (!assignLoaded && detail !== null) {
      setAssignLoaded(true);
      void repoIssueAssignees(HOST, target?.owner ?? '', target?.repo ?? '')
        .then((list) => setAssignees(list))
        .catch((raw: unknown) => show(raw));
    }
  };

  const postComment = async () => {
    if (detail === null || target === null || commentInput.trim() === '') {
      return;
    }
    setPostingComment(true);
    try {
      const created = await repoIssueCommentCreate(
        HOST,
        target.owner,
        target.repo,
        detail.number,
        commentInput,
      );
      setComments((current) => [...current, created]);
      setCommentInput('');
      pushToast({ tone: 'success', title: t('github.prs.commentPostedToast') });
    } catch (raw) {
      show(raw);
    } finally {
      setPostingComment(false);
    }
  };

  const toggleState = async () => {
    if (detail === null || target === null) {
      return;
    }
    try {
      const updated = await repoIssueStateSet({
        host: HOST,
        owner: target.owner,
        repo: target.repo,
        number: detail.number,
        open: detail.state !== 'open',
      });
      setDetail(updated);
      onChanged();
      pushToast({
        tone: 'success',
        title:
          updated.state === 'open'
            ? t('github.issues.reopenedToast')
            : t('github.issues.closedToast'),
      });
    } catch (raw) {
      show(raw);
    }
  };

  const saveEdit = async () => {
    if (detail === null || target === null) {
      return;
    }
    const title = editTitle.trim() === '' || editTitle === detail.title ? undefined : editTitle;
    const body = editBody === editBaseBody ? undefined : editBody;
    if (title === undefined && body === undefined) {
      setEditing(false);
      return;
    }
    setSavingEdit(true);
    try {
      const updated = await repoIssueEdit({
        host: HOST,
        owner: target.owner,
        repo: target.repo,
        number: detail.number,
        ...(title === undefined ? {} : { title }),
        ...(body === undefined ? {} : { body }),
      });
      setDetail(updated);
      setEditing(false);
      onChanged();
      pushToast({ tone: 'success', title: t('github.issues.editedToast') });
    } catch (raw) {
      show(raw);
    } finally {
      setSavingEdit(false);
    }
  };

  const saveAssignees = async () => {
    if (detail === null || target === null) {
      return;
    }
    setSavingAssign(true);
    try {
      const updated = await repoIssueAssigneesSet({
        host: HOST,
        owner: target.owner,
        repo: target.repo,
        number: detail.number,
        assignees: [...selected],
      });
      setDetail(updated);
      pushToast({ tone: 'success', title: t('github.issues.assignedToast') });
    } catch (raw) {
      show(raw);
    } finally {
      setSavingAssign(false);
    }
  };

  const toggleAssignee = (login: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(login)) {
        next.delete(login);
      } else {
        next.add(login);
      }
      return next;
    });
  };

  return (
    <Dialog open={target !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl" closeLabel={t('common:actions.close')}>
        <DialogHeader>
          <DialogTitle>
            {t('github.issues.detailTitle', {
              number: target?.number ?? 0,
              title: detail?.title ?? '',
            })}
          </DialogTitle>
          <DialogDescription>{target ? `${target.owner}/${target.repo}` : ''}</DialogDescription>
        </DialogHeader>

        {loading ? (
          <p className="text-13 text-fg-subtle" data-testid="issue-loading">
            {t('github.repos.loading')}
          </p>
        ) : null}

        {failed && !loading ? <ErrorState title={t('github.repos.listErrorHint')} /> : null}

        {detail !== null && !loading ? (
          <div
            className="flex max-h-[60vh] flex-col gap-3 overflow-auto"
            data-testid="issue-detail"
          >
            <div className="flex flex-wrap items-center gap-2 text-12 text-fg-subtle">
              <span
                className="rounded-sm border border-line px-1.5 py-0.5"
                data-testid="issue-state-badge"
              >
                {t(`github.issues.stateTab.${detail.state === 'open' ? 'open' : 'closed'}`)}
              </span>
              <span>{t('github.prs.author', { author: detail.author })}</span>
              <span data-testid="issue-assignees">
                {detail.assignees.length === 0
                  ? t('github.issues.unassigned')
                  : detail.assignees.join(', ')}
              </span>
            </div>

            {detail.labels.length > 0 ? (
              <div className="flex flex-wrap gap-1" data-testid="issue-labels">
                {detail.labels.map((label) => (
                  <span
                    key={label}
                    className="rounded-sm border border-line px-1.5 py-0.5 text-11 text-fg-muted"
                  >
                    {label}
                  </span>
                ))}
              </div>
            ) : null}

            {editing ? (
              <div className="flex flex-col gap-2" data-testid="issue-edit-form">
                <p className="text-13 font-medium">{t('github.issues.editTitle')}</p>
                <input
                  value={editTitle}
                  onChange={(event) => setEditTitle(event.target.value)}
                  aria-label={t('github.issues.titleLabel')}
                  className="fd-transition rounded-md border border-line bg-surface px-2.5 py-1.5 text-13 focus:border-brand"
                  data-testid="issue-edit-title"
                />
                <textarea
                  value={editBody}
                  onChange={(event) => setEditBody(event.target.value)}
                  placeholder={t('github.issues.bodyLabel')}
                  aria-label={t('github.issues.bodyLabel')}
                  rows={4}
                  className="fd-transition rounded-md border border-line bg-surface px-2.5 py-1.5 text-13 focus:border-brand"
                  data-testid="issue-edit-body"
                />
                <div className="flex gap-2">
                  <Button
                    type="button"
                    variant="secondary"
                    disabled={savingEdit}
                    onClick={() => void saveEdit()}
                    data-testid="issue-edit-save"
                  >
                    {t('github.issues.editSave')}
                  </Button>
                  <Button type="button" variant="secondary" onClick={() => setEditing(false)}>
                    {t('common:actions.cancel')}
                  </Button>
                </div>
              </div>
            ) : (
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => {
                    if (target === null) {
                      return;
                    }
                    setEditTitle(detail.title);
                    setEditBody('');
                    setEditBaseBody('');
                    setEditing(true);
                    // 编辑器预填的是描述**原文**（Markdown 进 textarea 是
                    // 惰性文本，与评论正文同一边界判断），展示仍走消毒 HTML
                    void repoIssueBody(HOST, target.owner, target.repo, detail.number)
                      .then((raw) => {
                        setEditBody(raw);
                        setEditBaseBody(raw);
                      })
                      .catch(() => undefined);
                  }}
                  data-testid="issue-edit-open"
                >
                  {t('github.issues.editOpen')}
                </Button>
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => void toggleState()}
                  data-testid="issue-state-toggle"
                >
                  {detail.state === 'open'
                    ? t('github.issues.closeIssue')
                    : t('github.issues.reopenIssue')}
                </Button>
              </div>
            )}

            {detail.bodyHtml !== undefined ? (
              <div
                className="max-h-48 overflow-auto rounded-md border border-line bg-surface p-3 text-13 leading-relaxed"
                onClick={(event) => {
                  const anchor = (event.target as HTMLElement).closest('a');
                  if (anchor !== null) {
                    event.preventDefault();
                    const href = anchor.getAttribute('href') ?? '';
                    if (href !== '') {
                      void navigator.clipboard?.writeText(href).catch(() => undefined);
                      pushToast({ tone: 'info', title: t('github.repos.linkCopiedToast') });
                    }
                  }
                }}
                data-testid="issue-body"
                dangerouslySetInnerHTML={{ __html: detail.bodyHtml }}
              />
            ) : null}

            <div className="flex flex-col gap-2" data-testid="issue-assign-panel">
              <Button
                type="button"
                variant="secondary"
                aria-expanded={assignOpen}
                onClick={openAssignees}
                data-testid="issue-assign-open"
              >
                {t('github.issues.assignOpen')}
              </Button>
              {assignOpen ? (
                <div className="flex flex-col gap-2">
                  <div className="flex flex-wrap gap-2" data-testid="issue-assign-list">
                    {assignees.length === 0 ? (
                      <span className="text-12 text-fg-subtle">
                        {assignLoaded ? t('github.issues.unassigned') : t('github.repos.loading')}
                      </span>
                    ) : (
                      assignees.map((assignee) => (
                        <label key={assignee.login} className="flex items-center gap-1.5 text-12">
                          <input
                            type="checkbox"
                            checked={selected.has(assignee.login)}
                            onChange={() => toggleAssignee(assignee.login)}
                            data-testid={`issue-assign-${assignee.login}`}
                          />
                          {assignee.login}
                        </label>
                      ))
                    )}
                  </div>
                  <Button
                    type="button"
                    variant="secondary"
                    disabled={savingAssign}
                    onClick={() => void saveAssignees()}
                    data-testid="issue-assign-save"
                  >
                    {t('github.issues.assignSave')}
                  </Button>
                </div>
              ) : null}
            </div>

            <div className="flex flex-col gap-2" data-testid="issue-comments">
              <p className="text-13 font-medium">{t('github.prs.commentsTitle')}</p>
              <ul className="flex flex-col gap-1 text-12" data-testid="issue-comment-list">
                {comments.length === 0 ? (
                  <li className="text-fg-subtle">{t('github.prs.noComments')}</li>
                ) : (
                  comments.map((comment) => (
                    <li key={comment.id} className="rounded-sm border border-line px-2 py-1">
                      <span className="font-medium">{comment.author}</span>
                      <span className="ml-2 text-fg-muted">{comment.body}</span>
                    </li>
                  ))
                )}
              </ul>
              <div className="flex flex-col gap-1">
                <textarea
                  value={commentInput}
                  onChange={(event) => setCommentInput(event.target.value)}
                  placeholder={t('github.prs.commentPlaceholder')}
                  aria-label={t('github.prs.commentPlaceholder')}
                  rows={2}
                  className="fd-transition rounded-md border border-line bg-surface px-2.5 py-1.5 text-13 focus:border-brand"
                  data-testid="issue-comment-input"
                />
                <Button
                  type="button"
                  variant="secondary"
                  disabled={postingComment || commentInput.trim() === ''}
                  onClick={() => void postComment()}
                  data-testid="issue-comment-post"
                >
                  {t('github.prs.commentPost')}
                </Button>
              </div>
            </div>
          </div>
        ) : null}

        <div className="flex justify-end gap-2">
          <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
            {t('common:actions.close')}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
