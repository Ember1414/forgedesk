import { expect, test, type Page } from '@playwright/test';

/**
 * T3.1 交互级验收：冲突页（状态机视图）。
 *
 * # 为什么是 mock IPC 而不是真实冲突仓库
 *
 * 与 sync.spec.ts 同一取舍：后端行为（状态采集、stage 校验、abort 快照）由
 * Rust 集成测试在真实仓库上覆盖（`crates/services/tests/conflict.rs`），这里用
 * mock 的 IPC 验证**产品自己的那半条链路**：
 *
 *   状态查询 → 界面正确落出操作横幅 / 文件列表 / 按钮可用性 → 用户动作真的
 *   发出对应的命令（参数正确）→ `__errs` 为空。
 *
 * mock 的字段一律 camelCase（docs/API.md §1 契约）：`Option` 是 **null**
 * 不是 undefined（T2.10 的 serde 教训）。
 */
const MOCK_SCRIPT = `
  window.__errs = [];
  window.__conflictCalls = [];
  const conflictFiles = [
    { path: "src/a.ts", kind: "text", base: { size: 5, isBinary: false, encodingHint: "utf-8", content: "base" },
      ours: { size: 5, isBinary: false, encodingHint: "utf-8", content: "ours" },
      theirs: { size: 6, isBinary: false, encodingHint: "utf-8", content: "theirs" }, worktreeExists: true }
  ];
  let resolved = false;
  function conflictState() {
    if (!resolved) {
      return { opKind: "merge", opInProgress: true, currentStep: null, totalSteps: null,
        headName: null, intoBranch: "main", files: conflictFiles,
        canContinue: false, canAbort: true, canSkip: false };
    }
    return { opKind: "merge", opInProgress: true, currentStep: null, totalSteps: null,
      headName: null, intoBranch: "main", files: [],
      canContinue: true, canAbort: true, canSkip: false };
  }
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { return 1; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "plugin:event|listen") return Promise.resolve(1);
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_all" || command === "settings_get") return Promise.resolve(null);
      if (command === "settings_set") return Promise.resolve(null);
      if (command === "git_branch_list") return Promise.resolve([]);
      if (command === "workspace_status") return Promise.resolve({
        branch: { oid: "a1b2c3", head: "main", detached: false, upstream: null, ahead: null, behind: null },
        operation: "merge", staged: [], unstaged: [], untracked: [], conflicted: ["src/a.ts"], ignored: [], ignoredCount: null
      });
      if (command === "git_conflict_state") return Promise.resolve(conflictState());
      if (command === "git_conflict_mark_resolved") {
        window.__conflictCalls.push({ command: command, args: args });
        resolved = true;
        return Promise.resolve(null);
      }
      if (command === "git_conflict_continue" || command === "git_conflict_abort" || command === "git_conflict_skip") {
        window.__conflictCalls.push({ command: command, args: args });
        if (command === "git_conflict_continue") return Promise.resolve({ oid: "a1b2c3d4", conflicts: [] });
        if (command === "git_conflict_abort") return Promise.resolve({ headOid: "a1b2c3", headRef: "main", snapshotId: 7 });
        return Promise.resolve({ oid: null, conflicts: ["src/a.ts"] });
      }
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

async function openConflictPage(page: Page): Promise<void> {
  await page.goto('/#/repo/1/conflict');
  await expect(page.getByTestId('conflict-page')).toBeVisible();
}

async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(MOCK_SCRIPT);
});

test('冲突页显示操作类型、文件清单，且未解决时不能继续', async ({ page }) => {
  await openConflictPage(page);

  await expect(page.getByTestId('conflict-op-kind')).toHaveText('合并');
  await expect(page.getByTestId('conflict-into-branch')).toContainText('main');
  await expect(page.getByTestId('conflict-file-row')).toHaveCount(1);
  await expect(page.getByTestId('conflict-file-row')).toContainText('src/a.ts');
  await expect(page.getByTestId('conflict-file-row')).toContainText('文本冲突');
  await expect(page.getByTestId('conflict-continue')).toBeDisabled();
  await expect(page.getByTestId('conflict-abort')).toBeEnabled();
  await expectNoPageErrors(page);
});

test('标记已解决后可以继续，继续发出的是 git_conflict_continue', async ({ page }) => {
  await openConflictPage(page);

  await page.getByRole('button', { name: '标记已解决' }).click();
  await expect(page.getByTestId('conflict-continue')).toBeEnabled({ timeout: 5000 });
  await page.getByTestId('conflict-continue').click();

  await expect
    .poll(() => page.evaluate(() => window.__conflictCalls?.map((call) => call.command) ?? []))
    .toEqual(['git_conflict_mark_resolved', 'git_conflict_continue']);
  // mark_resolved 的参数必须是状态页报告的路径数组
  const calls = await page.evaluate(() => window.__conflictCalls ?? []);
  expect(calls[0]?.args).toEqual({ repoId: 1, paths: ['src/a.ts'] });
  await expectNoPageErrors(page);
});

test('中止必须经过确认框，确认后发出 git_conflict_abort', async ({ page }) => {
  await openConflictPage(page);

  await page.getByTestId('conflict-abort').click();
  // 确认框弹出但尚未执行
  await expect(page.getByTestId('conflict-abort-confirm')).toBeVisible();
  expect(await page.evaluate(() => window.__conflictCalls ?? [])).toEqual([]);

  await page.getByTestId('conflict-abort-confirm').click();
  await expect
    .poll(() => page.evaluate(() => window.__conflictCalls?.map((call) => call.command) ?? []))
    .toEqual(['git_conflict_abort']);
  await expectNoPageErrors(page);
});
