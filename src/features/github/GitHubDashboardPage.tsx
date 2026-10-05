/**
 * GitHub 概览页（T4.11 Dashboard 聚合）。
 *
 * # 聚合的数据形态
 *
 * 后端一次命令带回每仓库两个摘要（open PR 数 + 待我审查数、最近一次
 * run）：单仓库失败在**后端**已降级（摘要 null + errors），这里对 null
 * 渲染"获取失败"并保留其他仓库——聚合页最忌讳一个仓库拖死整屏。
 *
 * # 仓库上下文
 *
 * 与 PR/Issue/Actions 页同一形态：显式输入，逗号分隔的 owner/repo
 * 列表（会话内记忆上次输入）。超过 10 个目标在前端截断（后端同款
 * 校验兜底）——聚合的请求预算必须可控（M4 风险表的配额顾虑）。
 *
 * # 徽标复用
 *
 * run 的状态/结论文案复用 `github.actions.*`（同一语义不写两份文案）。
 */
import { useState } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import { repoDashboard } from '@/lib/ipc';
import type { RepoDashboard } from '@/lib/ipc';
import { Button } from '@/ui/components/button';
import { ErrorState } from '@/ui/components/error-state';
import { Input } from '@/ui/components/input';

import { MAX_DASHBOARD_TARGETS } from '@/lib/ipc/dashboard';

/** 与其他 GitHub 页一致的站点常量。 */
const HOST = 'github.com';

/** 仓库定位。 */
interface RepoRef {
  readonly owner: string;
  readonly repo: string;
}

/** 解析逗号/空白分隔的 owner/repo 列表；不合式的条目直接丢弃。 */
function parseTargets(raw: string): RepoRef[] {
  const seen = new Set<string>();
  const targets: RepoRef[] = [];
  for (const part of raw.split(/[,\s]+/)) {
    if (part === '') {
      continue;
    }
    const [owner, repo] = part.split('/');
    if (owner === undefined || repo === undefined || owner === '' || repo === '') {
      continue;
    }
    const key = `${owner}/${repo}`.toLowerCase();
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    targets.push({ owner, repo });
    if (targets.length >= MAX_DASHBOARD_TARGETS) {
      break;
    }
  }
  return targets;
}

export function GitHubDashboardPage() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const [repoInput, setRepoInput] = useState('');
  const [targets, setTargets] = useState<readonly RepoRef[]>([]);
  const [reports, setReports] = useState<readonly RepoDashboard[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);

  const load = (parsed: readonly RepoRef[]) => {
    setLoading(true);
    setFailed(false);
    void repoDashboard({
      host: HOST,
      targets: parsed.map((target) => ({ owner: target.owner, repo: target.repo })),
    })
      .then((report) => {
        setReports(report.repos);
        setFailed(false);
      })
      .catch((raw: unknown) => {
        setFailed(true);
        show(raw);
      })
      .finally(() => setLoading(false));
  };

  const submit = () => {
    const parsed = parseTargets(repoInput);
    if (parsed.length === 0) {
      return;
    }
    setTargets(parsed);
    load(parsed);
  };

  const runBadge = (report: RepoDashboard): string => {
    const run = report.runs;
    if (run === null || run === undefined) {
      return report.errors.length > 0
        ? t('github.dashboard.fetchFailed')
        : t('github.dashboard.noRuns');
    }
    return run.status === 'completed' && run.conclusion !== null
      ? t(`github.actions.conclusion.${run.conclusion}`, { defaultValue: run.conclusion })
      : t(`github.actions.status.${run.status}`, { defaultValue: run.status });
  };

  return (
    <section className="flex flex-col gap-3" data-testid="dashboard-page">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.githubDashboard.title')}</h1>
        <p className="text-13 text-fg-muted">{t('pages.githubDashboard.description')}</p>
      </header>

      <form
        className="flex gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <Input
          value={repoInput}
          onChange={(event) => setRepoInput(event.target.value)}
          placeholder={t('github.dashboard.repoInput')}
          aria-label={t('github.dashboard.repoInput')}
          data-testid="dashboard-repo-input"
        />
        <Button type="submit" variant="secondary" data-testid="dashboard-go">
          {t('github.repos.searchGo')}
        </Button>
      </form>

      {loading ? (
        <p className="text-13 text-fg-subtle" data-testid="dashboard-loading">
          {t('github.repos.loading')}
        </p>
      ) : null}

      {failed && !loading ? (
        <ErrorState
          title={t('github.repos.listErrorHint')}
          onRetry={() => load(targets)}
          retryLabel={t('github.repos.retry')}
        />
      ) : null}

      {targets.length === 0 && !loading && !failed ? (
        <p className="text-13 text-fg-subtle" data-testid="dashboard-empty">
          {t('github.dashboard.empty')}
        </p>
      ) : null}

      <ul className="flex flex-col gap-2" data-testid="dashboard-repos">
        {(reports ?? []).map((report) => (
          <li
            key={`${report.owner}/${report.repo}`}
            className="rounded-md border border-line bg-surface px-3 py-2"
            data-testid={`dashboard-item-${report.owner}-${report.repo}`}
          >
            <div className="flex flex-wrap items-center gap-2 text-13">
              <span className="font-mono font-medium">
                {report.owner}/{report.repo}
              </span>
              <span
                className="rounded-sm border border-line px-1.5 py-0.5 text-11 text-fg-muted"
                data-testid={`dashboard-run-${report.owner}-${report.repo}`}
              >
                {runBadge(report)}
              </span>
              <span className="ml-auto text-12 text-fg-muted">
                {t('github.dashboard.latestRun')} · {report.runs?.name ?? ''}
              </span>
            </div>
            <div className="mt-1 flex flex-wrap gap-3 text-12 text-fg-muted">
              {report.pulls !== null ? (
                <>
                  <span data-testid={`dashboard-pulls-${report.owner}-${report.repo}`}>
                    {t('github.dashboard.openPulls')}:{' '}
                    {report.pulls.openTruncated
                      ? t('github.dashboard.truncated')
                      : report.pulls.openTotal}
                  </span>
                  <span
                    className={report.pulls.awaitingReview > 0 ? 'font-medium text-fg' : undefined}
                    data-testid={`dashboard-awaiting-${report.owner}-${report.repo}`}
                  >
                    {t('github.dashboard.awaitingReview')}: {report.pulls.awaitingReview}
                  </span>
                </>
              ) : (
                <span className="text-fg-subtle">{t('github.dashboard.fetchFailed')}</span>
              )}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
