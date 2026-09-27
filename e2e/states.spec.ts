import { expect, test, type Page } from '@playwright/test';

import { gotoApp } from './helpers';

// OPS-7 items 5-7: non-happy states, keyboard access and the uncaught-error gate.

test.beforeEach(async ({ page }) => {
  await gotoApp(page);
});

test('language detection: without a stored preference the UI follows the browser language', async ({
  browser,
}) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  // No pinChineseLanguage here on purpose: this context has an empty localStorage.
  await page.goto('/#/');
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  // The Playwright browser reports en-US, so the en bundle must render.
  await expect(page.getByRole('heading', { level: 1, name: 'Dashboard' })).toBeVisible();
  await context.close();
});

test('error states render without crashing when the IPC bridge is absent (browser preview)', async ({
  page,
}) => {
  // In a plain browser there is no Tauri IPC: settings reads and log reads fail.
  // The pages must show the unified error presentation instead of blank screens.
  await page.getByRole('link', { name: '设置' }).click();
  await expect(page.getByRole('heading', { level: 1, name: '通用' })).toBeVisible();
  await expect(page.getByText('读取设置失败')).toBeVisible({ timeout: 10_000 });
  await expect(page.getByRole('button', { name: '重试' })).toBeVisible();

  await page.getByRole('link', { name: '高级' }).click();
  await expect(page.getByRole('heading', { level: 1, name: '高级' })).toBeVisible();
  await expect(page.getByText('读取日志失败')).toBeVisible({ timeout: 10_000 });
  // 操作历史（T1.11）同样要有降级路径：读不到就说明原因，而不是一张空表
  await expect(page.getByText('读取操作历史失败')).toBeVisible({ timeout: 10_000 });
});

test('retry keeps the error state stable instead of throwing', async ({ page }) => {
  await page.goto('/#/settings/advanced');
  await expect(page.getByText('读取日志失败')).toBeVisible({ timeout: 10_000 });
  // 高级页上有两个"重试"（日志与操作历史）：这里说的是日志那一个
  await page.getByRole('button', { name: '重试' }).first().click();
  await expect(page.getByText('读取日志失败')).toBeVisible({ timeout: 10_000 });
});

test('theme toggle applies data-theme and survives a reload', async ({ page }) => {
  await page.goto('/#/settings/appearance');
  await expect(page.getByRole('heading', { level: 1, name: '外观' })).toBeVisible();

  await page.getByRole('radio', { name: '暗色' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
});

test('keyboard: skip link first, Ctrl+K focuses search, Esc closes the switcher menu', async ({
  page,
}) => {
  await page.keyboard.press('Tab');
  await expect
    .poll(() => page.evaluate(() => document.activeElement?.textContent ?? ''))
    .toContain('跳到主内容');

  await page.keyboard.press('Control+k');
  await expect(
    page.evaluate(() => document.activeElement?.getAttribute('placeholder') ?? ''),
  ).resolves.toContain('搜索');

  const trigger = page.getByRole('button', { name: '当前仓库' });
  await trigger.click();
  // ARIA menu pattern: the menu is named by its trigger (aria-labelledby), so the
  // accessible name is "当前仓库: …" — the visible "选择仓库" is a menu label, not the name.
  await expect(page.getByRole('menu', { name: /当前仓库/ })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu', { name: /当前仓库/ })).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test('uncaught error collection stays empty after all interactions (DoD gate)', async ({
  page,
}) => {
  await page.goto('/#/settings/advanced');
  await expect(page.getByText('读取日志失败')).toBeVisible({ timeout: 10_000 });
  await page.getByRole('button', { name: '重试' }).first().click();
  await openSampleRepoPath(page);
  await page.keyboard.press('Control+k');
  await page.keyboard.press('Escape');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

async function openSampleRepoPath(page: Page): Promise<void> {
  await page.goto('/#/repo/example-forgedesk/status');
  await expect(page.getByRole('heading', { level: 1, name: '工作区' })).toBeVisible();
}
