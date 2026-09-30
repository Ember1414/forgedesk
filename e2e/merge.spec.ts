import { expect, test, type Page } from '@playwright/test';

/**
 * T3.4 交互级验收：合并对话框与进行中横幅。
 *
 * 与冲突页 e2e 同一取舍：后端行为（预检一致性、ff/squash 语义）由 services
 * 集成测试在真实仓库上覆盖（crates/services/tests/merge.rs，12 条）；这里
 * 用 mock IPC 验证产品自己的半条链路：
 *
 *   分支页"合并"按钮 → 选源 → 预览（计划内容正确呈现）→ 执行（参数正确）
 *   → conflicted 落到冲突页；合并进行中横幅（继续/中止）→ __errs 为空。
 */
const MOCK_SCRIPT = `
  window.__errs = [];
  window.__mergeCalls = [];
  const branches = [
    { name: "main", isRemote: false, isHead: true, target: "a1b2c3", upstream: null, ahead: null, behind: null, upstreamGone: false },
    { name: "feature", isRemote: false, isHead: false, target: "d4e5f6", upstream: null, ahead: null, behind: null, upstreamGone: false }
  ];
  window.__TAURI_INTERNALS__ = {
    transformCallback: function () { return 1; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "plugin:event|listen") return Promise.resolve(1);
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_all" || command === "settings_get") return Promise.resolve(null);
      if (command === "settings_set") return Promise.resolve(null);
      if (command === "git_branch_list") return Promise.resolve(branches);
      if (command === "git_tag_list") return Promise.resolve([]);
      if (command === "workspace_status") return Promise.resolve({
        branch: { oid: "a1b2c3", head: "main", detached: false, upstream: null, ahead: null, behind: null },
        operation: "none", staged: [], unstaged: [], untracked: [], conflicted: [], ignored: [], ignoredCount: null
      });
      if (command === "git_merge_prepare") {
        window.__mergeCalls.push({ command: command, args: args });
        return Promise.resolve({ planId: "plan-1", source: "feature", strategy: "merge",
          verdict: "trueMerge", sourceOnlyCommits: [{ oid: "d4e5f6a", subject: "feature work", authorTime: 1 }],
          sourceCommitCount: 1, previewAvailable: true, conflicted: ["a.txt"],
          defaultMessage: "Merge branch 'feature' into main", equivalentCommand: "git merge feature" });
      }
      if (command === "git_merge_execute") {
        window.__mergeCalls.push({ command: command, args: args });
        return Promise.resolve({ kind: "conflicted", oid: null, conflicts: ["a.txt"], snapshotId: 7 });
      }
      if (command === "git_conflict_state") {
        return Promise.resolve({ opKind: "merge", opInProgress: true, currentStep: null, totalSteps: null,
          headName: null, intoBranch: "main",
          files: [{ path: "a.txt", kind: "text", base: null, ours: null, theirs: null, worktreeExists: true }],
          canContinue: false, canAbort: true, canSkip: false });
      }
      if (command === "git_merge_continue" || command === "git_conflict_abort") {
        window.__mergeCalls.push({ command: command, args: args });
        return Promise.resolve({ kind: "mergeCommit", oid: "abc123", conflicts: [], snapshotId: 8 });
      }
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(MOCK_SCRIPT);
});

test('合并对话框：预览展示预检冲突，执行发出带 planId 的命令', async ({ page }) => {
  await page.goto('/#/repo/1/branches');
  await page.getByTestId('branches-merge').click();

  await page.getByTestId('merge-source').selectOption('feature');
  await page.getByTestId('merge-preview').click();

  const plan = page.getByTestId('merge-plan');
  await expect(plan).toBeVisible();
  await expect(page.getByTestId('merge-conflicts')).toContainText('a.txt');
  await expect(page.getByTestId('merge-equivalent')).toContainText('git merge feature');
  await expect(page.getByTestId('merge-message')).toHaveValue("Merge branch 'feature' into main");

  await page.getByTestId('merge-execute').click();
  await expect
    .poll(() => page.evaluate(() => window.__mergeCalls?.map((call) => call.command) ?? []))
    .toEqual(['git_merge_prepare', 'git_merge_execute']);
  const calls = await page.evaluate(() => window.__mergeCalls ?? []);
  const executeCall = calls[1] as { command: string; args: { spec: { planId: string } } } | undefined;
  expect(executeCall?.args.spec.planId).toBe('plan-1');
  await expectNoPageErrors(page);
});

test('合并进行中横幅：常驻显示冲突数，中止必须发出 abort', async ({ page }) => {
  // 进入"合并中"状态：workspace_status 的 operation 改为 merge
  await page.addInitScript(`
    const originalInvoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = function (command, args) {
      if (command === "workspace_status") {
        return originalInvoke(command, args).then((status) => ({ ...status, operation: "merge" }));
      }
      return originalInvoke(command, args);
    };
  `);

  await page.goto('/#/repo/1/status');
  const banner = page.getByTestId('merge-banner');
  await expect(banner).toBeVisible();
  await expect(banner).toContainText('1 个文件待解决');
  // 有未解决冲突：不能继续，但可以中止
  await expect(page.getByTestId('merge-banner-continue')).toBeDisabled();
  await page.getByTestId('merge-banner-abort').click();
  await expect
    .poll(() => page.evaluate(() => window.__mergeCalls?.map((call) => call.command) ?? []))
    .toContain('git_conflict_abort');
  await expectNoPageErrors(page);
});
