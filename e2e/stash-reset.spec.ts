/**
 * T2.10 第 2 条的 E2E 补齐：stash 全流程 + reset 影响预览与取消。
 *
 *   1. stash：save → 列表出现 → 展开 diff → apply（保留条目）→ drop（确认框）→ 列表清空；
 *   2. reset：选中提交 → prepare 出计划（预览被丢弃的提交）→ **取消** → execute 从未被调用。
 *
 * mock 方式与 loop.spec.ts / history.spec.ts 一致：应用脚本运行前注入
 * `window.__TAURI_INTERNALS__`。界面与契约的闭环在浏览器里验证；真实 git
 * 语义由 `crates/services/tests/*.rs` 与 `crates/commands/tests/write_ops_safety_net.rs`
 * 在真实仓库上保证（分工见 docs/CODING_STYLE.md §3.6）。
 *
 * 每个 test 结尾断言 `window.__errs` 为空。
 */
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

async function pinLanguage(page: Page): Promise<void> {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
}

/** 断言页面自身没有未捕获错误。 */
async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

/** stash 面板的 mock：内存里的 stash 栈，save/apply/pop/drop/clear 都改它。 */
const STASH_FIXTURE = `
  window.__stashes = [];
  window.__stashSeq = 0;
  window.__calls = [];
  function stashEntry(oid, message) {
    return { index: 0, oid: oid, baseOid: 'b'.repeat(40), message: message,
             createdAt: 1700000000, includesUntracked: false, untrackedOid: null };
  }
  function reindexStashes() {
    for (var i = 0; i < window.__stashes.length; i++) window.__stashes[i].index = i;
  }
`;

const STASH_MOCK = `
  ${STASH_FIXTURE}
  var listeners = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === 'workspace_status') {
        return Promise.resolve({
          branch: { oid: 'a'.repeat(40), head: 'main', detached: false, upstream: null, ahead: 0, behind: 0 },
          operation: 'none',
          staged: [],
          unstaged: [{ path: 'a.txt', kind: 'ordinary', indexStatus: '.', worktreeStatus: 'M', isBinary: false, isLfs: false, isSubmodule: false, sizeBytes: 8 }],
          untracked: [],
          conflicted: [],
          ignored: [],
          ignoredCount: null
        });
      }
      if (command === 'git_stash_list') {
        reindexStashes();
        return Promise.resolve(window.__stashes);
      }
      if (command === 'git_stash_save') {
        window.__calls.push({ command: command });
        window.__stashSeq += 1;
        var entry = stashEntry('s' + window.__stashSeq + '0'.repeat(36), 'WIP on main: stash ' + window.__stashSeq);
        window.__stashes.unshift(entry);
        reindexStashes();
        return Promise.resolve({ stashed: true, entry: entry, snapshotId: 100 + window.__stashSeq });
      }
      if (command === 'git_stash_show') {
        var shown = window.__stashes.filter(function (s) { return s.index === args.index; })[0] || null;
        return Promise.resolve({
          entry: shown,
          diff: { files: [{ path: 'a.txt', added: 1, removed: 1, binary: false, truncated: false,
            hunks: [{ header: '@@ -1 +1 @@', oldStart: 1, oldLines: 1, newStart: 1, newLines: 1,
              lines: [{ kind: 'removed', content: 'old', oldNo: 1, newNo: null },
                      { kind: 'added', content: 'new', oldNo: null, newNo: 1 }] }] }] },
          untracked: null
        });
      }
      if (command === 'git_stash_apply' || command === 'git_stash_pop') {
        window.__calls.push({ command: command, index: args.index });
        if (command === 'git_stash_pop') {
          window.__stashes = window.__stashes.filter(function (s) { return s.index !== args.index; });
          reindexStashes();
        }
        return Promise.resolve({ conflicts: [], snapshotId: 200 });
      }
      if (command === 'git_stash_drop') {
        window.__calls.push({ command: command, index: args.index });
        var dropped = window.__stashes.filter(function (s) { return s.index === args.index; });
        window.__stashes = window.__stashes.filter(function (s) { return s.index !== args.index; });
        reindexStashes();
        return Promise.resolve({ dropped: dropped });
      }
      if (command === 'git_stash_clear') {
        window.__calls.push({ command: command });
        var all = window.__stashes.slice();
        window.__stashes = [];
        return Promise.resolve({ dropped: all });
      }
      if (command === 'repo_recent_list') return Promise.resolve([
        { id: 1, path: '/tmp/repo', name: 'repo', defaultBranch: 'main', lastOpenedAt: 1, createdAt: 1, isOpen: true }
      ]);
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

test('stash 全流程：保存 → 展开 diff → 应用（保留）→ 丢弃（确认框）', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(STASH_MOCK);
  await page.goto('/#/repo/1/status');

  const panel = page.getByTestId('stash-panel');
  await expect(panel).toBeVisible();
  // 初始：没有条目（空态文案 + 无清除按钮）
  await expect(page.getByTestId('stash-save')).toBeEnabled();
  await expect(page.getByTestId('stash-clear')).toHaveCount(0);

  // ① save：列表出现一条
  await page.getByTestId('stash-save').click();
  await expect(page.getByTestId('stash-list')).toBeVisible();
  await expect(page.getByTestId('stash-toggle-0')).toBeVisible();
  await expect(page.getByTestId('stash-apply-0')).toBeVisible();
  await expect(page.getByTestId('stash-drop-0')).toBeVisible();
  // 清除按钮随条目出现
  await expect(page.getByTestId('stash-clear')).toBeVisible();

  // ② 展开：显示相对 base 的文件清单（stash_show 被调）
  await page.getByTestId('stash-toggle-0').click();
  await expect(page.getByTestId('stash-files')).toBeVisible();
  await expect(page.getByTestId('stash-files')).toContainText('a.txt');

  // ③ apply：无冲突、条目保留（apply 不删条目）
  await page.getByTestId('stash-apply-0').click();
  await expect(page.getByTestId('stash-conflict-banner')).toHaveCount(0);
  await expect(page.getByTestId('stash-toggle-0')).toBeVisible();
  const calls = await page.evaluate(
    (): readonly { command: string; index?: number }[] => window.__calls ?? [],
  );
  expect(calls.some((call) => call.command === 'git_stash_apply')).toBe(true);

  // ④ drop：确认框 → 确认 → 列表清空
  await page.getByTestId('stash-drop-0').click();
  const dialog = page.getByTestId('stash-drop-dialog');
  await expect(dialog).toBeVisible();
  await page.getByTestId('stash-drop-confirm').click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId('stash-toggle-0')).toHaveCount(0);

  await expectNoPageErrors(page);
});

// ---------------------------------------------------------------- reset 预览与取消

const RESET_FIXTURE = `
  function sig(n) { return { name: 'A' + n, email: 'a' + n + '@t.dev', time: 1700000000 }; }
  var commits = [];
  var rows = [];
  var edges = [];
  for (var i = 0; i < 5; i++) {
    var oid = ('000000000000000000000000000000000000000' + i).slice(-40);
    commits.push({ oid: oid, parents: i > 0 ? [('000000000000000000000000000000000000000' + (i - 1)).slice(-40)] : [],
      author: sig(i), committer: sig(i), refs: i === 0 ? ['HEAD -> main'] : [], signature: 'unsigned',
      subject: 'feat: commit ' + i, body: null });
    rows.push({ oid: oid, row: i, lane: 0, colorIndex: 0, isMerge: false, hidden: false, collapsed: [] });
    if (i > 0) edges.push({ fromOid: oid, toOid: ('000000000000000000000000000000000000000' + (i - 1)).slice(-40),
      fromLane: 0, toLane: 0, kind: 'straight' });
  }
  window.__resetPage = { commits: commits, layout: { rows: rows, edges: edges, laneCount: 1 }, nextCursor: null };
  window.__resetPrepared = null;
`;

const RESET_MOCK = `
  ${RESET_FIXTURE}
  var listeners = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === 'git_log_page') {
        return Promise.resolve(window.__resetPage);
      }
      if (command === 'git_branch_list') return Promise.resolve([]);
      if (command === 'git_log_authors') return Promise.resolve([]);
      if (command === 'git_reflog') return Promise.resolve([]);
      if (command === 'git_commit_detail') {
        var doid = (args && args.oid) || '0'.repeat(40);
        return Promise.resolve({
          meta: { oid: doid, shortOid: doid.slice(-7), parents: [],
                  author: { name: 'A', email: 'a@t.dev', time: 1700000000 },
                  committer: { name: 'A', email: 'a@t.dev', time: 1700000000 },
                  subject: 'feat: commit', body: null, signature: 'unsigned' },
          refs: [], stats: { filesChanged: 0, insertions: 0, deletions: 0 },
          files: [], isMerge: false, isHead: false, isPushed: false,
          webUrl: null, parentIndex: 0
        });
      }
      if (command === 'git_reset_prepare') {
        var mode = ((args || {}).spec || {}).mode || 'mixed';
        var plan = {
          planId: 'plan-e2e-1', mode: mode,
          targetOid: '0000000000000000000000000000000000000002',
          targetSubject: 'feat: commit 2',
          headBefore: '0000000000000000000000000000000000000000',
          discarded: [
            { oid: '0000000000000000000000000000000000000003', subject: 'feat: commit 3', authorTime: 1700000003 },
            { oid: '0000000000000000000000000000000000000004', subject: 'feat: commit 4', authorTime: 1700000004 }
          ],
          discardedTruncated: false, discardedCount: 2,
          lostStaged: [], lostWorktree: [], untrackedToRemove: [],
          remote: { upstream: null, notOnRemote: 2 },
          requiresConfirmation: mode === 'hard',
          confirmationWord: mode === 'hard' ? 'reset' : undefined,
          snapshotRequired: true
        };
        window.__resetPrepared = plan;
        return Promise.resolve(plan);
      }
      if (command === 'git_reset_execute') {
        window.__resetExecuted = (window.__resetExecuted || 0) + 1;
        return Promise.resolve({ mode: 'mixed', headBefore: '0'.repeat(40), headAfter: '2'.repeat(40),
          discardedCount: 2, snapshotId: 300 });
      }
      if (command === 'repo_recent_list') return Promise.resolve([
        { id: 1, path: '/tmp/repo', name: 'repo', defaultBranch: 'main', lastOpenedAt: 1, createdAt: 1, isOpen: true }
      ]);
      if (command === 'app_version') return Promise.resolve({ version: '0.0.1', gitDescribe: null });
      if (command === 'settings_get') return Promise.resolve(null);
      if (command === 'settings_all') return Promise.resolve({});
      if (command === 'logs_tail') return Promise.resolve([]);
      if (command === 'workspace_status') return Promise.resolve({ branch: { oid: 'a'.repeat(40), head: 'main', detached: false, upstream: null, ahead: 0, behind: 0 }, operation: 'none', staged: [], unstaged: [], untracked: [], conflicted: [], ignored: [], ignoredCount: null });
      if (command === 'plugin:event|unlisten') return Promise.resolve(null);
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

test('reset 影响预览：计划摊开被丢弃的提交；取消后 execute 从未被调用', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(RESET_MOCK);
  await page.goto('/#/repo/1/history');

  // 列表模式点第 1 行：选中提交（detailOid 是 reset 的目标）
  await page.getByTestId('history-mode-list').click();
  await page.locator('#fd-history-list-row-1').click();
  await expect(page.getByTestId('history-ops-reset')).toBeEnabled();

  // prepare：计划对话框出现，摊开将被丢弃的提交
  await page.getByTestId('history-ops-reset').click();
  const dialog = page.getByTestId('reset-plan-dialog');
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId('reset-plan-commits')).toContainText('feat: commit 3');
  await expect(page.getByTestId('reset-plan-commits')).toContainText('feat: commit 4');

  // 取消：对话框关闭，execute 一次都没被调
  await dialog.getByRole('button', { name: '取消' }).click();
  await expect(dialog).toHaveCount(0);
  const executed = await page.evaluate(() => window.__resetExecuted ?? 0);
  expect(executed, '取消后不应执行 reset').toBe(0);

  await expectNoPageErrors(page);
});

test('reset --hard 需要确认词：预览给出输入框，输入后才能执行', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(RESET_MOCK);
  await page.goto('/#/repo/1/history');

  await page.getByTestId('history-mode-list').click();
  await page.locator('#fd-history-list-row-1').click();

  // 切到 hard 模式再 prepare（label 全串是"硬重置（丢弃工作区）"）
  await page.getByRole('radio', { name: '硬重置（丢弃工作区）' }).click();
  await page.getByTestId('history-ops-reset').click();
  const dialog = page.getByTestId('reset-plan-dialog');
  await expect(dialog).toBeVisible();

  // 确认词输入框出现；不输入时确认按钮禁用（R7 的第三道闸）
  const input = page.getByTestId('reset-confirm-input');
  await expect(input).toBeVisible();
  await expect(page.getByTestId('reset-confirm')).toBeDisabled();

  // 输入确认词 → 按钮可用 → 执行一次
  await input.fill('reset');
  await expect(page.getByTestId('reset-confirm')).toBeEnabled();
  await page.getByTestId('reset-confirm').click();
  await expect(dialog).toHaveCount(0);
  const executed = await page.evaluate(() => window.__resetExecuted ?? 0);
  expect(executed).toBe(1);

  await expectNoPageErrors(page);
});
