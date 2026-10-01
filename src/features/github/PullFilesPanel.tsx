/**
 * PR 变更文件面板（T4.7 收尾）：行级 diff + 行锚点行内评论。
 *
 * # 为什么不复用 M1 的 DiffView
 *
 * DiffView 面向本地工作区（按 repoId + staged/unstaged 查询，自带暂存/
 * 丢弃动作），数据获取长在组件里；这里的输入是远端 PR 的已解析 hunk
 * （后端把 GitHub 的 patch 按行展开），渲染与评论锚点是另一组交互，硬套
 * 只会把两个域的状态搅在一起。行的 kind/oldNo/newNo 与工作区 diff DTO
 * 同名同义，视觉沿用同一套语义 token（docs/PLAN.md M4.2 的"复用 M1 diff
 * 组件"落在数据形状与视觉规则上的复用）。
 *
 * # 行内评论的锚点
 *
 * 可评论行（context/added/removed）点击后打开输入框：added 行锚 RIGHT、
 * removed 行锚 LEFT、context 行缺省锚 RIGHT（新文件行号更常被引用）。
 * 行是否合法由后端在创建时再校验一次（M4 验收"行号越界有明确错误"的
 * 落点在后端）；前端展示的行全部来自后端解析的 hunk，正常必然合法。
 *
 * # 评论按行号回贴
 *
 * 后端返回的评论带 path/side/line：匹配到当前 diff 行的作为线程贴在行
 * 下方（回复按 inReplyTo 嵌套）；diff 已变化导致匹配不上的列入
 * "不在此 diff 上的评论"，不丢数据。正文是 Markdown 原文，与时间线评论
 * 同一规则用纯文本渲染。
 */
import { useEffect, useMemo, useState, type ReactElement } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import {
  repoPullFiles,
  repoPullReviewCommentCreate,
  repoPullReviewCommentReply,
  repoPullReviewCommentsList,
} from '@/lib/ipc';
import type { PullDiffLine, PullFile, PullReviewComment } from '@/lib/ipc';
import { pushToast } from '@/stores/toastStore';

import { Button } from '@/ui/components/button';
import { ErrorState } from '@/ui/components/error-state';

/** 与其他 GitHub 页一致的站点常量。 */
const HOST = 'github.com';

/** 行内评论的锚点。 */
interface LineAnchor {
  readonly path: string;
  readonly side: 'LEFT' | 'RIGHT';
  readonly line: number;
}

export interface PullFilesPanelProps {
  readonly owner: string;
  readonly repo: string;
  readonly number: number;
}

/** 一行可评论的锚点（context 行两个侧都可能被引用，返回两个）。 */
function rowAnchors(path: string, line: PullDiffLine): readonly LineAnchor[] {
  if (line.kind === 'added' && line.newNo != null) {
    return [{ path, side: 'RIGHT', line: line.newNo }];
  }
  if (line.kind === 'removed' && line.oldNo != null) {
    return [{ path, side: 'LEFT', line: line.oldNo }];
  }
  if (line.kind === 'context') {
    const anchors: LineAnchor[] = [];
    if (line.oldNo != null) {
      anchors.push({ path, side: 'LEFT', line: line.oldNo });
    }
    if (line.newNo != null) {
      anchors.push({ path, side: 'RIGHT', line: line.newNo });
    }
    return anchors;
  }
  return [];
}

/** 评论的锚点缺省取法：added→RIGHT、removed→LEFT、context→RIGHT。 */
function defaultAnchor(path: string, line: PullDiffLine): LineAnchor | null {
  if (line.kind === 'added' && line.newNo != null) {
    return { path, side: 'RIGHT', line: line.newNo };
  }
  if (line.kind === 'removed' && line.oldNo != null) {
    return { path, side: 'LEFT', line: line.oldNo };
  }
  if (line.kind === 'context' && line.newNo != null) {
    return { path, side: 'RIGHT', line: line.newNo };
  }
  return null;
}

function threadKey(path: string, side: string, line: number): string {
  return `${path}\u0000${side}\u0000${line}`;
}

function anchorKey(anchor: LineAnchor): string {
  return threadKey(anchor.path, anchor.side, anchor.line);
}

const KIND_CLASS: Readonly<Record<string, string>> = {
  context: 'bg-surface text-fg',
  added: 'bg-success/10 text-fg',
  removed: 'bg-danger/10 text-fg',
  noNewline: 'text-fg-subtle',
};

export function PullFilesPanel({ owner, repo, number }: PullFilesPanelProps) {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const [files, setFiles] = useState<readonly PullFile[] | null>(null);
  const [comments, setComments] = useState<readonly PullReviewComment[]>([]);
  const [failed, setFailed] = useState(false);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [composer, setComposer] = useState<LineAnchor | null>(null);
  const [composerText, setComposerText] = useState('');
  const [posting, setPosting] = useState(false);
  const [replyTo, setReplyTo] = useState<number | null>(null);
  const [replyText, setReplyText] = useState('');

  useEffect(() => {
    const cancelled = { value: false };
    void Promise.all([
      repoPullFiles({ host: HOST, owner, repo, number, perPage: 100 }),
      repoPullReviewCommentsList(HOST, owner, repo, number).catch(() => [] as PullReviewComment[]),
    ])
      .then(([filePage, commentList]) => {
        if (cancelled.value) {
          return;
        }
        setFiles(filePage.items);
        setComments(commentList);
        setFailed(false);
      })
      .catch((raw: unknown) => {
        if (!cancelled.value) {
          setFailed(true);
          show(raw);
        }
      });
    return () => {
      cancelled.value = true;
    };
  }, [owner, repo, number, show]);

  const threadRoots = useMemo(() => {
    const roots = new Map<string, PullReviewComment[]>();
    const replies = new Map<number, PullReviewComment[]>();
    for (const comment of comments) {
      if (comment.inReplyTo == null) {
        if (comment.path == null || comment.line == null) {
          continue;
        }
        const key = threadKey(
          comment.path,
          comment.side === 'LEFT' ? 'LEFT' : 'RIGHT',
          comment.line,
        );
        const bucket = roots.get(key);
        if (bucket === undefined) {
          roots.set(key, [comment]);
        } else {
          bucket.push(comment);
        }
      } else {
        const bucket = replies.get(comment.inReplyTo);
        if (bucket === undefined) {
          replies.set(comment.inReplyTo, [comment]);
        } else {
          bucket.push(comment);
        }
      }
    }
    return { roots, replies };
  }, [comments]);

  if (failed) {
    return <ErrorState title={t('github.prs.filesError')} />;
  }
  if (files === null) {
    return (
      <p className="text-13 text-fg-subtle" data-testid="pull-files-loading">
        {t('github.repos.loading')}
      </p>
    );
  }

  const toggleFile = (path: string): void => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  const postInlineComment = async (): Promise<void> => {
    if (composer === null || composerText.trim() === '') {
      return;
    }
    setPosting(true);
    try {
      const created = await repoPullReviewCommentCreate({
        host: HOST,
        owner,
        repo,
        number,
        path: composer.path,
        side: composer.side,
        line: composer.line,
        body: composerText,
      });
      setComments((current) => [...current, created]);
      setComposer(null);
      setComposerText('');
      pushToast({ tone: 'success', title: t('github.prs.inlinePostedToast') });
    } catch (raw) {
      show(raw);
    } finally {
      setPosting(false);
    }
  };

  const postReply = async (commentId: number): Promise<void> => {
    if (replyText.trim() === '') {
      return;
    }
    setPosting(true);
    try {
      const created = await repoPullReviewCommentReply({
        host: HOST,
        owner,
        repo,
        number,
        commentId,
        body: replyText,
      });
      setComments((current) => [...current, created]);
      setReplyTo(null);
      setReplyText('');
      pushToast({ tone: 'success', title: t('github.prs.replyPostedToast') });
    } catch (raw) {
      show(raw);
    } finally {
      setPosting(false);
    }
  };

  const renderThread = (comment: PullReviewComment): ReactElement => {
    const replies = threadRoots.replies.get(comment.id) ?? [];
    return (
      <li key={comment.id} className="rounded-sm border border-line bg-surface px-2 py-1">
        <p className="text-12">
          <span className="font-medium">{comment.author}</span>
          <span className="ml-2 text-fg-muted">{comment.body}</span>
        </p>
        {replies.length > 0 ? (
          <ul className="mt-1 flex flex-col gap-1 border-l border-line pl-2">
            {replies.map((reply) => (
              <li key={reply.id} className="text-12">
                <span className="font-medium">{reply.author}</span>
                <span className="ml-2 text-fg-muted">{reply.body}</span>
              </li>
            ))}
          </ul>
        ) : null}
        {replyTo === comment.id ? (
          <div className="mt-1 flex flex-col gap-1">
            <textarea
              value={replyText}
              onChange={(event) => setReplyText(event.target.value)}
              placeholder={t('github.prs.replyPlaceholder')}
              aria-label={t('github.prs.replyPlaceholder')}
              rows={2}
              className="fd-transition rounded-md border border-line bg-surface px-2.5 py-1.5 text-13 focus:border-brand"
              data-testid="pull-reply-input"
            />
            <div className="flex gap-2">
              <Button
                type="button"
                variant="secondary"
                disabled={posting || replyText.trim() === ''}
                onClick={() => void postReply(comment.id)}
                data-testid="pull-reply-post"
              >
                {t('github.prs.commentPost')}
              </Button>
              <Button type="button" variant="secondary" onClick={() => setReplyTo(null)}>
                {t('common:actions.cancel')}
              </Button>
            </div>
          </div>
        ) : (
          <button
            type="button"
            className="fd-transition mt-1 text-12 text-fg-muted hover:text-fg focus:text-fg"
            onClick={() => {
              setReplyText('');
              setReplyTo(comment.id);
            }}
            data-testid={`pull-reply-open-${comment.id}`}
          >
            {t('github.prs.reply')}
          </button>
        )}
      </li>
    );
  };

  const renderFile = (file: PullFile): ReactElement => {
    const isOpen = expanded.has(file.filename);
    // 当前文件里"会渲染在某行下"的顶层评论：剩下的列进"不在此 diff 上"
    const matchedKeys = new Set<string>(
      file.hunks.flatMap((hunk) =>
        hunk.lines.flatMap((line) => rowAnchors(file.filename, line).map(anchorKey)),
      ),
    );
    const orphans = comments.filter(
      (comment) =>
        comment.inReplyTo == null &&
        comment.path === file.filename &&
        (comment.line == null ||
          !matchedKeys.has(
            threadKey(file.filename, comment.side === 'LEFT' ? 'LEFT' : 'RIGHT', comment.line),
          )),
    );
    return (
      <div className="rounded-md border border-line" data-testid="pull-file">
        <button
          type="button"
          className="fd-transition flex w-full items-center gap-2 px-2 py-1.5 text-left text-13 hover:bg-surface focus:bg-surface"
          aria-expanded={isOpen}
          onClick={() => toggleFile(file.filename)}
          data-testid="pull-file-toggle"
        >
          <span className="font-mono">{file.filename}</span>
          {file.previousFilename != null && file.previousFilename !== '' ? (
            <span className="font-mono text-12 text-fg-subtle">← {file.previousFilename}</span>
          ) : null}
          <span className="text-12 text-fg-subtle">
            {t(`github.prs.fileStatus.${file.status}`, { defaultValue: file.status })}
          </span>
          <span className="ml-auto font-mono text-12">
            <span className="text-fg-muted">+{file.additions}</span>{' '}
            <span className="text-fg-subtle">−{file.deletions}</span>
          </span>
        </button>

        {isOpen ? (
          file.hunks.length === 0 ? (
            <p className="px-2 pb-2 text-12 text-fg-subtle" data-testid="pull-file-unavailable">
              {t('github.prs.fileDiffUnavailable')}
            </p>
          ) : (
            <div className="flex flex-col gap-2 px-2 pb-2" data-testid="pull-file-diff">
              {file.hunks.map((hunk, hunkIndex) => (
                <div key={hunkIndex} className="overflow-hidden rounded-sm border border-line">
                  <p className="bg-surface px-2 py-0.5 font-mono text-12 text-fg-subtle">
                    @@ -{hunk.oldStart},{hunk.oldLines} +{hunk.newStart},{hunk.newLines} @@{' '}
                    {hunk.header}
                  </p>
                  {hunk.lines.map((line, lineIndex) => {
                    const anchors = rowAnchors(file.filename, line);
                    const anchor = defaultAnchor(file.filename, line);
                    const threads =
                      anchor === null ? [] : (threadRoots.roots.get(anchorKey(anchor)) ?? []);
                    return (
                      <div key={lineIndex}>
                        {anchors.length > 0 ? (
                          <button
                            type="button"
                            className={`fd-transition grid w-full grid-cols-[3rem_3rem_1fr] text-left font-mono text-12 leading-6 hover:outline hover:outline-brand ${
                              KIND_CLASS[line.kind] ?? 'text-fg'
                            }`}
                            aria-label={t('github.prs.addLineComment', {
                              path: file.filename,
                              side:
                                (anchor?.side ?? 'RIGHT') === 'LEFT'
                                  ? t('github.prs.sideOld')
                                  : t('github.prs.sideNew'),
                              line: anchor?.line ?? 0,
                            })}
                            onClick={() => {
                              // 锚点取本行的缺省侧（added→RIGHT / removed→LEFT /
                              // context→RIGHT），与 composer 的渲染条件同一基准
                              setComposer(anchor);
                              setComposerText('');
                            }}
                            data-testid="pull-line"
                          >
                            <span className="px-1 text-right text-fg-subtle">
                              {line.oldNo ?? ''}
                            </span>
                            <span className="px-1 text-right text-fg-subtle">
                              {line.newNo ?? ''}
                            </span>
                            <span className="whitespace-pre px-2">{line.content}</span>
                          </button>
                        ) : (
                          <div
                            className={`grid grid-cols-[3rem_3rem_1fr] font-mono text-12 leading-6 ${
                              KIND_CLASS[line.kind] ?? 'text-fg'
                            }`}
                          >
                            <span className="px-1 text-right text-fg-subtle">
                              {line.oldNo ?? ''}
                            </span>
                            <span className="px-1 text-right text-fg-subtle">
                              {line.newNo ?? ''}
                            </span>
                            <span className="whitespace-pre px-2">{line.content}</span>
                          </div>
                        )}

                        {threads.length > 0 ? (
                          <ul className="m-1 flex flex-col gap-1" data-testid="pull-inline-thread">
                            {threads.map(renderThread)}
                          </ul>
                        ) : null}

                        {anchor !== null &&
                        composer !== null &&
                        anchorKey(composer) === anchorKey(anchor) ? (
                          <div
                            className="m-1 rounded-sm border border-line bg-surface p-2"
                            data-testid="pull-inline-composer"
                          >
                            <p className="text-12 text-fg-subtle">
                              {t('github.prs.inlineAnchor', {
                                path: composer.path,
                                side:
                                  composer.side === 'LEFT'
                                    ? t('github.prs.sideOld')
                                    : t('github.prs.sideNew'),
                                line: composer.line,
                              })}
                            </p>
                            <textarea
                              value={composerText}
                              onChange={(event) => setComposerText(event.target.value)}
                              placeholder={t('github.prs.inlinePlaceholder')}
                              aria-label={t('github.prs.inlinePlaceholder')}
                              rows={2}
                              className="fd-transition mt-1 rounded-md border border-line bg-surface px-2.5 py-1.5 text-13 focus:border-brand"
                              data-testid="pull-inline-input"
                            />
                            <div className="mt-1 flex gap-2">
                              <Button
                                type="button"
                                variant="secondary"
                                disabled={posting || composerText.trim() === ''}
                                onClick={() => void postInlineComment()}
                                data-testid="pull-inline-post"
                              >
                                {t('github.prs.commentPost')}
                              </Button>
                              <Button
                                type="button"
                                variant="secondary"
                                onClick={() => setComposer(null)}
                              >
                                {t('common:actions.cancel')}
                              </Button>
                            </div>
                          </div>
                        ) : null}
                      </div>
                    );
                  })}
                </div>
              ))}
            </div>
          )
        ) : null}

        {isOpen && orphans.length > 0 ? (
          <div className="border-t border-line px-2 py-1.5" data-testid="pull-orphan-comments">
            <p className="text-12 text-fg-subtle">{t('github.prs.outdatedComments')}</p>
            <ul className="mt-1 flex flex-col gap-1">
              {orphans.map((comment) => (
                <li key={comment.id} className="text-12">
                  <span className="font-medium">{comment.author}</span>
                  {comment.line != null ? (
                    <span className="ml-1 font-mono text-fg-subtle">:{comment.line}</span>
                  ) : null}
                  <span className="ml-2 text-fg-muted">{comment.body}</span>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </div>
    );
  };

  return (
    <div className="flex flex-col gap-2" data-testid="pull-files">
      <p className="text-13 font-medium">{t('github.prs.filesTitle')}</p>
      {files.length === 0 ? (
        <p className="text-12 text-fg-subtle">{t('github.prs.filesEmpty')}</p>
      ) : (
        files.map(renderFile)
      )}
    </div>
  );
}
