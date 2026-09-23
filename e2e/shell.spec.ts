import { expect, test } from '@playwright/test';

import { NAV_SECTIONS, navItemPath } from '../src/app/shell/navItems';

import { expectHitTarget, gotoApp, openSampleRepo } from './helpers';

// OPS-7 items 1-4 for the application shell: hit-testing, click-through,
// cross-panel consistency and the repository id/name mapping.

test.beforeEach(async ({ page }) => {
  await gotoApp(page);
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
  // state must come from the same source of truth.
  await expect(page).toHaveURL(/#\/repo\/example-forgedesk\/status/);
  await expect(nav.getByRole('link', { name: '工作区' })).toBeEnabled();
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
  const repoId = 'example-forgedesk';
  for (const item of NAV_SECTIONS.flatMap((section) => section.items)) {
    const path = navItemPath(item, repoId);
    expect(path, `${item.id} must resolve to a path`).not.toBeNull();
    await page.goto(`/#${path}`);
    await expect(page.getByRole('heading', { level: 1 }).first()).not.toBeEmpty();
  }
});
