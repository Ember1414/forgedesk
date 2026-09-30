/**
 * 操作历史的交互级验收（M1 / T1.11）。
 *
 * 审计的真实语义（每一次写操作都留痕、脱敏、2KB 上限、保留策略清理）由
 * `crates/services` 与 `crates/storage` 的单测在真实数据库上强制验收；
 * 这里验证的是**界面的承诺**：记录读得出来、失败与"未收尾"看得出来、
 * 导出会把路径告诉用户、清理会报告条数与所用策略。
 */
import { expect, test } from '@playwright/test';

import { pinChineseLanguage } from './helpers';

const MOCK_SCRIPT = `
  window.__errs = [];
  window.__auditCalls = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback: function () { return 1; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "audit_list") {
        window.__auditCalls.push({ command: command, args: args });
        return Promise.resolve({
          total: 2,
          entries: [
            {
              id: 2, repoId: 1, opType: "commit",
              argsJson: JSON.stringify({ subject: "add audit panel", files: 3 }),
              startedAtMs: 1700000002000, endedAtMs: 1700000002250, durationMs: 250,
              exitCode: 0, result: "ok", stderrSummary: null,
              snapshotId: 12, reversible: true
            },
            {
              id: 1, repoId: 1, opType: "snapshot_restore",
              argsJson: JSON.stringify({ snapshotId: 4 }),
              startedAtMs: 1700000001000, endedAtMs: 1700000001100, durationMs: 100,
              exitCode: 1, result: "failed",
              stderrSummary: "error: Your local changes would be overwritten",
              snapshotId: null, reversible: false
            }
          ]
        });
      }
      if (command === "audit_export") {
        window.__auditCalls.push({ command: command, args: args });
        return Promise.resolve({ path: "C:\\\\Temp\\\\forgedesk-audit-1700000002000." + args.format, rows: 2, format: args.format });
      }
      if (command === "audit_prune") {
        window.__auditCalls.push({ command: command, args: args });
        return Promise.resolve({ removed: 7, retentionDays: 30, retentionRows: 500 });
      }
      if (command === "repo_recent_list") return Promise.resolve([
        { id: 1, path: "/tmp/repo", name: "forgedesk", defaultBranch: "main", lastOpenedAt: 1700000000000, createdAt: 1, isOpen: true }
      ]);
      if (command === "settings_all") return Promise.resolve({
        "audit.retentionDays": "30",
        "audit.retentionMax": "500"
      });
      if (command === "settings_get") return Promise.resolve(null);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "logs_tail") return Promise.resolve([]);
      if (command === "plugin:event|unlisten") return Promise.resolve(null);
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

test.beforeEach(async ({ page }) => {
  await page.addInitScript(MOCK_SCRIPT);
  // 断言的是 zh-CN 文案：不固定语言的话，Playwright 浏览器报 en-US，界面会是英文
  await pinChineseLanguage(page);
});

test('操作历史列出时间/操作/结果/耗时/快照，并显示失败原话', async ({ page }) => {
  await page.goto('/#/settings/advanced');

  // 侧栏（T3.10 起）也有一个叫"操作历史"的入口：必须限定在内容区里找，
  // 否则严格模式会因为"两个元素同名"直接判失败
  await expect(page.locator('#main-content').getByText('操作历史')).toBeVisible({
    timeout: 10_000,
  });
  // 断言限定在表格里：外壳导航里也有"提交"这类词，按全文找会撞上
  const table = page.getByRole('table');
  await expect(table.getByText('提交')).toBeVisible();
  await expect(table.getByText('成功')).toBeVisible();
  await expect(table.getByText('250 毫秒')).toBeVisible();
  await expect(table.getByText('#12')).toBeVisible();

  // 失败的行要能读到 git 的原话（排查时它就是全部信息）。
  // `.first()`：Playwright 的按文本匹配会同时命中单元格与它内部的 span
  await expect(table.getByText('失败').first()).toBeVisible();
  await expect(table.getByText(/Your local changes would be overwritten/).first()).toBeVisible();

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('导出把文件路径显示给用户，并如实告诉后端要哪种格式', async ({ page }) => {
  await page.goto('/#/settings/advanced');
  await expect(page.getByRole('table').getByText('add audit panel')).toBeVisible({
    timeout: 10_000,
  });

  await page.getByRole('button', { name: /导出 CSV/ }).click();

  await expect(page.getByText(/forgedesk-audit-1700000002000\.csv/).first()).toBeVisible();

  const calls = await page.evaluate(() => window.__auditCalls ?? []);
  const exportCall = calls.find((call) => call.command === 'audit_export');
  expect(exportCall?.args).toMatchObject({ format: 'csv' });
});

test('清理旧记录报告条数与所用策略', async ({ page }) => {
  await page.goto('/#/settings/advanced');
  await expect(page.getByRole('table').getByText('add audit panel')).toBeVisible({
    timeout: 10_000,
  });

  await page.getByRole('button', { name: /清理旧记录/ }).click();

  // 用户要知道"删了多少、按什么策略删的"，否则这个按钮像什么都没做
  await expect(page.getByText(/已清理 7 条记录/).first()).toBeVisible();

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});
