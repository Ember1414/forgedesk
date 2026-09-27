import { expect, test } from '@playwright/test';

import { NAV_SECTIONS, navItemPath } from '../src/app/shell/navItems';

import { expectHitTarget, openSampleRepo, pinChineseLanguage } from './helpers';

// OPS-7 items 1-4 for the application shell: hit-testing, click-through,
// cross-panel consistency and the repository id/name mapping.

/**
 * 外壳测试需要一份"最近打开的仓库"记录。
 *
 * M1 收尾前这里不需要 mock：切换器读的是写死的示例仓库，而测试点的也是它
 * （`example-forgedesk`）。换成真实数据源（`repo_recent_list`）之后，
 * 示例仓库不存在了，测试必须提供自己的数据——这也是它该有的样子：
 * 断言依赖的是**明确的输入**，而不是产品里恰好写死的那几个名字。
 */
const MOCK_SCRIPT = `
  window.__errs = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback: function () { return 1; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "repo_recent_list") return Promise.resolve([
        { id: 1, path: "E:\\\\Projects\\\\ForgeDesk", name: "ForgeDesk", defaultBranch: "main", lastOpenedAt: 1700000000000, createdAt: 1, isOpen: true },
        { id: 2, path: "C:\\\\work\\\\notes", name: "notes", defaultBranch: "trunk", lastOpenedAt: 1699999999000, createdAt: 1, isOpen: false }
      ]);
      if (command === "settings_all" || command === "settings_get") return Promise.resolve(null);
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
  await pinChineseLanguage(page);
  await page.goto('/#/');
  await expect(page.getByRole('heading', { level: 1, name: '仪表盘' })).toBeVisible();
});

test('sidebar entries and titlebar buttons are genuinely hittable', async ({ page }) => {
  const nav = page.getByRole('navigation', { name: '主导航' });
  for (const name of ['仪表盘', '代码托管', '插件', '设置']) {
    await expectHitTarget(nav.getByRole('link', { name }));
  }
  // Titlebar: repository switcher and account. The search field is an input.
  await expectHitTarget(page.getByRole('button', { name: '当前仓库' }));
  await expectHitTarget(page.getByRole('button', { name: '账号' }));
  await expectHitTarget(page.getByPlaceholder(/搜索/));
});

test('repository-scoped entries are disabled without a repo and enabled after picking one', async ({
  page,
}) => {
  const nav = page.getByRole('navigation', { name: '主导航' });
  const workspace = nav.getByRole('button', { name: '工作区' });
  await expect(workspace).toBeDisabled();
  expect(await workspace.getAttribute('title')).toContain('请先选择一个仓库');

  await openSampleRepo(page);

  // ID mapping (OPS-7 item 4): the URL segment, the store id and the sidebar
  // state must come from the same source of truth. 1 就是 mock 里第一条记录的 id。
  await expect(page).toHaveURL(/#\/repo\/1\/status/);
  await expect(nav.getByRole('link', { name: '工作区' })).toBeEnabled();
  // 顶栏与仓库页标题都解析出了同一个仓库名（同一份缓存）
  await expect(page.getByRole('button', { name: '当前仓库: ForgeDesk' })).toBeVisible();
  await expect(page.getByRole('heading', { level: 1, name: 'ForgeDesk' })).toBeVisible();
});

test('the switcher lists repositories from local records, not sample data', async ({ page }) => {
  await page.getByRole('button', { name: '当前仓库' }).click();
  const items = page.getByRole('menuitem');
  await expect(items).toHaveCount(2);
  await expect(items.first()).toContainText('ForgeDesk');
  await expect(items.first()).toContainText('E:\\Projects\\ForgeDesk');
  await expect(items.nth(1)).toContainText('notes');
  // 打开/克隆的入口还没实现：必须如实说明，而不是给一个点了必然失败的菜单项
  await expect(page.getByText(/打开 \/ 克隆 \/ 初始化仓库的界面还没做/)).toBeVisible();
});

test('every sidebar entry opens its own page', async ({ page }) => {
  await openSampleRepo(page);

  const nav = page.getByRole('navigation', { name: '主导航' });
  const expectations: Array<[string, string]> = [
    ['工作区', '工作区'],
    ['历史', '历史'],
    ['分支', '分支'],
    ['冲突', '冲突'],
    ['终端', '终端'],
    // 代码托管与设置是"区域"入口，默认打开各自的子页（h1 属于页面而不是导航项）
    ['代码托管', '远程仓库'],
    ['插件', '插件'],
    ['设置', '通用'],
  ];
  for (const [name, heading] of expectations) {
    // 仓库页内部还有同名标签（如"工作区"），必须把范围限定在侧栏导航内
    await nav.getByRole('link', { name }).click();
    await expect(page.getByRole('heading', { level: 1, name: heading })).toBeVisible();
  }
});

test('detail panel switches right -> bottom -> hidden', async ({ page }) => {
  await openSampleRepo(page);
  const panel = page.getByRole('complementary', { name: '详情' });
  await expect(panel).toBeVisible();

  // Radix 的单选 ToggleGroup 在真实浏览器中暴露为 radiogroup/radio
  // （jsdom 的可访问名实现与浏览器不同——这正是 E2E 必须存在的原因之一）
  await page.getByRole('radio', { name: '底部' }).click();
  await expect(panel).toBeVisible();
  await page.getByRole('radio', { name: '隐藏' }).click();
  await expect(panel).toHaveCount(0);
});

test('route table entries resolve (nav vs routes consistency)', async ({ page }) => {
  const repoId = '1';
  for (const item of NAV_SECTIONS.flatMap((section) => section.items)) {
    const path = navItemPath(item, repoId);
    expect(path, `${item.id} must resolve to a path`).not.toBeNull();
    await page.goto(`/#${path}`);
    await expect(page.getByRole('heading', { level: 1 }).first()).not.toBeEmpty();
  }
});
