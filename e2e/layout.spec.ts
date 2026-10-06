import { expect, test } from '@playwright/test';

/**
 * 布局系统 E2E（T5.10）：预设切换 → 持久化 → 重启（reload）保持；
 * 布局损坏容错（坏 JSON → 回退默认 + 提示）。
 */
const LAYOUT_MOCK = `
  window.__errs = [];
  // mock 的 settings 表在 reload 时会被 init 脚本重建——用 localStorage 做跨 reload 持久化
  const settings = JSON.parse(window.localStorage.getItem('forgedesk.e2e.layoutSettings') || '{}');
  const persistSettings = () => window.localStorage.setItem('forgedesk.e2e.layoutSettings', JSON.stringify(settings));
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { return 1; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "settings_set") { settings[args.key] = args.value; persistSettings(); return Promise.resolve(null); }
      if (command === "settings_get") return Promise.resolve(settings[args.key] || null);
      if (command === "settings_all") return Promise.resolve(settings);
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "workspace_status") return Promise.resolve({
        branch: { oid: "a1b2c3", head: "main", detached: false, upstream: null, ahead: 0, behind: 0 },
        operation: "none", staged: [], unstaged: [], untracked: [], conflicted: [], ignored: [], ignoredCount: null
      });
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
  window.__layoutSettings = settings;
`;

test('布局预设保存 → reload 后保持；恢复默认生效', async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
  await page.addInitScript(LAYOUT_MOCK);
  await page.goto('/#/settings/layout');
  await expect(page.getByRole('heading', { level: 1, name: '布局' })).toBeAttached();

  // 选"审查"预设 → 持久化（ToggleGroup 单选项是 radio 语义）
  await page.getByRole('radio', { name: '审查' }).click();
  await expect
    .poll(
      async () =>
        JSON.parse((await page.evaluate(() => window.__layoutSettings?.['ui.layout'])) ?? '{}')
          .preset,
    )
    .toBe('review');

  // reload：设置从 mock 重新载入 → 预设保持
  await page.reload();
  await expect(page.getByRole('radio', { name: '审查' })).toBeChecked();

  // 恢复默认
  await page.getByRole('button', { name: '恢复默认布局' }).click();
  await expect
    .poll(
      async () =>
        JSON.parse((await page.evaluate(() => window.__layoutSettings?.['ui.layout'])) ?? '{}')
          .preset,
    )
    .toBe('default');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('布局损坏容错：坏 JSON → 回退默认并提示', async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
  await page.addInitScript(`${LAYOUT_MOCK}; window.__layoutSettings['ui.layout'] = '{not-json';`);
  await page.goto('/#/settings/layout');

  await expect(page.getByText('保存的布局数据无效，已回退默认')).toBeVisible();
  await expect(page.getByRole('radio', { name: '默认' })).toBeChecked();

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});
