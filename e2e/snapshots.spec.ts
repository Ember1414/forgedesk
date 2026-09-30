/**
 * 快照页的交互级验收（M1 / T1.9；T3.8 补内容备份的可见性）。
 *
 * 回滚的真实语义（porcelain/HEAD/索引/未跟踪内容逐字节恢复）由
 * `crates/snapshot/tests/ref_manager.rs` 在真实仓库上强制验收；
 * 这里验证的是**界面的闸门与如实转述**：
 * 差异摘要在确认之前展示、锚点丢失的快照拒绝回滚、超限未备份时说清代价、
 * 回滚报告把"哪一步成了"列出来。
 */
import { expect, test } from '@playwright/test';

const MOCK_SCRIPT = `
  const snapshots = [
    { id: 2, label: "pre-commit", kind: "pre-commit", headOid: "2222222222222222222222222222222222222222", branch: "main", detached: false, createdAtMs: 1700000002000 },
    { id: 1, label: "pre-commit", kind: "pre-commit", headOid: "1111111111111111111111111111111111111111", branch: "main", detached: false, createdAtMs: 1700000001000 }
  ];
  window.__errs = [];
  window.__snapshotCalls = [];
  const listeners = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "snapshot_list") return Promise.resolve(snapshots);
      if (command === "snapshot_usage") {
        return Promise.resolve({
          repoId: 1, snapshotCount: 2, backupBytes: 4096,
          maxSnapshotBytes: 209715200, maxRepoBytes: 2147483648,
          orphanDirs: ["999999"]
        });
      }
      if (command === "snapshot_diff") {
        window.__snapshotCalls.push({ command: command, args: args });
        // 快照 1 的锚点被"外部清掉"：界面必须拒绝回滚它
        const refMissing = args.snapshotId === 1;
        return Promise.resolve({
          headChanged: !refMissing,
          indexChanged: !refMissing,
          currentHeadOid: "9999999999999999999999999999999999999999",
          currentIndexTreeOid: refMissing ? null : "8888888888888888888888888888888888888888",
          refMissing: refMissing,
          // 未跟踪内容的三分类（T3.8）：会恢复 1 个、找不回 0 个、不会删 1 个
          untrackedRestorable: refMissing ? [] : ["scratch.txt"],
          untrackedMissing: [],
          untrackedExtra: ["notes/new.txt"]
        });
      }
      if (command === "snapshot_create") {
        window.__snapshotCalls.push({ command: command, args: args });
        if (window.__createOutcome) return Promise.resolve(window.__createOutcome);
        return Promise.resolve({ id: 3, backupBytes: 1024, backedUp: 1, untrackedTotal: 1, skipped: [], warnings: [], pruned: [] });
      }
      if (command === "snapshot_cleanup") {
        window.__snapshotCalls.push({ command: command, args: args });
        return Promise.resolve({ orphansRemoved: 1, reclaimed: [1], freedBytes: 2048, remainingBytes: 2048 });
      }
      if (command === "snapshot_restore") {
        window.__snapshotCalls.push({ command: command, args: args });
        for (const listener of listeners) {
          listener({ event: "repo:changed", id: 0, payload: { repoId: 1, kind: "refs", paths: [] } });
        }
        return Promise.resolve({
          restoredSnapshotId: args.snapshotId,
          headOid: "2222222222222222222222222222222222222222",
          indexTreeOid: "7777777777777777777777777777777777777777",
          preRestoreSnapshotId: 3,
          untrackedPaths: ["scratch.txt"],
          untrackedRestored: 1,
          untrackedFailed: [],
          untrackedExtra: ["notes/new.txt"],
          verified: true
        });
      }
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_get") return Promise.resolve(null);
      if (command === "plugin:event|unlisten") return Promise.resolve(null);
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(MOCK_SCRIPT);
});

test('快照列表 → 差异摘要（含未跟踪三分类）→ 回滚 → 报告 → __errs 为空', async ({ page }) => {
  await page.goto('/#/repo/1/snapshots');
  await expect(page.getByTestId('snapshots-page')).toBeVisible();

  // 两条快照，新的在前
  await expect(page.getByText('2222222')).toBeVisible();
  await expect(page.getByText('1111111')).toBeVisible();

  // 回滚最新的（列表第一行）：确认框里必须先出现差异摘要
  await page.getByRole('button', { name: '回滚到这里' }).first().click();
  // AlertDialog 的角色是 alertdialog（不是 dialog）
  const dialog = page.getByRole('alertdialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/HEAD 将从 9999999 回到 2222222/)).toBeVisible();
  // T3.8：未跟踪内容的三种后果必须分开说
  await expect(dialog.getByText(/会写回 1 个未跟踪文件/)).toBeVisible();
  await expect(dialog.getByText(/不会被删除/)).toBeVisible();

  await dialog.getByRole('button', { name: '回滚', exact: true }).click();

  // Radix Toast 会同时渲染可见元素与 aria-live 通知元素，取第一个
  await expect(page.getByText('已回滚到 2222222').first()).toBeVisible();
  const calls = await page.evaluate(() => window.__snapshotCalls ?? []);
  expect(calls.at(-1)).toMatchObject({
    command: 'snapshot_restore',
    args: { snapshotId: 2 },
  });

  // 回滚报告：可折叠清单，说清"哪一步成了、校验过没过"
  const report = page.getByTestId('snapshot-report');
  await expect(report).toBeVisible();
  await expect(report).toContainText('未跟踪文件已恢复 1 个');
  await expect(report).toContainText('校验通过');
  await expect(report).toContainText('未被删除');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('锚点丢失的快照拒绝回滚并说明原因', async ({ page }) => {
  await page.goto('/#/repo/1/snapshots');
  await expect(page.getByTestId('snapshots-page')).toBeVisible();

  // 快照 1（列表第二行）的锚点被 mock 标记为丢失
  await page.getByRole('button', { name: '回滚到这里' }).nth(1).click();
  const dialog = page.getByRole('alertdialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/锚点已丢失/)).toBeVisible();

  const confirm = dialog.getByRole('button', { name: '回滚', exact: true });
  await expect(confirm).toBeDisabled();

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('占用行显示配额与孤儿目录，清理缓存发出 snapshot_cleanup', async ({ page }) => {
  await page.goto('/#/repo/1/snapshots');
  await expect(page.getByTestId('snapshots-page')).toBeVisible();

  const usage = page.getByTestId('snapshot-usage');
  await expect(usage).toContainText('2 个快照');
  await expect(usage).toContainText('4.0 KB');
  await expect(usage).toContainText('上限 2.0 GB');
  // 孤儿目录要在界面上点出来（否则用户不知道清理能清到什么）
  await expect(page.getByTestId('snapshot-orphans')).toContainText('1 个孤立目录');

  await page.getByTestId('snapshot-cleanup').click();
  await expect
    .poll(async () =>
      (await page.evaluate(() => window.__snapshotCalls ?? [])).map((call) => call.command),
    )
    .toContain('snapshot_cleanup');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('手动打点超限时把"未包含什么"挂在页面上', async ({ page }) => {
  await page.addInitScript(() => {
    window.__createOutcome = {
      id: 3,
      backupBytes: 0,
      backedUp: 0,
      untrackedTotal: 3,
      skipped: ['big.bin', 'scratch/a.txt', 'scratch/b.txt'],
      warnings: [
        {
          kind: 'untrackedBackupSkipped',
          count: 3,
          bytes: 314572800,
          limit: 209715200,
          paths: [],
          detail: null,
          removed: [],
          freedBytes: null,
        },
      ],
      pruned: [],
    };
  });
  await page.goto('/#/repo/1/snapshots');
  await expect(page.getByTestId('snapshots-page')).toBeVisible();

  await page.getByTestId('snapshot-create').click();

  const warning = page.getByTestId('snapshot-warning');
  await expect(warning).toBeVisible();
  await expect(warning).toContainText('未包含 3 个未跟踪文件');
  await expect(warning).toContainText('300.0 MB');
  await expect(warning).toContainText('200.0 MB');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});
