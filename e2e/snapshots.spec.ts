/**
 * 快照页的交互级验收（M1 / T1.9）。
 *
 * 回滚的真实语义（porcelain/HEAD/索引/未跟踪四项恢复）由
 * `crates/snapshot/tests/ref_manager.rs` 在真实仓库上强制验收；
 * 这里验证的是**界面的闸门**：差异摘要在确认之前展示、
 * 锚点丢失的快照拒绝回滚、操作后状态随之刷新。
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
      if (command === "snapshot_diff") {
        window.__snapshotCalls.push({ command: command, args: args });
        // 快照 1 的锚点被"外部清掉"：界面必须拒绝回滚它
        const refMissing = args.snapshotId === 1;
        return Promise.resolve({
          headChanged: !refMissing,
          indexChanged: !refMissing,
          currentHeadOid: "9999999999999999999999999999999999999999",
          currentIndexTreeOid: refMissing ? null : "8888888888888888888888888888888888888888",
          refMissing: refMissing
        });
      }
      if (command === "snapshot_restore") {
        window.__snapshotCalls.push({ command: command, args: args });
        for (const listener of listeners) {
          listener({ event: "repo:changed", id: 0, payload: { repoId: 1, paths: [] } });
        }
        return Promise.resolve({
          restoredSnapshotId: args.snapshotId,
          headOid: "2222222222222222222222222222222222222222",
          indexTreeOid: "7777777777777777777777777777777777777777",
          preRestoreSnapshotId: 3,
          untrackedPaths: ["scratch.txt"]
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

test('快照列表 → 差异摘要确认 → 回滚 → __errs 为空', async ({ page }) => {
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
  // 未跟踪文件的边界必须如实转述
  await expect(dialog.getByText(/未跟踪文件不会被删除或恢复/)).toBeVisible();

  await dialog.getByRole('button', { name: '回滚', exact: true }).click();

  // Radix Toast 会同时渲染可见元素与 aria-live 通知元素，取第一个
  await expect(page.getByText('已回滚到 2222222').first()).toBeVisible();
  const calls = await page.evaluate(() => window.__snapshotCalls ?? []);
  expect(calls.at(-1)).toMatchObject({
    command: 'snapshot_restore',
    args: { snapshotId: 2 },
  });

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
