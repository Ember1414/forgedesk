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
  window.__branches = [
    { name: 'main', isRemote: false, isHead: true, target: '0000000000000000000000000000000000000000', upstream: 'origin/main', ahead: 0, behind: 0, upstreamGone: false },
    { name: 'feature/xyz', isRemote: false, isHead: false, target: '0000000000000000000000000000000000000000', upstream: null, ahead: null, behind: null, upstreamGone: false },
    { name: 'origin/main', isRemote: true, isHead: false, target: '0000000000000000000000000000000000000000', upstream: null, ahead: null, behind: null, upstreamGone: false },
  ];
  window.__tags = [
    { name: 'v1.0.0', target: 't0', commit: '0000000000000000000000000000000000000000', annotated: false, message: null, createdAt: 1700000000 },
  ];
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
      if (command === 'git_log_authors') {
        var authors = [];
        for (var ai = 0; ai < 20; ai++) {
          authors.push({ name: 'Author ' + ai, email: 'author' + ai + '@test.dev', commitCount: 1 });
        }
        return Promise.resolve(authors);
      }
      if (command === 'git_branch_list') {
        return Promise.resolve(window.__branches);
      }
      if (command === 'git_tag_list') {
        return Promise.resolve(window.__tags);
      }
      if (command === 'git_branch_create') {
        var createdSpec = (args && args.spec) || { name: 'new-branch' };
        window.__branches.push({
          name: createdSpec.name,
          isRemote: false,
          isHead: !!createdSpec.checkout,
          target: '0000000000000000000000000000000000000000',
          upstream: null,
          ahead: null,
          behind: null,
          upstreamGone: false,
        });
        if (createdSpec.checkout) {
          for (var bi2 = 0; bi2 < window.__branches.length; bi2++) {
            window.__branches[bi2].isHead = window.__branches[bi2].name === createdSpec.name;
          }
        }
        return Promise.resolve(null);
      }
      if (command === 'git_branch_switch') {
        var switchTarget = (args && args.target) || '';
        for (var bi3 = 0; bi3 < window.__branches.length; bi3++) {
          window.__branches[bi3].isHead = window.__branches[bi3].name === switchTarget;
        }
        return Promise.resolve(null);
      }
      if (command === 'git_branch_rename') {
        var rn = (args && args.spec) || {};
        for (var bi4 = 0; bi4 < window.__branches.length; bi4++) {
          if (window.__branches[bi4].name === rn.old) window.__branches[bi4].name = rn.new;
        }
        return Promise.resolve(null);
      }
      if (command === 'git_branch_delete') {
        var del = (args && args.spec) || { names: [] };
        window.__branches = window.__branches.filter(function (b2) {
          return del.names.indexOf(b2.name) < 0;
        });
        return Promise.resolve({ deleted: del.names });
      }
      if (command === 'git_branch_compare') {
        return Promise.resolve({
          ahead: 1,
          behind: 0,
          onlyInA: [['1111111111111111111111111111111111111111', 'unmerged work']],
        });
      }
      if (command === 'git_branch_set_upstream') return Promise.resolve(null);
      if (command === 'git_tag_create') {
        var tc = (args && args.spec) || { name: 'x' };
        window.__tags.push({
          name: tc.name,
          target: 't0',
          commit: '0000000000000000000000000000000000000000',
          annotated: !!tc.message,
          message: tc.message || null,
          createdAt: 1700000000,
        });
        return Promise.resolve(null);
      }
      if (command === 'git_tag_delete') {
        var td = (args && args.spec) || { names: [] };
        window.__tags = window.__tags.filter(function (tg) {
          return td.names.indexOf(tg.name) < 0;
        });
        return Promise.resolve(null);
      }
      if (command === 'git_log_page') {
        var cursor = (args && args.query && args.query.cursor) || 0;
        var q = (args && args.query) || {};
        // 让筛选真实生效（E2E 断言 filtered 数量）：author 子串 + 关键词大小写口径
        var activeAuthor = q.author ? String(q.author) : null;
        var term = q.messageContains ? String(q.messageContains) : null;
        var ci = q.caseInsensitive === undefined ? false : !!q.caseInsensitive;
        var filteredIdx = [];
        for (var fi2 = 0; fi2 < 20; fi2++) {
          var c = window.__historyFixture.commits[fi2];
          if (activeAuthor !== null
            && c.author.name.indexOf(activeAuthor) < 0
            && c.author.email.indexOf(activeAuthor) < 0) {
            continue;
          }
          if (term !== null) {
            var hay = ci ? c.subject.toLowerCase() : c.subject;
            var needle = ci ? term.toLowerCase() : term;
            if (hay.indexOf(needle) < 0) {
              continue;
            }
          }
          filteredIdx.push(fi2);
        }
        if (activeAuthor !== null || term !== null) {
          // 筛选态：一次回完整结果（避免 fixture 的分页语义与筛选叠加）
          // 行号按筛选后的流重排（与后端契约一致：cursor 是筛选结果里的全局序号）
          var rowsF = filteredIdx.map(function (i2, newRow) {
            var row = window.__historyFixture.layout.rows[i2];
            return { oid: row.oid, row: newRow, lane: row.lane, colorIndex: row.colorIndex, isMerge: row.isMerge, hidden: row.hidden, collapsed: row.collapsed };
          });
          var oidToRow = {};
          for (var ri = 0; ri < rowsF.length; ri++) oidToRow[rowsF[ri].oid] = rowsF[ri].row;
          var edgesF = window.__historyFixture.layout.edges.filter(function (e2) {
            return oidToRow[e2.fromOid] !== undefined && oidToRow[e2.toOid] !== undefined;
          }).map(function (e2) {
            var kind = oidToRow[e2.fromOid] === oidToRow[e2.toOid] ? 'straight' : 'branch';
            return { fromOid: e2.fromOid, toOid: e2.toOid, fromLane: e2.fromLane, toLane: e2.toLane, kind: kind };
          });
          return Promise.resolve({
            commits: filteredIdx.map(function (i2) { return window.__historyFixture.commits[i2]; }),
            layout: { rows: rowsF, edges: edgesF, laneCount: 3 },
            nextCursor: null,
          });
        }
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
      if (command === 'git_commit_detail') {
        var detailOid = (args && args.oid) || '';
        var detailIdx = parseInt(detailOid.slice(-1), 10);
        var zeroPad = function (n) {
          return ('000000000000000000000000000000000000000' + n).slice(-40);
        };
        var isMergeCommit = detailIdx === 5;
        var parents = detailIdx === 0
          ? []
          : (isMergeCommit ? [zeroPad(4), zeroPad(99)] : [zeroPad(detailIdx - 1)]);
        var parentIndex = (args && args.parentIndex) || 0;
        var detailFiles = isMergeCommit && parentIndex === 1
          ? [
              { path: 'main/b.txt', oldPath: null, kind: 'modified', binary: false, additions: 2, deletions: 1, truncated: false },
            ]
          : (isMergeCommit
              ? [
                  { path: 'feature/c.txt', oldPath: null, kind: 'added', binary: false, additions: 3, deletions: 0, truncated: false },
                  { path: 'feature/d.txt', oldPath: null, kind: 'added', binary: false, additions: 5, deletions: 0, truncated: false },
                ]
              : detailIdx === 0
                ? [{ path: 'base.txt', oldPath: null, kind: 'added', binary: false, additions: 1, deletions: 0, truncated: false }]
                : [{ path: 'file' + detailIdx + '.txt', oldPath: null, kind: 'modified', binary: false, additions: 2, deletions: 1, truncated: false }]);
        var insertions = 0;
        var deletions = 0;
        for (var fi = 0; fi < detailFiles.length; fi++) {
          insertions += detailFiles[fi].additions;
          deletions += detailFiles[fi].deletions;
        }
        var baseCommit = window.__historyFixture.commits[detailIdx] || window.__historyFixture.commits[0];
        return Promise.resolve({
          meta: {
            oid: detailOid,
            shortOid: detailOid.slice(-7),
            parents: parents,
            author: baseCommit.author,
            committer: baseCommit.committer,
            subject: baseCommit.subject,
            body: 'Detailed body for commit ' + detailIdx + '.',
            signature: 'unsigned',
          },
          refs: baseCommit.refs,
          stats: { filesChanged: detailFiles.length, insertions: insertions, deletions: deletions },
          files: detailFiles,
          isMerge: isMergeCommit,
          isHead: detailIdx === 0,
          isPushed: detailIdx === 0,
          webUrl: null,
          parentIndex: parentIndex,
        });
      }
      if (command === 'workspace_diff') {
        var spec = (args && args.spec) || {};
        var diffPath = (spec.paths && spec.paths[0]) || 'unknown.txt';
        return Promise.resolve({
          files: [
            {
              path: diffPath,
              oldPath: null,
              change: 'modified',
              binary: false,
              additions: 1,
              deletions: 0,
              truncated: false,
              hunks: [
                {
                  oldStart: 1,
                  oldLines: 1,
                  newStart: 1,
                  newLines: 2,
                  header: '',
                  lines: [
                    { kind: 'context', content: 'before' },
                    { kind: 'added', content: 'after' },
                  ],
                },
              ],
            },
          ],
          truncatedFiles: 0,
        });
      }
      if (command === 'workspace_diff_patch') return Promise.resolve([]);
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

// ---------------------------------------------------------------- T2.4 提交详情

test('详情面板：列表模式点行后展示元数据、统计与文件清单', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  // 切到列表模式，点第 1 行（面板默认在右侧详情位）
  await page.getByTestId('history-mode-list').click();
  await page.locator('#fd-history-list-row-1').click();

  const panel = page.getByTestId('commit-detail-panel');
  await expect(panel).toBeVisible();
  // 元数据与标记（提交 1 非 HEAD → 只有未推送标记）
  await expect(panel).toContainText('feat: commit message 1');
  await expect(panel).toContainText('Detailed body for commit 1.');
  await expect(panel).toContainText('未推送');
  // 统计行 + 文件清单
  await expect(panel).toContainText('1 个文件');
  await expect(panel).toContainText('file1.txt');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('详情面板：点击文件行展开行级 diff', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  await page.getByTestId('history-mode-list').click();
  await page.locator('#fd-history-list-row-1').click();

  const fileRow = page.getByRole('button', { name: /file1\.txt/ });
  await expect(fileRow).toBeVisible();
  await expect(fileRow).toHaveAttribute('aria-expanded', 'false');
  await fileRow.click();
  await expect(fileRow).toHaveAttribute('aria-expanded', 'true');
  // DiffView 渲染出了行级内容（mock 的补丁行）
  await expect(page.getByText('after')).toBeVisible();

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('详情面板：合并提交可切换双父 diff（验收项）', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  await page.getByTestId('history-mode-list').click();
  // 第 5 行是合并提交（fixture 里 i === 5 标记 isMerge）
  await page.locator('#fd-history-list-row-5').click();

  const panel = page.getByTestId('commit-detail-panel');
  await expect(panel).toContainText('feat: commit message 5 (merge)');
  // 默认第一父视角：feature 侧文件
  await expect(panel).toContainText('feature/c.txt');
  await expect(panel).toContainText('feature/d.txt');

  // 切到第二父视角：文件清单换成 main 侧
  await page.getByRole('radio', { name: /第二父/ }).click();
  await expect(panel).toContainText('main/b.txt');
  await expect(panel).not.toContainText('feature/c.txt');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('详情面板：钉住后点击其他行不跟随', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  await page.getByTestId('history-mode-list').click();
  await page.locator('#fd-history-list-row-2').click();
  const panel = page.getByTestId('commit-detail-panel');
  await expect(panel).toContainText('feat: commit message 2');

  // 钉住 → 点第 7 行：选中变化，但详情冻结在提交 2
  await page.getByTestId('commit-detail-pin').click();
  await page.locator('#fd-history-list-row-7').click();
  await expect(panel).toContainText('feat: commit message 2');
  await expect(panel).not.toContainText('feat: commit message 7');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

// ---------------------------------------------------------------- T2.3 筛选与搜索

test('筛选：按作者过滤生效并同步到 URL', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();
  await expect(page.getByTestId('history-loaded-count')).toContainText('15');

  // 打开筛选面板，选一个作者（fixture 里每个作者恰有一条提交）
  await page.getByTestId('history-filter-open').click();
  await page.getByTestId('history-filter-author').selectOption('author3@test.dev');

  // 筛选生效：mock 按作者过滤 → 只剩 1 条；URL 同步出 author 参数
  await expect(page.getByTestId('history-loaded-count')).toContainText('1');
  await expect(page).toHaveURL(/author=author3%40test\.dev/);
  // chip 可见且可单独移除
  await expect(page.getByTestId('history-chip-author')).toBeVisible();

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('筛选后为空时给出空态与清除筛选按钮', async ({ page }) => {
  await page.goto('/#/repo/1/history?q=不存在的关键词');
  await expect(page.getByTestId('history-clear-filters')).toBeVisible();
  await page.getByTestId('history-clear-filters').click();
  // 清除后 URL 上的筛选参数消失，回到完整列表
  await expect(page.getByTestId('history-loaded-count')).toContainText('15');
  await expect(page).not.toHaveURL(/q=/);

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('搜索：关键词高亮 + 上一处/下一处跳转 + 选中', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();

  // "message 1" 命中提交 1、10–19（共 11 条）；防抖 300ms 后请求发出
  await page.getByTestId('history-search-input').fill('message 1');
  await expect(page.getByTestId('history-search-nav')).toBeVisible();
  await expect(page.getByTestId('history-search-count')).toHaveText('0 / 11');

  // 下一处：滚动 + 选中（count 前进，且图模式里出现选中）
  await page.getByTestId('history-search-next').click();
  await expect(page.getByTestId('history-search-count')).toHaveText('1 / 11');
  await page.getByTestId('history-search-next').click();
  await expect(page.getByTestId('history-search-count')).toHaveText('2 / 11');
  await page.getByTestId('history-search-prev').click();
  await expect(page.getByTestId('history-search-count')).toHaveText('1 / 11');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('筛选状态刷新后保持（URL 为真相源）', async ({ page }) => {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();
  await page.getByTestId('history-search-input').fill('message 1');
  await expect(page.getByTestId('history-search-nav')).toBeVisible();

  await page.reload();
  // 关键词还在输入框里，导航与计数仍在（URL ?q=… 复现了筛选）
  await expect(page.getByTestId('history-search-input')).toHaveValue('message 1');
  await expect(page.getByTestId('history-search-nav')).toBeVisible();
  await expect(page).toHaveURL(/q=message(\+|%20)1/);

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

// ---------------------------------------------------------------- T2.5 分支与标签

test('T2.5: 分支页列表（当前置顶/分组/upstream/标签）', async ({ page }) => {
  await page.goto('/#/repo/1/branches');
  await expect(page.getByTestId('branches-current')).toContainText('main');
  await expect(page.getByTestId('branches-current')).toContainText('origin/main');
  await expect(page.getByTestId('branches-switch-feature/xyz')).toBeVisible();
  await expect(page.getByTestId('branches-tag-delete-v1.0.0')).toBeVisible();

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('T2.5: 创建分支 → 切换 → 重命名 → 删除 全流程（验收项）', async ({ page }) => {
  await page.goto('/#/repo/1/branches');
  await expect(page.getByTestId('branches-current')).toContainText('main');

  // ① 创建（并切换）
  await page.getByTestId('branches-create').click();
  await page.getByTestId('branches-create-name').fill('topic/t2.5');
  await page.getByTestId('branches-create-confirm').click();

  // ② 切换到另一分支（三策略对话框 → stash 推荐）
  await page.getByTestId('branches-switch-feature/xyz').click();
  await page.getByTestId('branches-switch-stash').click();
  await expect(page.getByTestId('branches-current')).toContainText('feature/xyz');

  // ③ 重命名（行内"重命名"按钮，第一条非当前本地分支）
  const row = page.getByRole('button', { name: '重命名' }).first();
  await row.click();
  await page.getByTestId('branches-rename-name').fill('topic/renamed');
  await page.getByTestId('branches-rename-confirm').click();

  // ④ 删除（未合并确认链：compare 返回 1 条独有提交 → 强删按钮）
  await page.getByTestId('branches-delete-topic/renamed').click();
  await expect(page.getByText('unmerged work')).toBeVisible();
  await page.getByTestId('branches-delete-confirm-unmerged').click();

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('T2.5: 标签创建与删除', async ({ page }) => {
  await page.goto('/#/repo/1/branches');
  await expect(page.getByTestId('branches-tag-delete-v1.0.0')).toBeVisible();

  await page.getByTestId('branches-tag-create').click();
  await page.getByTestId('branches-tag-name').fill('v1.1.0');
  await page.getByTestId('branches-tag-message').fill('second release');
  await page.getByTestId('branches-tag-confirm').click();
  await expect(page.getByTestId('branches-tag-delete-v1.1.0')).toBeVisible();

  await page.getByTestId('branches-tag-delete-v1.1.0').click();
  await expect(page.getByTestId('branches-tag-delete-v1.1.0')).toHaveCount(0);

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('T2.5: 侧栏分支切换器（搜索 + 快速创建）', async ({ page }) => {
  await page.goto('/#/repo/1/status');
  await expect(page.getByTestId('branch-switcher')).toBeVisible();
  await expect(page.getByTestId('branch-switcher')).toContainText('main');

  await page.getByTestId('branch-switcher').click();
  await page.getByTestId('branch-switcher-filter').fill('feat');
  await expect(page.getByTestId('branch-switcher-item-feature/xyz')).toBeVisible();
  await page.getByTestId('branch-switcher-item-feature/xyz').click();
  await expect(page.getByTestId('branch-switcher')).toContainText('feature/xyz');

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});
