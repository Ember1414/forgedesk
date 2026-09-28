import { expect, test } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

// T2.2 交互级验收：提交历史页（图模式 + 列表模式）的 E2E 覆盖。
//
// mock 方式与 workspace.spec.ts 完全一致：在应用脚本运行前注入
// window.__TAURI_INTERNALS__，把 git_log_page 路由到内存夹具。
// 夹具含 merge、多分支、ref 标签、hidden/collapsed 标记，
// 与后端 camelCase 契约一致。

const VISUAL_DIR = join(process.cwd(), 'test-results', 'graph-visual');

// 确保截图目录存在
mkdirSync(VISUAL_DIR, { recursive: true });

// ---------------------------------------------------------------- 夹具数据

/** 20 个提交（含 merge、多分支、ref、hidden、collapsed），分两页。 */
const HISTORY_FIXTURE = `
(function () {
  function sig(name, email) { return { name: name, email: email, time: 1700000000 }; }
  var commits = [];
  var rows = [];
  var edges = [];
  for (var i = 0; i < 20; i++) {
    var oid = ('000000000000000000000000000000000000000' + i).slice(-40);
    var isMerge = (i === 5 || i === 12);
    var lane = i < 5 ? 0 : (i < 10 ? 1 : (i < 15 ? 0 : 2));
    if (i === 5 || i === 12) lane = 0;
    var hidden = (i === 8 || i === 9);
    var collapsed = (i === 5) ? ['collapsible-branch-tip-1', 'collapsible-branch-tip-2'] : [];
    var refs = [];
    if (i === 0) refs = ['HEAD -> main', 'origin/main'];
    if (i === 3) refs = ['tag: v1.0.0'];
    if (i === 10) refs = ['feature/xyz'];
    commits.push({
      oid: oid,
      parents: i > 0 ? [('000000000000000000000000000000000000000' + (i - 1)).slice(-40)] : [],
      author: sig('Author ' + i, 'author' + i + '@test.dev'),
      committer: sig('Committer ' + i, 'comm' + i + '@test.dev'),
      refs: refs,
      signature: i % 3 === 0 ? 'good' : 'unsigned',
      subject: 'feat: commit message ' + i + (isMerge ? ' (merge)' : ''),
      body: i % 4 === 0 ? 'Detailed body for commit ' + i + '.\\nSecond line.' : null,
    });
    rows.push({
      oid: oid,
      row: i,
      lane: lane,
      colorIndex: lane % 8,
      isMerge: isMerge,
      hidden: hidden,
      collapsed: collapsed,
    });
    if (i > 0) {
      var parentOid = ('000000000000000000000000000000000000000' + (i - 1)).slice(-40);
      var kind = isMerge ? 'merge' : (lane !== rows[i-1].lane ? 'branch' : 'straight');
      edges.push({ fromOid: oid, toOid: parentOid, fromLane: lane, toLane: rows[i-1].lane, kind: kind });
    }
  }
  window.__historyFixture = {
    commits: commits,
    layout: { rows: rows, edges: edges, laneCount: 3 },
    nextCursor: 15,
  };
  window.__historyPage2 = {
    commits: commits.slice(15),
    layout: { rows: rows.slice(15), edges: edges.slice(15), laneCount: 3 },
    nextCursor: null,
  };
})();
`;

const MOCK_SCRIPT = `
  ${HISTORY_FIXTURE}
  var listeners = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === 'git_log_page') {
        var cursor = (args && args.query && args.query.cursor) || 0;
        if (cursor === 0) {
          // 首页返回前 15 个
          var page1 = window.__historyFixture;
          return Promise.resolve({
            commits: page1.commits.slice(0, 15),
            layout: {
              rows: page1.layout.rows.slice(0, 15),
              edges: page1.layout.edges.slice(0, 14),
              laneCount: 3,
            },
            nextCursor: 15,
          });
        }
        // 续页
        var page2 = window.__historyPage2;
        return Promise.resolve(page2);
      }
      if (command === 'repo_recent_list') return Promise.resolve([{ record: { id: 1, path: '/tmp/repo', name: 'repo' }, isOpen: true }]);
      if (command === 'app_version') return Promise.resolve({ version: '0.0.1', gitDescribe: null });
      if (command === 'settings_get') return Promise.resolve(null);
      if (command === 'workspace_status') return Promise.resolve({ branch: { oid: 'abc', head: 'main', detached: false, upstream: null, ahead: null, behind: null }, operation: 'none', staged: [], unstaged: [], untracked: [], conflicted: [], ignored: [], ignoredCount: null });
      if (command === 'plugin:event|unlisten') return Promise.resolve(null);
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

// ---------------------------------------------------------------- 设置

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(MOCK_SCRIPT);
});

// ---------------------------------------------------------------- 测试用例

test('页面加载出图：工具条可见 + 图模式渲染 + __errs 为空', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByRole('heading', { name: '历史' })).toBeVisible();
  await expect(page.getByTestId('history-toolbar')).toBeVisible();
  await expect(page.getByTestId('graph-overlay')).toBeVisible();
  // 确认已加载行数
  await expect(page.getByTestId('history-loaded-count')).toContainText('15');

  // 视觉截图：图模式全景
  await page
    .getByTestId('history-page')
    .screenshot({ path: join(VISUAL_DIR, '01-graph-overview.png') });

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('缩放：点放大→缩放级别变化→点缩小恢复', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  const zoomLevel = page.getByTestId('history-zoom-level');
  await expect(zoomLevel).toContainText('100%');

  await page.getByTestId('history-zoom-in').click();
  await expect(zoomLevel).toContainText('120%');

  await page.getByTestId('history-zoom-out').click();
  await expect(zoomLevel).toContainText('100%');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('平移：Ctrl+滚轮缩放（围绕指针）', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  const canvas = page.getByTestId('graph-hit-layer');
  const box = await canvas.boundingBox();
  expect(box).not.toBeNull();

  // Ctrl+滚轮向上 = 放大
  await page.keyboard.down('Control');
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
  await page.mouse.wheel(0, -100);
  await page.keyboard.up('Control');
  await page.waitForTimeout(100);

  const zoomLevel = page.getByTestId('history-zoom-level');
  const text = await zoomLevel.textContent();
  const percent = parseInt(text ?? '100', 10);
  expect(percent).toBeGreaterThan(100);

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('单选：点击节点选中提交', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  // 点击画布中心偏上的位置（应该能命中某一行）
  const canvas = page.getByTestId('graph-hit-layer');
  const box = await canvas.boundingBox();
  expect(box).not.toBeNull();

  // 点击第一行节点的大致位置（padLeft + laneWidth/2 ≈ 25px from left, rowHeight/2 ≈ 14px from top）
  await page.mouse.click(box!.x + 30, box!.y + 14);
  await page.waitForTimeout(300);

  // 选中可能挂出详情面板，但页面本身必须存活（回归护栏：详情面板的
  // useSyncExternalStore 快照曾经不稳定，选中后会把整个路由炸进错误边界）
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  // 视觉截图：选中态（全页面截图，避免 rAF 驱动的 canvas 重绘触发稳定性等待超时）
  await page.screenshot({ path: join(VISUAL_DIR, '03-selected-state.png') });

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('Ctrl 多选：点击两个节点', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  const canvas = page.getByTestId('graph-hit-layer');
  const box = await canvas.boundingBox();
  expect(box).not.toBeNull();

  // 第一行
  await page.mouse.click(box!.x + 30, box!.y + 14);
  await page.waitForTimeout(100);
  // 第二行（Ctrl+点击）
  await page.keyboard.down('Control');
  await page.mouse.click(box!.x + 30, box!.y + 42);
  await page.keyboard.up('Control');
  await page.waitForTimeout(100);

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('右键菜单打开且写操作项为禁用态', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  const canvas = page.getByTestId('graph-hit-layer');
  const box = await canvas.boundingBox();
  expect(box).not.toBeNull();

  // 右键点击节点区域
  await page.mouse.click(box!.x + 30, box!.y + 14, { button: 'right' });
  await page.waitForTimeout(300);

  // 菜单应出现
  const menu = page.getByTestId('graph-context-menu');
  await expect(menu).toBeVisible();

  // 可用项
  await expect(page.getByTestId('graph-menu-copy-oid')).toBeEnabled();
  await expect(page.getByTestId('graph-menu-copy-message')).toBeEnabled();

  // 禁用项（T2.8 写操作）
  await expect(page.getByTestId('graph-menu-create-tag')).toBeDisabled();
  await expect(page.getByTestId('graph-menu-create-branch')).toBeDisabled();
  await expect(page.getByTestId('graph-menu-cherry-pick')).toBeDisabled();
  await expect(page.getByTestId('graph-menu-revert')).toBeDisabled();
  await expect(page.getByTestId('graph-menu-reset')).toBeDisabled();

  // 视觉截图：右键菜单展开
  await page.screenshot({ path: join(VISUAL_DIR, '04-context-menu.png') });

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('列表模式切换与键盘导航', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  // 切到列表模式
  await page.getByTestId('history-mode-list').click();
  await expect(page.getByTestId('graph-list')).toBeVisible();
  await expect(page.getByTestId('graph-list')).toHaveAttribute('role', 'grid');

  // 键盘 ↓ 导航
  const grid = page.getByTestId('graph-list');
  await grid.focus();
  await expect(grid).toHaveAttribute('aria-activedescendant', 'fd-history-list-row-0');
  await page.keyboard.press('ArrowDown');
  await expect(grid).toHaveAttribute('aria-activedescendant', 'fd-history-list-row-1');
  await page.keyboard.press('ArrowDown');
  await expect(grid).toHaveAttribute('aria-activedescendant', 'fd-history-list-row-2');
  await page.keyboard.press('ArrowUp');
  await expect(grid).toHaveAttribute('aria-activedescendant', 'fd-history-list-row-1');

  // 单击行 = 选中该行（aria-selected 翻转）
  const row1 = page.locator('#fd-history-list-row-1');
  await row1.click();
  await expect(page.locator('[role="row"][aria-selected="true"]')).toHaveCount(1);

  // Enter 选中当前行（与点击走同一个 select 动作：选中 + 打开详情）
  await grid.press('Enter');
  await expect(page.locator('[role="row"][aria-selected="true"]')).toHaveCount(1);

  // Ctrl+A 全选当前已加载的一页
  await grid.press('Control+a');
  await expect(page.locator('[role="row"][aria-selected="true"]')).toHaveCount(15);

  // 视觉截图：列表模式
  await page.screenshot({ path: join(VISUAL_DIR, '05-list-mode.png') });

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('加载更多续页', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();
  await expect(page.getByTestId('history-loaded-count')).toContainText('15');

  // 点击"加载更多"
  await page.getByTestId('history-load-more').click();
  await page.waitForTimeout(500);

  // 行数应增加到 20
  await expect(page.getByTestId('history-loaded-count')).toContainText('20');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('hover 高亮态', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  const canvas = page.getByTestId('graph-hit-layer');
  const box = await canvas.boundingBox();
  expect(box).not.toBeNull();

  // 悬停到节点上
  await page.mouse.move(box!.x + 30, box!.y + 14);
  await page.waitForTimeout(300);

  // 视觉截图：hover 高亮态
  await page
    .getByTestId('history-page')
    .screenshot({ path: join(VISUAL_DIR, '02-hover-highlight.png') });

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('暗色主题截图', async ({ page }) => {
  // 注入暗色主题偏好
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.theme', 'dark');
  });
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();
  await page.waitForTimeout(300);

  // 视觉截图：暗色主题
  await page
    .getByTestId('history-page')
    .screenshot({ path: join(VISUAL_DIR, '06-dark-theme.png') });

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});
