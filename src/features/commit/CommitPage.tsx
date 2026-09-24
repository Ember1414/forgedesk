/**
 * 提交面板（M1 / T1.7）：写信息 → 预览 → 提交。
 *
 * # 为什么按钮上要写"预览并提交"而不是"提交"
 *
 * 因为它确实分两步：点击后先生成计划（`commit_prepare`），把文件清单、等价命令与
 * 钩子列表摆出来，用户在预览里确认后才真正写仓库。把两步的东西说成一步，
 * 用户就不会去看那一屏——而那一屏正是"提交了什么"的唯一可信来源。
 *
 * # 状态归属
 *
 * 表单内容、计划、预览开关都是**组件局部状态**（只有这个页面用），不入 store；
 * 服务端状态（工作区状态、风格提示）走 TanStack Query（AGENTS.md §6：
 * Git 状态不进 Zustand）。
 *
 * # 禁用与原因
 *
 * 没有暂存内容且不是 amend 时按钮禁用，并在旁边写明原因与出口（去工作区暂存）。
 * 只禁用一个按钮而不解释，是"界面看起来坏了"的经典来源。
 */
import { useEffect, useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Link, useParams } from 'react-router-dom';

import { CommitPreviewDialog } from '@/features/commit/CommitPreviewDialog';
import { countsOf, groupsOf } from '@/features/workspace/statusModel';
import { STATUS_QUERY_KEY } from '@/features/workspace/WorkspaceStatusPage';
import { normalizeError, useAppError } from '@/lib/errors';
import type { NormalizedError } from '@/lib/errors';
import { isTauriRuntime } from '@/lib/ipc/client';
import { commitExecute, commitMessageHint, commitPrepare } from '@/lib/ipc/commit';
import type { CommitPlan, CommitSignMode } from '@/lib/ipc/commit';
import { onRepoChanged, workspaceStatus } from '@/lib/ipc/workspace';
import { pushToast } from '@/stores/toastStore';
import { Button } from '@/ui/components/button';
import { Checkbox } from '@/ui/components/checkbox';
import { Input } from '@/ui/components/input';
import { SelectField } from '@/ui/components/select';
import { Skeleton } from '@/ui/components/skeleton';
import { Textarea } from '@/ui/components/textarea';

/**
 * 首行的建议长度。
 *
 * 与后端 `domain::git::commit_plan::SUBJECT_RECOMMENDED_MAX_CHARS` 保持一致——
 * 后端只在**建议**里用它（不阻断），前端据此显示计数器。两处都改才算改完。
 */
const SUBJECT_RECOMMENDED_MAX_CHARS = 72;

/** 最近提交下拉里展示的条数。 */
const RECENT_LIMIT = 5;

export function CommitPage() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const queryClient = useQueryClient();
  const { show } = useAppError();

  const [subject, setSubject] = useState('');
  const [description, setDescription] = useState('');
  const [amend, setAmend] = useState(false);
  const [signOff, setSignOff] = useState(false);
  const [noVerify, setNoVerify] = useState(false);
  const [sign, setSign] = useState<CommitSignMode>('auto');
  const [authorName, setAuthorName] = useState('');
  const [authorEmail, setAuthorEmail] = useState('');
  const [selectedRecent, setSelectedRecent] = useState<string | undefined>(undefined);
  const [plan, setPlan] = useState<CommitPlan | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [failure, setFailure] = useState<NormalizedError | null>(null);

  const statusQuery = useQuery({
    queryKey: [STATUS_QUERY_KEY, repoId, false],
    queryFn: () => workspaceStatus(repoId, false),
    enabled: Number.isFinite(repoId),
  });

  const hintQuery = useQuery({
    queryKey: ['commit-hint', repoId],
    queryFn: () => commitMessageHint(repoId),
    enabled: Number.isFinite(repoId),
  });

  // 暂存 / 提交等操作会发布 `repo:changed`：本页的"已暂存 N 个文件"与提示都要跟着变
  useEffect(() => {
    if (!isTauriRuntime() || !Number.isFinite(repoId)) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void onRepoChanged((payload) => {
      if (payload.repoId !== repoId) return;
      void queryClient.invalidateQueries({ queryKey: [STATUS_QUERY_KEY, repoId] });
      void queryClient.invalidateQueries({ queryKey: ['commit-hint', repoId] });
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [queryClient, repoId]);

  const stagedCount = statusQuery.data ? countsOf(groupsOf(statusQuery.data)).staged : 0;
  const subjectChars = subject.trim().length;
  // amend 可以只改信息，因此"没有暂存内容"在 amend 模式下不构成阻断
  const canSubmit = subject.trim() !== '' && (stagedCount > 0 || amend);

  const prepareMutation = useMutation({
    mutationFn: () => {
      const body = description.trim();
      const name = authorName.trim();
      const email = authorEmail.trim();
      const author = name !== '' && email !== '' ? { name, email } : undefined;
      return commitPrepare(repoId, {
        message: subject,
        amend,
        signOff,
        noVerify,
        sign,
        // exactOptionalPropertyTypes：可选字段按存在性展开，不传 undefined
        ...(body === '' ? {} : { description: body }),
        ...(author === undefined ? {} : { author }),
      });
    },
    onSuccess: (next) => {
      setPlan(next);
      setFailure(null);
      setPreviewOpen(true);
    },
    onError: (error) => {
      show(normalizeError(error));
    },
  });

  const executeMutation = useMutation({
    mutationFn: (planId: string) => commitExecute(planId),
    onSuccess: (outcome) => {
      pushToast({
        tone: 'success',
        title: t('commit.success', { subject: outcome.subject }),
      });
      setPreviewOpen(false);
      setPlan(null);
      setFailure(null);
      setSubject('');
      setDescription('');
      setAmend(false);
      setSelectedRecent(undefined);
      void queryClient.invalidateQueries({ queryKey: [STATUS_QUERY_KEY, repoId] });
      void queryClient.invalidateQueries({ queryKey: ['commit-hint', repoId] });
    },
    onError: (error) => {
      // 失败留在对话框里而不是只弹提示：钩子拒绝的原始输出必须能被读到
      // （T1.8 会把它结构化展示）
      setFailure(normalizeError(error));
    },
  });

  const recentMessages = hintQuery.data?.recentMessages ?? [];
  const recentOptions = recentMessages.slice(0, RECENT_LIMIT).map((message, index) => ({
    value: String(index),
    label: message,
  }));
  const template = hintQuery.data?.template ?? null;

  const submit = () => {
    if (!canSubmit) return;
    prepareMutation.mutate();
  };

  return (
    <section
      className="flex h-full flex-col gap-4"
      data-testid="commit-panel"
      onKeyDown={(event) => {
        // Ctrl/Cmd + Enter 提交：与预览对话框里同一个快捷键
        if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
          event.preventDefault();
          submit();
        }
      }}
    >
      <header className="flex flex-col gap-1">
        <h2 className="text-16 font-semibold tracking-tight">{t('commit.title')}</h2>
        <p className="text-12 text-fg-subtle">{t('commit.description')}</p>
      </header>

      <div className="grid min-h-0 flex-1 gap-4 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <div className="flex min-w-0 flex-col gap-3">
          <Textarea
            label={t('commit.messageLabel')}
            placeholder={t('commit.messagePlaceholder')}
            value={subject}
            rows={3}
            hint={t('commit.counter', {
              count: subjectChars,
              max: SUBJECT_RECOMMENDED_MAX_CHARS,
            })}
            onChange={(event) => {
              setSubject(event.target.value);
            }}
          />
          {subjectChars > SUBJECT_RECOMMENDED_MAX_CHARS ? (
            <p className="text-12 text-warning">
              {t('commit.warningSubjectTooLong', { max: SUBJECT_RECOMMENDED_MAX_CHARS })}
            </p>
          ) : null}

          <Textarea
            label={t('commit.bodyLabel')}
            placeholder={t('commit.bodyPlaceholder')}
            value={description}
            rows={6}
            onChange={(event) => {
              setDescription(event.target.value);
            }}
          />

          <fieldset className="flex flex-col gap-3 rounded-lg border border-line p-3">
            <legend className="px-1 text-12 font-medium text-fg-muted">
              {t('commit.optionsLabel')}
            </legend>

            <Checkbox
              label={t('commit.amend')}
              checked={amend}
              onCheckedChange={(next) => {
                setAmend(next === true);
              }}
            />
            <p className="text-11 text-fg-subtle">{t('commit.amendHint')}</p>

            <Checkbox
              label={t('commit.signOff')}
              checked={signOff}
              onCheckedChange={(next) => {
                setSignOff(next === true);
              }}
            />
            <p className="text-11 text-fg-subtle">{t('commit.signOffHint')}</p>

            <Checkbox
              label={t('commit.noVerify')}
              checked={noVerify}
              onCheckedChange={(next) => {
                setNoVerify(next === true);
              }}
            />
            <p className="text-11 text-fg-subtle">{t('commit.noVerifyHint')}</p>

            <SelectField
              label={t('commit.signLabel')}
              value={sign}
              options={[
                { value: 'auto', label: t('commit.signAuto') },
                { value: 'yes', label: t('commit.signYes') },
                { value: 'no', label: t('commit.signNo') },
              ]}
              onValueChange={(value) => {
                setSign(value as CommitSignMode);
              }}
            />

            <div className="flex flex-col gap-2">
              <Input
                label={t('commit.previewAuthor')}
                value={authorName}
                placeholder="Ada Lovelace"
                onChange={(event) => {
                  setAuthorName(event.target.value);
                }}
              />
              <Input
                srLabel="author email"
                value={authorEmail}
                placeholder="ada@example.com"
                onChange={(event) => {
                  setAuthorEmail(event.target.value);
                }}
              />
            </div>
          </fieldset>
        </div>

        <aside className="flex min-w-0 flex-col gap-3">
          <div className="rounded-lg border border-line bg-surface p-3">
            {statusQuery.isPending ? (
              <Skeleton className="h-5 w-40" />
            ) : stagedCount > 0 ? (
              <p className="text-13 text-fg">{t('commit.stagedCount', { count: stagedCount })}</p>
            ) : (
              <div className="flex flex-col gap-1">
                <p className="text-13 text-fg">{t('commit.stagedNone')}</p>
                <p className="text-11 text-fg-subtle">{t('commit.stagedNoneHint')}</p>
                <Link className="text-12 text-brand hover:underline" to="../status">
                  {t('items.status')}
                </Link>
              </div>
            )}
          </div>

          <SelectField
            label={t('commit.recentLabel')}
            value={selectedRecent}
            options={recentOptions}
            {...(recentOptions.length === 0 ? { placeholder: t('commit.recentEmpty') } : {})}
            onValueChange={(value) => {
              const message = recentMessages[Number(value)];
              if (message !== undefined) {
                setSubject(message);
                setSelectedRecent(value);
              }
            }}
          />

          {template !== null ? (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => {
                setSubject((current) => (current === '' ? template : current));
              }}
            >
              {t('commit.useTemplate', { template })}
            </Button>
          ) : null}

          <div className="mt-auto flex flex-col gap-1.5">
            <Button
              loading={prepareMutation.isPending}
              disabled={!canSubmit}
              onClick={submit}
              data-testid="commit-submit"
            >
              {t('commit.submit')}
            </Button>
            {!canSubmit ? (
              <p className="text-11 text-fg-subtle">
                {subject.trim() === ''
                  ? t('commit.messagePlaceholder')
                  : t('commit.disabledNothingStaged')}
              </p>
            ) : (
              <p className="text-11 text-fg-subtle">{t('commit.shortcut')}</p>
            )}
          </div>
        </aside>
      </div>

      <CommitPreviewDialog
        open={previewOpen}
        plan={plan}
        busy={executeMutation.isPending}
        failure={failure}
        onOpenChange={(open) => {
          setPreviewOpen(open);
          if (!open) {
            setFailure(null);
          }
        }}
        onConfirm={() => {
          if (plan !== null) {
            executeMutation.mutate(plan.planId);
          }
        }}
      />
    </section>
  );
}
