/**
 * 提交面板（M1 / T1.7 + T1.8）：写信息 → 预览 → 提交。
 *
 * # 为什么按钮上要写"预览并提交"而不是"提交"
 *
 * 因为它确实分两步：点击后先生成计划（`commit_prepare`），把文件清单、等价命令与
 * 钩子列表摆出来，用户在预览里确认后才真正写仓库。把两步的东西说成一步，
 * 用户就不会去看那一屏——而那一屏正是"提交了什么"的唯一可信来源。
 *
 * # amend 的两条纪律（T1.8）
 *
 * 1. **预填不覆盖**：勾选 amend 时只在输入还空着的情况下填入上一次的信息；
 *    用户已经写了一半的内容不该被一次开关抹掉。
 * 2. **语义必须选**："把暂存内容并进去"与"只改信息"是两种结果完全不同的操作
 *    （`git commit --amend` 默认是前者），因此界面上必须让用户明确选择，
 *    而不是替他决定。
 *
 * # 状态归属
 *
 * 表单内容、计划、预览开关都是**组件局部状态**（只有这个页面用），不入 store；
 * 服务端状态（工作区状态、风格提示、amend 语境、钩子清单）走 TanStack Query
 * （AGENTS.md §6：Git 状态不进 Zustand）。
 */
import { useCallback, useEffect, useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Link, useParams } from 'react-router-dom';

import { CommitPreviewDialog } from '@/features/commit/CommitPreviewDialog';
import { countsOf, groupsOf } from '@/features/workspace/statusModel';
import { STATUS_QUERY_KEY } from '@/features/workspace/WorkspaceStatusPage';
import { normalizeError, useAppError } from '@/lib/errors';
import type { NormalizedError } from '@/lib/errors';
import { isTauriRuntime } from '@/lib/ipc/client';
import {
  commitAmendContext,
  commitExecute,
  commitHooksList,
  commitMessageHint,
  commitPrepare,
} from '@/lib/ipc/commit';
import type { AmendContext, AmendMode, CommitPlan, CommitSignMode } from '@/lib/ipc/commit';
import { onRepoChanged, workspaceStatus } from '@/lib/ipc/workspace';
import { pushToast } from '@/stores/toastStore';
import { Button } from '@/ui/components/button';
import { Checkbox } from '@/ui/components/checkbox';
import { Input } from '@/ui/components/input';
import { SelectField } from '@/ui/components/select';
import { Skeleton } from '@/ui/components/skeleton';
import { Textarea } from '@/ui/components/textarea';
import { ToggleGroup } from '@/ui/components/toggle-group';

/**
 * 首行的建议长度。
 *
 * 与后端 `domain::git::commit_plan::SUBJECT_RECOMMENDED_MAX_CHARS` 保持一致——
 * 后端只在**建议**里用它（不阻断），前端据此显示计数器。两处都改才算改完。
 */
const SUBJECT_RECOMMENDED_MAX_CHARS = 72;

/** 最近提交下拉里展示的条数。 */
const RECENT_LIMIT = 5;

const AMEND_CONTEXT_QUERY_KEY = 'commit-amend';
const HOOKS_QUERY_KEY = 'commit-hooks';
const HINT_QUERY_KEY = 'commit-hint';

export function CommitPage() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const queryClient = useQueryClient();
  const { show } = useAppError();

  const [subject, setSubject] = useState('');
  const [description, setDescription] = useState('');
  const [amend, setAmend] = useState(false);
  const [amendMode, setAmendMode] = useState<AmendMode>('includeStaged');
  const [amendContext, setAmendContext] = useState<AmendContext | null>(null);
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
    queryKey: [HINT_QUERY_KEY, repoId],
    queryFn: () => commitMessageHint(repoId),
    enabled: Number.isFinite(repoId),
  });

  // 钩子清单是"这个仓库的现状"，与提交面板同屏最有用；它很小，随页面加载即可
  const hooksQuery = useQuery({
    queryKey: [HOOKS_QUERY_KEY, repoId],
    queryFn: () => commitHooksList(repoId),
    enabled: Number.isFinite(repoId),
  });

  // 本页依赖的三份服务端状态：工作区状态、风格提示、钩子清单。
  // 只用一处失效逻辑：提交成功后与收到 repo:changed 时走的是同一条路径
  const invalidateRepoQueries = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: [STATUS_QUERY_KEY, repoId] });
    void queryClient.invalidateQueries({ queryKey: [HINT_QUERY_KEY, repoId] });
    void queryClient.invalidateQueries({ queryKey: [HOOKS_QUERY_KEY, repoId] });
  }, [queryClient, repoId]);

  // 用户在别处（状态面板、终端）暂存或改文件后，本页的"已暂存 N 个文件"必须跟着变。
  // 这个 effect 里只做失效（不 setState），因此不会引发额外的渲染回合。
  useEffect(() => {
    if (!isTauriRuntime() || !Number.isFinite(repoId)) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void onRepoChanged((payload) => {
      if (payload.repoId === repoId) {
        invalidateRepoQueries();
      }
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
  }, [invalidateRepoQueries, repoId]);

  const stagedCount = statusQuery.data ? countsOf(groupsOf(statusQuery.data)).staged : 0;
  const subjectChars = subject.trim().length;
  // amend 可以只改信息，因此"没有暂存内容"在 amend 模式下不构成阻断
  const canSubmit = subject.trim() !== '' && (stagedCount > 0 || amend);

  /**
   * 勾选 amend：取回上一次提交的信息并预填。
   *
   * 刻意写成"用户动作 → 一次 await → 一次填充"，而不是"effect 里监听数据到达后
   * 改 state"：后者会让"什么时候发生"变得含糊，而且容易在数据刷新时把用户
   * 正在输入的内容覆盖掉。
   */
  const toggleAmend = async (next: boolean): Promise<void> => {
    setAmend(next);
    if (!next) {
      return;
    }

    try {
      const context = await queryClient.fetchQuery({
        queryKey: [AMEND_CONTEXT_QUERY_KEY, repoId],
        queryFn: () => commitAmendContext(repoId),
      });
      setAmendContext(context);
      // 只在空着的时候填：用户可能已经写好了新信息，开关不该抹掉它
      if (subject.trim() === '') {
        setSubject(context.subject ?? '');
      }
      if (description.trim() === '' && context.body !== null) {
        setDescription(context.body);
      }
    } catch (error) {
      show(normalizeError(error));
    }
  };

  const prepareMutation = useMutation({
    mutationFn: (override: { readonly noVerify?: boolean } = {}) => {
      const body = description.trim();
      const name = authorName.trim();
      const email = authorEmail.trim();
      const author = name !== '' && email !== '' ? { name, email } : undefined;
      return commitPrepare(repoId, {
        message: subject,
        amend,
        amendMode,
        signOff,
        // "跳过钩子重试"走 override：setState 之后同一轮里读到的还是旧值
        noVerify: override.noVerify ?? noVerify,
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
      setAmendContext(null);
      setSelectedRecent(undefined);
      invalidateRepoQueries();
    },
    onError: (error) => {
      // 失败留在对话框里而不是只弹提示：钩子拒绝的原始输出必须能被读到（T1.8）
      setFailure(normalizeError(error));
    },
  });

  const recentMessages = hintQuery.data?.recentMessages ?? [];
  const recentOptions = recentMessages.slice(0, RECENT_LIMIT).map((message, index) => ({
    value: String(index),
    label: message,
  }));
  const template = hintQuery.data?.template ?? null;
  const hooks = hooksQuery.data ?? [];

  const submit = () => {
    if (!canSubmit) return;
    prepareMutation.mutate({});
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

            <div className="flex flex-col gap-2">
              <Checkbox
                label={t('commit.amend')}
                checked={amend}
                disabled={amendContext !== null && amendContext.headOid === null}
                onCheckedChange={(next) => {
                  void toggleAmend(next === true);
                }}
              />
              {amendContext !== null && amendContext.headOid === null ? (
                <p className="text-11 text-fg-subtle">{t('commit.amendEmpty')}</p>
              ) : (
                <p className="text-11 text-fg-subtle">{t('commit.amendHint')}</p>
              )}

              {amend && amendContext !== null && amendContext.headOid !== null ? (
                <div className="flex flex-col gap-2 rounded-md border border-warning bg-surface p-2">
                  {amendContext.pushed ? (
                    <p className="text-12 text-warning">
                      {t(
                        amendContext.pushedRefs.length > 0
                          ? 'commit.amendPushedWarning'
                          : 'commit.amendPushedUnknown',
                        { refs: amendContext.pushedRefs.join(' · ') },
                      )}
                    </p>
                  ) : null}
                  <p className="text-11 text-fg-subtle">
                    {t('commit.amendLastMessage')}: {amendContext.subject ?? ''}
                  </p>
                  <ToggleGroup
                    label={t('commit.amendModeLabel')}
                    value={amendMode}
                    options={[
                      { value: 'includeStaged', label: t('commit.amendModeInclude') },
                      { value: 'messageOnly', label: t('commit.amendModeMessageOnly') },
                    ]}
                    onValueChange={(next) => {
                      setAmendMode(next as AmendMode);
                    }}
                  />
                  <p className="text-11 text-fg-subtle">
                    {amendMode === 'messageOnly'
                      ? t('commit.amendModeMessageOnlyHint')
                      : t('commit.amendModeIncludeHint')}
                  </p>
                </div>
              ) : null}
            </div>

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

          {/* 钩子清单：只展示不编辑（改钩子是编辑器的活，不是 Git 客户端的） */}
          <div className="flex flex-col gap-1 rounded-lg border border-line bg-surface p-3">
            <h3 className="text-12 font-medium text-fg-muted">{t('commit.hooksListTitle')}</h3>
            {hooksQuery.isPending ? (
              <Skeleton className="h-4 w-32" />
            ) : hooks.length === 0 ? (
              <p className="text-11 text-fg-subtle">{t('commit.hooksListEmpty')}</p>
            ) : (
              <ul className="flex flex-col gap-0.5">
                {hooks.map((hook) => (
                  <li key={hook.name} className="flex items-center gap-2 text-11">
                    <span className="font-mono text-fg-muted">{hook.name}</span>
                    {hook.commitHook ? (
                      <span className="text-brand">{t('commit.hooksPlannedShort')}</span>
                    ) : null}
                    {!hook.executable ? (
                      <span className="text-warning">{t('commit.hooksListNotExecutable')}</span>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </div>

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
        onSkipHooks={() => {
          // 用户明确选择了跳过钩子：把勾选状态与事实对齐，然后重新生成计划。
          // 重新生成（而不是直接执行）让用户再确认一次——跳过钩子是个有后果的决定。
          setNoVerify(true);
          prepareMutation.mutate({ noVerify: true });
        }}
      />
    </section>
  );
}
