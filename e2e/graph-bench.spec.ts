/**
 * 提交图渲染基准（T2.2 验收取数；默认套件中恒为 skipped）。
 *
 * # 为什么在常规套件里
 *
 * Playwright 对**显式指定的文件**同样套 testMatch 过滤——改名 `.bench.ts`
 * 会导致"想跑也跑不了"。因此本文件保留 `.spec.ts` 后缀留在套件里，
 * 但用环境变量门控：常规 `pnpm test:e2e` 下两个用例**立即 skip**（零成本、
 * 不会红），显式带上 `PERF_BENCH=1` 才真正执行：
 *
 * ```powershell
 * # PowerShell：$env:PERF_BENCH='1'; npx playwright test e2e/graph-bench.spec.ts
 * # Git Bash：  PERF_BENCH=1 npx playwright test e2e/graph-bench.spec.ts
 * ```
 *
 * # 方法（与 docs/PERF-BASELINE.md §4 的记载一致）
 *
 * mock `git_log_page` 一次性返回整段历史（5 万 / 10 万行、8 条泳道），
 * 让前端模型直接承载目标规模，然后测三件事：
 *  1. **冷首屏**：首次 goto 到"图可见 + 行数就位"——含 dev server 模块转换、
 *     浏览器脚本编译等一次性成本，**不代表稳态**；
 *  2. **热首屏**：reload 重走一次——这是任务书"5 万节点首屏 < 1.5s"的口径；
 *  3. **帧率**：rAF 计数 2 秒（静止与滚动各一次），并从 dev 性能面板读取
 *     绘制耗时 / 堆内存（面板自己的 fps 口径偏保守，两者都记录）。
 *
 * 本文件**不断言**性能数值（只在 `coldMs`/`warmMs` 上留存在性断言），
 * 数字看 stdout 的 `[BENCH]` 行，人工誊入 PERF-BASELINE.md。
 */
/* 基准数字靠 console.log 打到 stdout 誊入 PERF-BASELINE.md，是本文件的产出机制 */
/* eslint-disable no-console */
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

const SIZES = [50_000, 100_000] as const;

// 基准对机器性能敏感，不放进必须稳定的门禁套件：默认恒 skip
test.skip(process.env.PERF_BENCH !== '1', '基准用例：带 PERF_BENCH=1 显式运行');

function buildFixture(total: number): string {
  return `
  (function () {
    var total = ${total};
    var commits = [];
    var rows = [];
    var edges = [];
    function oidOf(i) {
      return ('0000000000000000000000000000000000000000' + i.toString(16)).slice(-40);
    }
    for (var i = 0; i < total; i++) {
      var lane = i % 8;
      commits.push({
        oid: oidOf(i), parents: [], author: { name: 'A', email: 'a@b.c', time: 1700000000 - i },
        committer: { name: 'A', email: 'a@b.c', time: 1700000000 - i },
        refs: [], signature: 'unsigned', subject: 'commit ' + i, body: null,
      });
      rows.push({ oid: oidOf(i), row: i, lane: lane, colorIndex: lane, isMerge: false, hidden: false, collapsed: [] });
      if (i > 0) {
        var prevLane = (i - 1) % 8;
        edges.push({ fromOid: oidOf(i), toOid: oidOf(i - 1), fromLane: lane, toLane: prevLane,
          kind: lane === prevLane ? 'straight' : 'branch' });
      }
    }
    window.__benchPage = { commits: commits, layout: { rows: rows, edges: edges, laneCount: 8 }, nextCursor: null };
    window.__TAURI_INTERNALS__ = {
      transformCallback: function () { return 1; },
      unregisterListener: function () {},
      invoke: function (command, args) {
        if (command === 'git_log_page') {
          return Promise.resolve(window.__benchPage);
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
  })();
  `;
}

/** 在页面上用 rAF 数 2 秒帧（作为真函数传入 evaluate；传字符串会拿到函数本身）。 */
async function countFrames(page: Page): Promise<number> {
  return page.evaluate(
    () =>
      new Promise<number>((resolve) => {
        let count = 0;
        const start = performance.now();
        function tick() {
          count += 1;
          if (performance.now() - start < 2000) {
            requestAnimationFrame(tick);
          } else {
            resolve(count);
          }
        }
        requestAnimationFrame(tick);
      }),
  );
}

for (const size of SIZES) {
  test(`前端渲染基准 ${size} 节点`, async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
    await page.addInitScript(buildFixture(size));

    // 冷首屏：含 dev server 模块转换与浏览器脚本编译（一次性成本）
    const coldStart = Date.now();
    await page.goto('/#/repo/1/history');
    await expect(page.getByTestId('graph-overlay')).toBeVisible();
    await expect(page.getByTestId('history-loaded-count')).toContainText('已加载');
    const coldMs = Date.now() - coldStart;

    // 热首屏：reload 重走一遍（任务书"5 万节点首屏 < 1.5s"的口径）
    await page.waitForTimeout(500);
    const warmStart = Date.now();
    await page.reload();
    await expect(page.getByTestId('graph-overlay')).toBeVisible();
    await expect(page.getByTestId('history-loaded-count')).toContainText('已加载');
    const warmMs = Date.now() - warmStart;

    // 打开 dev 性能面板读取绘制指标（面板自身口径偏保守的 fps 也一并记录）
    await page.getByTestId('history-perf-toggle').click();
    await expect(page.getByTestId('graph-perf-panel')).toBeVisible();

    const canvas = page.getByTestId('graph-hit-layer');
    const box = await canvas.boundingBox();
    expect(box).not.toBeNull();
    await page.mouse.move(box!.x + 200, box!.y + box!.height / 2);

    const framesIdle = await countFrames(page);
    for (let i = 0; i < 30; i += 1) {
      await page.mouse.wheel(0, 120);
    }
    for (let i = 0; i < 30; i += 1) {
      await page.mouse.wheel(0, -120);
    }
    const framesScrolled = await countFrames(page);

    const panelText = await page.getByTestId('graph-perf-panel').innerText();
    console.log(`[BENCH ${size}] coldMs=${coldMs} warmMs=${warmMs}`);
    console.log(
      `[BENCH ${size}] fps-idle=${(framesIdle / 2).toFixed(1)} fps-scroll=${(framesScrolled / 2).toFixed(1)}`,
    );
    console.log(`[BENCH ${size}] panel=${panelText.replace(/[\r\n]+/g, ' | ')}`);

    // 存在性断言：数字不进门禁，但"基准页面能用"必须成立
    expect(warmMs).toBeGreaterThan(0);
    expect(framesIdle).toBeGreaterThan(0);
  });
}
