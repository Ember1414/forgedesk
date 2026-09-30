import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

/** 交互方案的截图目录（供人工审批与回归对照；不进版本库）。 */
const VISUAL_DIR = join(process.cwd(), 'test-results', 'rebase-visual');
mkdirSync(VISUAL_DIR, { recursive: true });

/**
 * T3.6 交互级验收：拖拽式 Rebase 面板。
 *
 * # 为什么是 mock IPC 而不是真实 rebase 仓库
 *
 * 与 conflict.spec.ts 同一取舍：真实执行（todo 注入、三结局、edit 恢复）由
 * Rust 集成测试在真实仓库上覆盖（`crates/services/tests/rebase.rs`），这里用
 * mock 验证**产品自己的那半条链路**：
 *
 *   历史页框选/右键 → 面板列出区间提交 → 编辑动作与顺序 → 预览 →
 *   确认执行（发给后端的 steps 与生成预览的 steps 逐字一致）→ 三结局的界面出路。
 *
 * "新历史与 preview 预测一致"在 E2E 层的等价断言是：**execute 收到的 steps
 * 与最后一次 preview 的 steps 完全相同**——界面不可能在执行时偷偷换一份计划。
 *
 * mock 的字段一律 camelCase（docs/API.md §1 契约）：`Option` 是 **null** 不是 undefined。
 */

/** oid：短前缀 + 补零（前 7 位在界面上可见，便于断言）。 */
const MOCK_SCRIPT = `
  window.__errs = [];
  window.__rebaseCalls = [];
  function oid(tag) { return tag.padEnd(40, '0'); }
  var REBASE_COMMITS = [
    { oid: oid('base'), subject: 'base', parents: [], author: 'Ada', authorTime: 1700000000 },
    { oid: oid('c1'), subject: 'one', parents: [oid('base')], author: 'Ada', authorTime: 1700000100 },
    { oid: oid('c2'), subject: 'two', parents: [oid('c1')], author: 'Ada', authorTime: 1700000200 },
    { oid: oid('c3'), subject: 'three', parents: [oid('c2')], author: 'Ada', authorTime: 1700000300 }
  ];
  var commitByOid = {};
  REBASE_COMMITS.forEach(function (c) { commitByOid[c.oid] = c; });
  function signature(name, time) { return { name: name, email: 'ada@example.com', time: time }; }
  // 图序：新 → 旧（row 0 = c3）
  var ORDER = [REBASE_COMMITS[3], REBASE_COMMITS[2], REBASE_COMMITS[1], REBASE_COMMITS[0]];
  var commits = ORDER.map(function (c, i) {
    return {
      oid: c.oid, parents: c.parents,
      author: signature(c.author, c.authorTime), committer: signature(c.author, c.authorTime),
      refs: i === 0 ? ['HEAD -> main'] : [], signature: 'unsigned', subject: c.subject, body: null
    };
  });
  var rows = ORDER.map(function (c, i) {
    return { oid: c.oid, row: i, lane: 0, colorIndex: 0, isMerge: false, hidden: false, collapsed: [] };
  });
  var edges = [];
  for (var i = 1; i < ORDER.length; i++) {
    edges.push({ fromOid: ORDER[i - 1].oid, toOid: ORDER[i].oid, fromLane: 0, toLane: 0, kind: 'straight' });
  }
  // 区间清单：git rev-list base..head（不含 base，从旧到新）
  var RANGE = [REBASE_COMMITS[1], REBASE_COMMITS[2], REBASE_COMMITS[3]];

  function rebasePreview(steps) {
    var surviving = [];
    var dropped = [];
    var reworded = [];
    var squashed = [];
    steps.forEach(function (s) {
      var subject = (commitByOid[s.oid] || {}).subject || '';
      if (s.action === 'pick' || s.action === 'edit') {
        surviving.push({ oid: s.oid, subject: subject });
      } else if (s.action === 'reword') {
        reworded.push(s.oid);
        surviving.push({ oid: s.oid, subject: s.newMessage || subject });
      } else if (s.action === 'squash') {
        var last = surviving[surviving.length - 1];
        if (last) { last.subject = last.subject + '\\n\\n' + subject; squashed.push(s.oid + ' -> ' + last.oid); }
      } else if (s.action === 'fixup') {
        var prev = surviving[surviving.length - 1];
        if (prev) { squashed.push(s.oid + ' -> (fixup)'); }
      } else if (s.action === 'drop') {
        dropped.push(s.oid);
      }
    });
    var todo = steps.map(function (s) {
      return s.action + ' ' + s.oid.slice(0, 7) + ' ' + ((commitByOid[s.oid] || {}).subject || '');
    }).join('\\n');
    return {
      surviving: surviving, dropped: dropped, reworded: reworded, squashed: squashed,
      affectedCount: steps.length, touchesPushed: false, todoText: todo + '\\n'
    };
  }

  window.__TAURI_INTERNALS__ = {
    transformCallback: function () { return 1; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === 'plugin:event|listen') return Promise.resolve(1);
      if (command === 'plugin:event|unlisten') return Promise.resolve(null);
      if (command === 'app_version') return Promise.resolve({ version: '0.0.1', gitDescribe: null });
      if (command === 'repo_recent_list') {
        return Promise.resolve([{ record: { id: 1, path: '/tmp/repo', name: 'repo' }, isOpen: true }]);
      }
      if (command === 'settings_get' || command === 'settings_all' || command === 'settings_set') {
        return Promise.resolve(null);
      }
      if (command === 'git_log_page') {
        return Promise.resolve({ commits: commits, layout: { rows: rows, edges: edges, laneCount: 1 }, nextCursor: null });
      }
      if (command === 'git_log_authors') {
        return Promise.resolve([{ name: 'Ada', email: 'ada@example.com', commitCount: 4 }]);
      }
      if (command === 'git_branch_list') {
        return Promise.resolve([
          { name: 'main', isRemote: false, isHead: true, target: oid('c3'), upstream: null, ahead: null, behind: null, upstreamGone: false }
        ]);
      }
      if (command === 'git_tag_list') return Promise.resolve([]);
      if (command === 'git_commit_detail') {
        var detailOid = (args && args.oid) || '';
        var c = commitByOid[detailOid] || REBASE_COMMITS[3];
        return Promise.resolve({
          meta: {
            oid: c.oid, shortOid: c.oid.slice(0, 7), parents: c.parents,
            author: signature(c.author, c.authorTime), committer: signature(c.author, c.authorTime),
            subject: c.subject, body: null, signature: 'unsigned'
          },
          refs: [], stats: { filesChanged: 1, insertions: 1, deletions: 0 },
          files: [{ path: 'a.txt', oldPath: null, kind: 'modified', binary: false, additions: 1, deletions: 0, truncated: false }],
          isMerge: false, isHead: false, isPushed: false, webUrl: null, parentIndex: 0
        });
      }
      if (command === 'workspace_diff') return Promise.resolve({ files: [] });
      if (command === 'workspace_diff_patch') return Promise.resolve([]);
      if (command === 'workspace_status') {
        return Promise.resolve({
          branch: { oid: oid('c3'), head: 'main', detached: false, upstream: null, ahead: null, behind: null },
          operation: 'none', staged: [], unstaged: [], untracked: [], conflicted: [], ignored: [], ignoredCount: null
        });
      }
      if (command === 'git_rebase_range') {
        // 与真实语义一致：base..head 之间的提交（不含 base，从旧到新）
        var rangeBase = (args && args.base) || '';
        var rangeHead = (args && args.head) || '';
        var baseIdx = -1;
        var headIdx = -1;
        for (var ri = 0; ri < REBASE_COMMITS.length; ri++) {
          if (REBASE_COMMITS[ri].oid === rangeBase) baseIdx = ri;
          if (REBASE_COMMITS[ri].oid === rangeHead) headIdx = ri;
        }
        var picked = [];
        for (var rj = baseIdx + 1; rj <= headIdx; rj++) picked.push(REBASE_COMMITS[rj]);
        return Promise.resolve(picked.map(function (c) {
          return { oid: c.oid, parents: c.parents, subject: c.subject, author: c.author, authorTime: c.authorTime };
        }));
      }
      if (command === 'git_rebase_preview_only') {
        window.__rebaseCalls.push({ command: command, steps: args.spec.steps });
        return Promise.resolve(rebasePreview(args.spec.steps));
      }
      if (command === 'git_rebase_execute') {
        window.__rebaseCalls.push({ command: command, steps: args.spec.steps });
        if (window.__rebaseExecuteOutcome) return Promise.resolve(window.__rebaseExecuteOutcome);
        return Promise.resolve({ kind: 'completed', oid: oid('newhead'), snapshotId: 1 });
      }
      if (command === 'git_rebase_continue_edit') {
        window.__rebaseCalls.push({ command: command });
        if (window.__rebaseContinueOutcome) return Promise.resolve(window.__rebaseContinueOutcome);
        return Promise.resolve({ kind: 'completed', oid: oid('newhead2'), snapshotId: null });
      }
      if (command === 'git_conflict_state') {
        return Promise.resolve({
          opKind: null, opInProgress: false, currentStep: null, totalSteps: null,
          headName: null, intoBranch: null, files: [], canContinue: false, canAbort: false, canSkip: false
        });
      }
      if (command === 'git_conflict_abort') {
        window.__rebaseCalls.push({ command: command });
        return Promise.resolve({ headOid: oid('c3'), headRef: 'main', snapshotId: 9 });
      }
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

/** 图上的节点坐标（与 history.spec.ts 的既有取值一致：行高 28、首行中心 y=14）。 */
const NODE_X = 30;
const ROW_HEIGHT = 28;
const ROW_TOP = 14;

/** 夹具的完整 oid（与 mock 的 `oid(tag)` 一致：tag + 补零到 40 位）。 */
const OID = {
  base: `base${'0'.repeat(36)}`,
  c1: `c1${'0'.repeat(38)}`,
  c2: `c2${'0'.repeat(38)}`,
  c3: `c3${'0'.repeat(38)}`,
} as const;

async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

async function clickRow(
  page: Page,
  row: number,
  modifiers: { readonly shift?: boolean } = {},
): Promise<void> {
  const box = await page.getByTestId('graph-hit-layer').boundingBox();
  expect(box).not.toBeNull();
  if (modifiers.shift === true) {
    await page.keyboard.down('Shift');
  }
  await page.mouse.click(box!.x + NODE_X, box!.y + ROW_TOP + row * ROW_HEIGHT);
  if (modifiers.shift === true) {
    await page.keyboard.up('Shift');
  }
  await page.waitForTimeout(120);
}

async function rightClickRow(page: Page, row: number): Promise<void> {
  const box = await page.getByTestId('graph-hit-layer').boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.click(box!.x + NODE_X, box!.y + ROW_TOP + row * ROW_HEIGHT, { button: 'right' });
  await expect(page.getByTestId('graph-context-menu')).toBeVisible();
}

async function openHistory(page: Page): Promise<void> {
  await page.goto('/#/repo/1/history');
  await expect(page.getByTestId('graph-overlay')).toBeVisible();
}

/** 框选 c1..c3（行 2 点选 + Shift 点行 0）→ 右键 → 打开面板。 */
async function openPanelWithRange(page: Page): Promise<void> {
  await openHistory(page);
  await clickRow(page, 2);
  await clickRow(page, 0, { shift: true });
  await rightClickRow(page, 0);
  await page.getByTestId('graph-menu-organize').click();
  await expect(page.getByTestId('rebase-step-list')).toBeVisible();
  await expect(page.getByTestId('rebase-preview')).toBeVisible();
}

/** 面板上的步骤行（从旧到新的顺序）。 */
function stepRows(page: Page) {
  return page.getByTestId('rebase-step-list').getByRole('listitem');
}

async function lastCalls(page: Page) {
  return page.evaluate(() => window.__rebaseCalls ?? []);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(MOCK_SCRIPT);
});

test('框选区间 → 右键整理 → 面板列出区间提交并给出预览', async ({ page }) => {
  await openPanelWithRange(page);

  // 三行按从旧到新列出（one / two / three）
  const rows = stepRows(page);
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText('one');
  await expect(rows.nth(1)).toContainText('two');
  await expect(rows.nth(2)).toContainText('three');
  await expect(rows.nth(0)).toContainText('Ada');

  await expect(page.getByTestId('rebase-preview')).toContainText('将重写 3 个提交');
  // 审批用截图：拖拽清单 + 实时预览 + 底部 todo/执行（T3.6 交互方案）
  await page.screenshot({ path: join(VISUAL_DIR, '01-panel.png') });
  await expectNoPageErrors(page);
});

test('Alt+↓ 重排后执行：发给后端的 steps 与最后一次预览逐字一致', async ({ page }) => {
  await openPanelWithRange(page);

  await stepRows(page).nth(0).click();
  await page.keyboard.press('Alt+ArrowDown');
  await expect(stepRows(page).nth(0)).toContainText('two');
  await expect(stepRows(page).nth(1)).toContainText('one');

  await page.getByTestId('rebase-execute').click();
  await expect(page.getByTestId('rebase-confirm-execute')).toBeVisible();
  // 审批用截图：确认对话框（重写数量 + force-with-lease + 快照承诺）
  await page.screenshot({ path: join(VISUAL_DIR, '02-confirm.png') });
  await page.getByTestId('rebase-confirm-execute').click();

  await expect(page.getByTestId('rebase-outcome')).toContainText('重写完成');
  const calls = await lastCalls(page);
  const previews = calls.filter((call) => call.command === 'git_rebase_preview_only');
  const executes = calls.filter((call) => call.command === 'git_rebase_execute');
  expect(executes).toHaveLength(1);
  // 界面在执行时不能换一份计划：steps 必须与生成预览的那一份相同
  expect(executes[0]?.steps).toEqual(previews[previews.length - 1]?.steps);
  expect(executes[0]?.steps?.map((step) => step.action)).toEqual(['pick', 'pick', 'pick']);
  expect(executes[0]?.steps?.map((step) => step.oid)).toEqual([OID.c2, OID.c1, OID.c3]);
  await expect(page.getByTestId('rebase-snapshot-hint')).toContainText('#1');
  await expectNoPageErrors(page);
});

test('squash 三个提交：预览显示并入，执行发出的动作序列正确', async ({ page }) => {
  await openPanelWithRange(page);

  // 第二、三条改成 squash（第一条保持 pick）
  await page.getByTestId(`rebase-action-${OID.c2}`).click();
  await page.getByTestId('rebase-action-option-squash').click();
  await page.getByTestId(`rebase-action-${OID.c3}`).click();
  await page.getByTestId('rebase-action-option-squash').click();

  await expect(page.getByTestId(`rebase-merged-${OID.c1}`)).toContainText('并入 2 个');

  await page.getByTestId('rebase-execute').click();
  await page.getByTestId('rebase-confirm-execute').click();
  await expect(page.getByTestId('rebase-outcome')).toContainText('重写完成');

  const calls = await lastCalls(page);
  const execute = calls.find((call) => call.command === 'git_rebase_execute');
  expect(execute?.steps?.map((step) => step.action)).toEqual(['pick', 'squash', 'squash']);
  await expectNoPageErrors(page);
});

test('drop 中间提交：预览划掉，执行发出的动作序列正确', async ({ page }) => {
  await openPanelWithRange(page);

  await page.getByTestId(`rebase-action-${OID.c2}`).click();
  await page.getByTestId('rebase-action-option-drop').click();
  await expect(page.getByTestId('rebase-preview')).toContainText('已丢弃');

  await page.getByTestId('rebase-execute').click();
  await page.getByTestId('rebase-confirm-execute').click();
  await expect(page.getByTestId('rebase-outcome')).toContainText('重写完成');

  const calls = await lastCalls(page);
  const execute = calls.find((call) => call.command === 'git_rebase_execute');
  expect(execute?.steps?.map((step) => step.action)).toEqual(['pick', 'drop', 'pick']);
  await expectNoPageErrors(page);
});

test('reword：输入的新信息随执行发送，且预览先用新信息展示', async ({ page }) => {
  await openPanelWithRange(page);

  await page.getByTestId(`rebase-action-${OID.c1}`).click();
  await page.getByTestId('rebase-action-option-reword').click();
  await page.getByTestId(`rebase-message-${OID.c1}`).fill('one rewritten');
  await expect(page.getByTestId('rebase-preview')).toContainText('one rewritten');

  await page.getByTestId('rebase-execute').click();
  await page.getByTestId('rebase-confirm-execute').click();
  await expect(page.getByTestId('rebase-outcome')).toContainText('重写完成');

  const calls = await lastCalls(page);
  const execute = calls.find((call) => call.command === 'git_rebase_execute');
  expect(execute?.steps?.[0]).toEqual({
    oid: OID.c1,
    action: 'reword',
    newMessage: 'one rewritten',
  });
  await expectNoPageErrors(page);
});

test('单提交快捷入口（reword 预设）直接以该动作打开面板', async ({ page }) => {
  await openHistory(page);
  // 直接右键 c2（不预选）：区间 = c1..c2，只含 c2 自己
  await rightClickRow(page, 1);
  await page.getByTestId('graph-menu-reword-commit').click();

  await expect(page.getByTestId('rebase-step-list')).toBeVisible();
  await expect(stepRows(page)).toHaveCount(1);
  await expect(stepRows(page).nth(0)).toContainText('two');
  // preset 生效：该行已切到 reword，并出现信息输入框
  await expect(page.getByTestId(`rebase-message-${OID.c2}`)).toBeVisible();
  await expectNoPageErrors(page);
});

test('冲突暂停 → 中止并还原走 conflict_abort', async ({ page }) => {
  await page.addInitScript(() => {
    window.__rebaseExecuteOutcome = {
      kind: 'pausedConflict',
      conflicts: ['src/a.ts'],
      snapshotId: 5,
    };
  });
  await openPanelWithRange(page);

  await page.getByTestId('rebase-execute').click();
  await page.getByTestId('rebase-confirm-execute').click();

  const outcome = page.getByTestId('rebase-outcome');
  await expect(outcome).toContainText('已暂停：需要解决冲突');
  await expect(outcome).toContainText('src/a.ts');
  await expect(page.getByTestId('rebase-goto-conflict')).toBeVisible();
  // 审批用截图：冲突暂停的出口（去冲突页 / 中止并还原 + 快照提示）
  await page.screenshot({ path: join(VISUAL_DIR, '03-paused-conflict.png') });

  await page.getByTestId('rebase-abort').click();
  await expect
    .poll(async () => (await lastCalls(page)).map((call) => call.command))
    .toContain('git_conflict_abort');
  await expectNoPageErrors(page);
});

test('edit 暂停 → 修改完成继续走 continue_edit', async ({ page }) => {
  await page.addInitScript(() => {
    window.__rebaseExecuteOutcome = {
      kind: 'pausedEdit',
      oid: 'c2000000000000000000000000000000000000000',
      snapshotId: 7,
    };
  });
  await openPanelWithRange(page);

  await page.getByTestId('rebase-execute').click();
  await page.getByTestId('rebase-confirm-execute').click();
  await expect(page.getByTestId('rebase-outcome')).toContainText('已暂停：请修改内容后点击继续');

  await page.getByTestId('rebase-continue-edit').click();
  await expect
    .poll(async () => (await lastCalls(page)).map((call) => call.command))
    .toContain('git_rebase_continue_edit');
  await expect(page.getByTestId('rebase-outcome')).toContainText('重写完成');
  await expectNoPageErrors(page);
});
