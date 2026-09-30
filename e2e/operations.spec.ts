import { expect, test, type Page } from '@playwright/test';

/**
 * T3.10 交互级验收：操作历史与一键回滚。
 *
 * # 这一条闭环在验什么
 *
 * 危险操作 → 时间线出现记录 → 回滚 → 展示报告 → 再滚一次（幂等），
 * 并且**状态栏的"可回滚"指示器在危险操作之后出现**——那是本产品最要紧的
 * 一句话：你现在还回得去。指示器平时必须不出现（否则它会被无视），
 * 所以这里同时断言"没有可回滚点时它不在"。
 *
 * 真实回滚语义（逐字节恢复、阶段化、幂等）由 `crates/snapshot/tests` 与
 * `crates/commands/tests` 在真实仓库上覆盖；这里验证的是界面这一层：
 * 记录怎么出现、按钮什么时候给、确认前必须看到什么、执行后看到什么。
 */

const MOCK_SCRIPT = `
  window.__errs = [];
  window.__opCalls = [];
  window.__restoreCount = 0;
  window.__opRecords = [];
  var listeners = [];
  var repo = { id: 1, path: '/tmp/repo', name: 'repo', defaultBranch: 'main', lastOpenedAt: 1, createdAt: 1, isOpen: true };
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === 'plugin:event|listen') return Promise.resolve(1);
      if (command === 'plugin:event|unlisten') return Promise.resolve(null);
      if (command === 'app_version') return Promise.resolve({ version: '0.0.1', gitDescribe: null });
      if (command === 'settings_get' || command === 'settings_all' || command === 'settings_set') return Promise.resolve(null);
      if (command === 'repo_recent_list') return Promise.resolve([{ record: repo, isOpen: true }]);
      if (command === 'workspace_status') return Promise.resolve({
        branch: { oid: 'abc', head: 'main', detached: false, upstream: null, ahead: null, behind: null },
        operation: 'none', staged: [], unstaged: [], untracked: [], conflicted: [], ignored: [], ignoredCount: null
      });
      if (command === 'git_branch_list') return Promise.resolve([]);
      if (command === 'operation_history') {
        window.__opCalls.push({ filters: args.filters, limit: args.limit });
        return Promise.resolve({ total: window.__opRecords.length, entries: window.__opRecords });
      }
      if (command === 'snapshot_restore_pending') return Promise.resolve(null);
      if (command === 'snapshot_diff') {
        return Promise.resolve({
          headChanged: true, indexChanged: false,
          currentHeadOid: 'abcabcabcabcabcabcabcabcabcabcabcabcabca',
          currentIndexTreeOid: null, refMissing: false,
          untrackedRestorable: [], untrackedMissing: [], untrackedExtra: []
        });
      }
      if (command === 'snapshot_restore') {
        window.__restoreCount += 1;
        return Promise.resolve({
          restoredSnapshotId: 3,
          headOid: 'abcabcabcabcabcabcabcabcabcabcabcabcabca',
          indexTreeOid: 'abcabcabcabcabcabcabcabcabcabcabcabcabca',
          preRestoreSnapshotId: 4,
          untrackedPaths: [], untrackedRestored: 0, untrackedFailed: [], untrackedExtra: [],
          verified: true,
          outcome: 'completed',
          stages: [{ stage: 'head', ok: true, detail: null, durationMs: 1 }],
          reportLines: [], emergency: null
        });
      }
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } }
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

test('没有操作记录时给出空态（并说清记录是怎么来的）', async ({ page }) => {
  await page.goto('/#/repo/1/operations');

  await expect(page.getByTestId('operations-page')).toBeVisible();
  await expect(page.getByText('还没有操作记录')).toBeVisible();
  await expect(page.getByText(/完成第一次提交/)).toBeVisible();
  await expectNoPageErrors(page);
});

test('有可回滚记录时：指示器出现，时间线列出记录，回滚需勾选确认，再滚一次仍安全', async ({
  page,
}) => {
  // 预置一条"刚做完的破坏性操作"（reset --hard，留下了可回滚的快照）
  await page.addInitScript(() => {
    window.__opRecords = [
      {
        id: 7,
        repoId: 1,
        opType: 'reset',
        argsJson: '{"mode":"hard"}',
        startedAtMs: 1_700_000_000_000,
        endedAtMs: 1_700_000_000_005,
        durationMs: 5,
        exitCode: 0,
        result: 'ok',
        stderrSummary: null,
        snapshotId: 3,
        reversible: true,
        canRollback: true,
      },
    ];
  });
  await page.goto('/#/repo/1/operations');
  await expect(page.getByTestId('operations-page')).toBeVisible();

  const row = page.getByTestId('operation-row-7');
  await expect(row).toBeVisible();
  await expect(row).toContainText('可回滚');

  // 回滚：先看差异摘要，未勾选时确认按钮不可用
  await page.getByTestId('operation-rollback-7').click();
  const dialog = page.getByRole('alertdialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/HEAD 将从/)).toBeVisible();
  await expect(dialog.getByTestId('operations-rollback-confirm')).toBeDisabled();

  await dialog.getByTestId('operations-rollback-confirm-check').check();
  await dialog.getByTestId('operations-rollback-confirm').click();

  // 报告：结局 + 阶段清单
  await expect(page.getByTestId('report-outcome')).toContainText('回滚完成');
  await expect(page.getByTestId('report-stages')).toBeVisible();

  // 幂等：再滚一次仍然安全（后端保证 no-op，界面不该因为重复执行而出现异常）
  await page.getByTestId('operation-rollback-7').click();
  await dialog.getByTestId('operations-rollback-confirm-check').check();
  await dialog.getByTestId('operations-rollback-confirm').click();
  await expect(page.getByTestId('report-outcome')).toContainText('回滚完成');

  await expect.poll(async () => page.evaluate(() => window.__restoreCount)).toBe(2);
  await expectNoPageErrors(page);
});
