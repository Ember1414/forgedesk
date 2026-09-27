import { expect, type Locator, type Page } from '@playwright/test';

// Shared helpers for the M0 interaction acceptance suite (OPS-7).

/**
 * Pin the UI language to zh-CN before any app code runs.
 *
 * Real finding from the first E2E run: with no stored preference the app follows
 * the browser language, and the Playwright browser is en-US — the UI rendered in
 * English. The acceptance suite asserts the zh-CN bundle (same as the unit tests),
 * so the preference is pinned here; the detection behavior gets its own spec.
 */
export async function pinChineseLanguage(page: Page): Promise<void> {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
}

/** Navigate to the app and wait for the dashboard heading. */
export async function gotoApp(page: Page, hash = '/#/'): Promise<void> {
  await pinChineseLanguage(page);
  await page.goto(hash);
  await expect(page.getByRole('heading', { level: 1, name: '仪表盘' })).toBeVisible();
}

/**
 * Hit-testing (OPS-7 item 1): elementFromPoint at the element center must land on
 * the element itself or a descendant. Catches "visible but not clickable" caused
 * by overlays, z-index mistakes or pointer-events. Never judge by eye.
 */
export async function expectHitTarget(locator: Locator): Promise<void> {
  const box = await locator.boundingBox();
  expect(box, 'element must be laid out (visible and sized)').not.toBeNull();
  const point = { x: box!.x + box!.width / 2, y: box!.y + box!.height / 2 };
  const hitInside = await locator.first().evaluate((el, { x, y }) => {
    const hit = document.elementFromPoint(x, y);
    return hit !== null && (hit === el || el.contains(hit));
  }, point);
  expect(
    hitInside,
    `elementFromPoint(${point.x},${point.y}) landed outside the target - covered by an overlay?`,
  ).toBe(true);
}

/** Open the repository switcher and pick the sample repository. */
/**
 * 从顶栏的仓库切换器里选第一个仓库（表单来自 `repo_recent_list` 的 mock）。
 *
 * 返回被选中的记录 id（字符串形式）：调用方用它拼后续的 URL 断言。
 * 曾经这里点的是写死的示例仓库 `example-forgedesk`——那种"测试依赖占位数据"
 * 的写法会让测试在数据源换成真实记录时一起失效（本轮就是这样）。
 */
export async function openSampleRepo(page: Page, repoId = '1'): Promise<void> {
  await page.getByRole('button', { name: '当前仓库' }).click();
  await page.getByRole('menuitem').first().click();
  await expect(page).toHaveURL(new RegExp(`#\\/repo\\/${repoId}\\/status`));
}
