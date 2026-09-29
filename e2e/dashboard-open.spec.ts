/**
 * GIT-01 UI 入口的 E2E：首次启动 → 仪表盘输入路径 → 打开仓库 → 进入工作区。
 *
 * mock 方式与 loop.spec.ts 一致（`window.__TAURI_INTERNALS__` 注入）：这里验证
 * 的是界面与契约的闭环（表单 → repo_open → 跳转 → 最近列表出现），真实打开
 * 语义由 `crates/services/tests/repository_lifecycle.rs` 在真实仓库上保证。
 */
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

async function pinLanguage(page: Page): Promise<void> {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
}

async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

const OPEN_MOCK = `
  var listeners = [];
  var recentRepos = [];
  var nextRepoId = 1;
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === 'repo_open') {
        var path = (args && args.path) || '';
        if (path.indexOf('not-a-repo') >= 0) {
          return Promise.reject({ code: 'PATH_NOT_REPO', message: 'not a git repository' });
        }
        var recordId = nextRepoId++;
        recentRepos.unshift({ id: recordId, path: path, name: path.split('/').pop(),
          defaultBranch: 'main', lastOpenedAt: 1, createdAt: 1, isOpen: true });
        return Promise.resolve({
          recordId: recordId,
          repository: { workdir: path, gitDir: path + '/.git', isBare: false, isEmpty: false,
            head: 'main', detached: false },
          audit: { findings: [] },
          gitVersion: '2.50.0', gitVersionSupported: true, needsGitUpgrade: false
        });
      }
      if (command === 'repo_recent_list') return Promise.resolve(recentRepos);
      if (command === 'workspace_status') {
        return Promise.resolve({ branch: { oid: 'a'.repeat(40), head: 'main', detached: false,
          upstream: null, ahead: 0, behind: 0 }, operation: 'none', staged: [], unstaged: [],
          untracked: [], conflicted: [], ignored: [], ignoredCount: null });
      }
      if (command === 'repo_recent_list') return Promise.resolve(recentRepos);
      if (command === 'app_version') return Promise.resolve({ version: '0.0.1', gitDescribe: null });
      if (command === 'settings_get') return Promise.resolve(null);
      if (command === 'settings_all') return Promise.resolve({});
      if (command === 'logs_tail') return Promise.resolve([]);
      if (command === 'plugin:event|unlisten') return Promise.resolve(null);
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

test('仪表盘输入路径打开仓库：跳工作区、最近列表出现、__errs 为空', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(OPEN_MOCK);
  await page.goto('/#/');
  await expect(page.getByRole('heading', { level: 1, name: '仪表盘' })).toBeVisible();

  // 添加仓库卡片可见；空路径时按钮禁用
  const card = page.getByTestId('add-repo-card');
  await expect(card).toBeVisible();
  await expect(page.getByTestId('add-repo-open-submit')).toBeDisabled();

  // 输入路径 → 打开 → 跳到工作区
  await page.getByTestId('add-repo-path').fill('D:/demo/my-repo');
  await page.getByTestId('add-repo-open-submit').click();
  await expect(page).toHaveURL(/#\/repo\/1\/status/);

  // 回到仪表盘：最近列表出现该仓库（打开成功即登记）
  await page.goBack();
  await expect(page.getByRole('heading', { level: 1, name: '仪表盘' })).toBeVisible();
  await expect(page.getByText('my-repo').first()).toBeVisible();

  await expectNoPageErrors(page);
});

test('打开非仓库路径：弹错误提示、停留在仪表盘', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(OPEN_MOCK);
  await page.goto('/#/');

  await page.getByTestId('add-repo-path').fill('D:/not-a-repo');
  await page.getByTestId('add-repo-open-submit').click();

  // 错误 toast 出现（PATH_NOT_REPO 的标题来自 errors i18n），且不发生跳转
  await expect(page.getByText('这里不是 Git 仓库').first()).toBeVisible();
  await expect(page).toHaveURL(/#\/$/);

  await expectNoPageErrors(page);
});
