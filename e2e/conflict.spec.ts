import { expect, test, type Page } from '@playwright/test';

/**
 * T3.1 交互级验收：冲突页（状态机视图）。
 *
 * # 为什么是 mock IPC 而不是真实冲突仓库
 *
 * 与 sync.spec.ts 同一取舍：后端行为（状态采集、stage 校验、abort 快照）由
 * Rust 集成测试在真实仓库上覆盖（`crates/services/tests/conflict.rs`），这里用
 * mock 的 IPC 验证**产品自己的那半条链路**：
 *
 *   状态查询 → 界面正确落出操作横幅 / 文件列表 / 按钮可用性 → 用户动作真的
 *   发出对应的命令（参数正确）→ `__errs` 为空。
 *
 * mock 的字段一律 camelCase（docs/API.md §1 契约）：`Option` 是 **null**
 * 不是 undefined（T2.10 的 serde 教训）。
 */
const MOCK_SCRIPT = `
  window.__errs = [];
  window.__conflictCalls = [];
  const conflictFiles = [
    { path: "src/a.ts", kind: "text", base: { size: 5, isBinary: false, encodingHint: "utf-8", content: "base" },
      ours: { size: 5, isBinary: false, encodingHint: "utf-8", content: "ours" },
      theirs: { size: 6, isBinary: false, encodingHint: "utf-8", content: "theirs" }, worktreeExists: true }
  ];
  let resolved = false;
  function conflictState() {
    if (!resolved) {
      return { opKind: "merge", opInProgress: true, currentStep: null, totalSteps: null,
        headName: null, intoBranch: "main", files: conflictFiles,
        canContinue: false, canAbort: true, canSkip: false };
    }
    return { opKind: "merge", opInProgress: true, currentStep: null, totalSteps: null,
      headName: null, intoBranch: "main", files: [],
      canContinue: true, canAbort: true, canSkip: false };
  }
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { return 1; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "plugin:event|listen") return Promise.resolve(1);
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_all" || command === "settings_get") return Promise.resolve(null);
      if (command === "settings_set") return Promise.resolve(null);
      if (command === "git_branch_list") return Promise.resolve([]);
      if (command === "workspace_status") return Promise.resolve({
        branch: { oid: "a1b2c3", head: "main", detached: false, upstream: null, ahead: null, behind: null },
        operation: "merge", staged: [], unstaged: [], untracked: [], conflicted: ["src/a.ts"], ignored: [], ignoredCount: null
      });
      if (command === "git_conflict_state") return Promise.resolve(conflictState());
      if (command === "git_conflict_file_detail") {
        return Promise.resolve({ path: "src/a.ts", kind: "text",
          base: { size: 5, isBinary: false, encodingHint: "utf-8", content: "base" },
          ours: { size: 5, isBinary: false, encodingHint: "utf-8", content: "ours" },
          theirs: { size: 6, isBinary: false, encodingHint: "utf-8", content: "theirs" },
          worktreeExists: true, eol: "lf", bom: false, trailingNewline: true,
          blocks: [ { type: "conflict", base: ["base"], ours: ["ours"], theirs: ["theirs"] } ] });
      }
      if (command === "git_conflict_apply_resolution") {
        window.__conflictCalls.push({ command: command, args: args });
        return Promise.resolve(null);
      }
      if (command === "git_conflict_mark_resolved") {
        window.__conflictCalls.push({ command: command, args: args });
        resolved = true;
        return Promise.resolve(null);
      }
      if (command === "git_conflict_continue" || command === "git_conflict_abort" || command === "git_conflict_skip") {
        window.__conflictCalls.push({ command: command, args: args });
        if (command === "git_conflict_continue") return Promise.resolve({ oid: "a1b2c3d4", conflicts: [] });
        if (command === "git_conflict_abort") return Promise.resolve({ headOid: "a1b2c3", headRef: "main", snapshotId: 7 });
        return Promise.resolve({ oid: null, conflicts: ["src/a.ts"] });
      }
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

async function openConflictPage(page: Page): Promise<void> {
  await page.goto('/#/repo/1/conflict');
  await expect(page.getByTestId('conflict-page')).toBeVisible();
}

async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(MOCK_SCRIPT);
});

test('冲突页显示操作类型、文件清单，且未解决时不能继续', async ({ page }) => {
  await openConflictPage(page);

  await expect(page.getByTestId('conflict-op-kind')).toHaveText('合并');
  await expect(page.getByTestId('conflict-into-branch')).toContainText('main');
  await expect(page.getByTestId('conflict-list-item-src/a.ts')).toHaveCount(1);
  await expect(page.getByTestId('conflict-list-item-src/a.ts')).toContainText('src/a.ts');
  await expect(page.getByTestId('conflict-continue')).toBeDisabled();
  await expect(page.getByTestId('conflict-abort')).toBeEnabled();
  await expectNoPageErrors(page);
});

test('标记已解决后可以继续，继续发出的是 git_conflict_continue', async ({ page }) => {
  await openConflictPage(page);

  // 打开编辑器，用"标记已解决（不保存）"（mock 里 mark_resolved 后 state 清空）
  await page.getByTestId('conflict-list-item-src/a.ts').click();
  await page.getByTestId('editor-mark-only').click();
  await expect(page.getByTestId('conflict-continue')).toBeEnabled({ timeout: 5000 });
  await page.getByTestId('conflict-continue').click();

  await expect
    .poll(() => page.evaluate(() => window.__conflictCalls?.map((call) => call.command) ?? []))
    .toEqual(['git_conflict_mark_resolved', 'git_conflict_continue']);
  // mark_resolved 的参数必须是状态页报告的路径数组
  const calls = await page.evaluate(() => window.__conflictCalls ?? []);
  expect(calls[0]?.args).toEqual({ repoId: 1, paths: ['src/a.ts'] });
  await expectNoPageErrors(page);
});

test('中止必须经过确认框，确认后发出 git_conflict_abort', async ({ page }) => {
  await openConflictPage(page);

  await page.getByTestId('conflict-abort').click();
  // 确认框弹出但尚未执行
  await expect(page.getByTestId('conflict-abort-confirm')).toBeVisible();
  expect(await page.evaluate(() => window.__conflictCalls ?? [])).toEqual([]);

  await page.getByTestId('conflict-abort-confirm').click();
  await expect
    .poll(() => page.evaluate(() => window.__conflictCalls?.map((call) => call.command) ?? []))
    .toEqual(['git_conflict_abort']);
  await expectNoPageErrors(page);
});

// ---------------------------------------------------------------- T3.2 编辑器

/** 二进制冲突场景：blob 内容不可显示，编辑器应落到"采用一方"面板。 */
const BINARY_MOCK_SCRIPT = `
  window.__errs = [];
  window.__conflictCalls = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback: function () { return 1; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "plugin:event|listen") return Promise.resolve(1);
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_all" || command === "settings_get") return Promise.resolve(null);
      if (command === "settings_set") return Promise.resolve(null);
      if (command === "git_branch_list") return Promise.resolve([]);
      if (command === "workspace_status") return Promise.resolve({
        branch: { oid: "a1b2c3", head: "main", detached: false, upstream: null, ahead: null, behind: null },
        operation: "merge", staged: [], unstaged: [], untracked: [], conflicted: ["img.png"], ignored: [], ignoredCount: null
      });
      if (command === "git_conflict_state") {
        return Promise.resolve({ opKind: "merge", opInProgress: true, currentStep: null, totalSteps: null,
          headName: null, intoBranch: "main",
          files: [{ path: "img.png", kind: "binary", base: null, ours: null, theirs: null, worktreeExists: true }],
          canContinue: false, canAbort: true, canSkip: false });
      }
      if (command === "git_conflict_file_detail") {
        return Promise.resolve({ path: "img.png", kind: "binary",
          base: { size: 10, isBinary: true, encodingHint: null, content: null },
          ours: { size: 12, isBinary: true, encodingHint: null, content: null },
          theirs: { size: 14, isBinary: true, encodingHint: null, content: null },
          worktreeExists: true, eol: "lf", bom: false, trailingNewline: false, blocks: [] });
      }
      if (command === "git_conflict_take_side") {
        window.__conflictCalls.push({ command: command, args: args });
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

/** 编辑器场景的 mock：单文件 8 块冲突（4 未解决起手）+ 可切换的解析状态。 */
const EDITOR_MOCK_SCRIPT = `
  window.__errs = [];
  window.__conflictCalls = [];
  const blocks = [];
  for (let i = 0; i < 8; i += 1) {
    blocks.push({ type: "context", lines: ["ctx-" + i] });
    blocks.push({
      type: "conflict",
      base: ["base-" + i],
      ours: ["ours-" + i],
      theirs: ["theirs-" + i]
    });
  }
  window.__TAURI_INTERNALS__ = {
    transformCallback: function () { return 1; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "plugin:event|listen") return Promise.resolve(1);
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_all" || command === "settings_get") return Promise.resolve(null);
      if (command === "settings_set") return Promise.resolve(null);
      if (command === "git_branch_list") return Promise.resolve([]);
      if (command === "workspace_status") return Promise.resolve({
        branch: { oid: "a1b2c3", head: "main", detached: false, upstream: null, ahead: null, behind: null },
        operation: "merge", staged: [], unstaged: [], untracked: [], conflicted: ["a.txt"], ignored: [], ignoredCount: null
      });
      if (command === "git_conflict_state") {
        return Promise.resolve({ opKind: "merge", opInProgress: true, currentStep: null, totalSteps: null,
          headName: null, intoBranch: "main",
          files: [{ path: "a.txt", kind: "text", base: null, ours: null, theirs: null, worktreeExists: true }],
          canContinue: false, canAbort: true, canSkip: false });
      }
      if (command === "git_conflict_file_detail") {
        return Promise.resolve({ path: "a.txt", kind: "text",
          base: { size: 6, isBinary: false, encodingHint: "utf-8", content: "base" },
          ours: { size: 6, isBinary: false, encodingHint: "utf-8", content: "ours" },
          theirs: { size: 6, isBinary: false, encodingHint: "utf-8", content: "theirs" },
          worktreeExists: true, eol: "lf", bom: false, trailingNewline: true, blocks: blocks });
      }
      if (command === "git_conflict_apply_resolution" || command === "git_conflict_mark_resolved") {
        window.__conflictCalls.push({ command: command, args: args });
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

test.describe('冲突编辑器（T3.2）', () => {
  test('逐块采用本方后保存，捕获到的结果文本无残留标记', async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
    await page.addInitScript(EDITOR_MOCK_SCRIPT);
    await page.goto('/#/repo/1/conflict');
    await expect(page.getByTestId('conflict-page')).toBeVisible();

    // 打开编辑器
    await page.getByTestId('conflict-list-item-a.txt').click();
    await expect(page.getByTestId('conflict-editor')).toBeVisible();

    // 8 个冲突块全部采用本方；每次点击后等卡片翻转（React 重渲期间的
    // 点击可能落在被替换的节点上，等状态落地是顺序操作的正确姿势）
    for (let i = 0; i < 8; i += 1) {
      await page.getByTestId(`conflict-adopt-ours-${i}`).click();
      await expect(page.locator('article[data-conflict-card][data-resolved="true"]')).toHaveCount(
        i + 1,
      );
    }
    await page.getByTestId('editor-save-resolve').click();

    await expect
      .poll(() => page.evaluate(() => window.__conflictCalls?.map((call) => call.command) ?? []))
      .toContain('git_conflict_apply_resolution');
    const content = await page.evaluate(() => {
      const applied = (window.__conflictCalls ?? []).find(
        (call) => call.command === 'git_conflict_apply_resolution',
      );
      const spec = (applied?.args as { spec?: { content?: string } } | undefined)?.spec;
      return spec?.content ?? '';
    });
    // 内容级 grep 断言：所有块都采用后结果里不允许残留 git 标记
    // （真实文件的字节写回与 stage 校验由 services 集成测试覆盖）
    expect(content).toContain('ours-0');
    expect(content).not.toContain('<<<<<<<');
    expect(content).not.toContain('=======');
    expect(content).not.toContain('>>>>>>>');
    await expectNoPageErrors(page);
  });

  test('纯键盘完成一次完整冲突解决（任务书 T3.3 验收）', async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
    await page.addInitScript(MOCK_SCRIPT);
    await page.goto('/#/repo/1/conflict');
    await expect(page.getByTestId('conflict-page')).toBeVisible();
    // 准备步骤：打开编辑器（解决操作本身全程键盘）
    await page.getByTestId('conflict-list-item-src/a.ts').click();
    await expect(page.getByTestId('conflict-editor')).toBeVisible();

    await page.keyboard.press('j'); // 选中第一处冲突
    await expect(page.locator('article[data-conflict-card][data-selected="true"]')).toHaveCount(1);
    await page.keyboard.press('o'); // 采用本方
    await expect(page.locator('article[data-conflict-card][data-resolved="true"]')).toHaveCount(1);
    await page.keyboard.press('Control+s'); // 保存并标记已解决

    await expect
      .poll(() => page.evaluate(() => window.__conflictCalls?.map((call) => call.command) ?? []))
      .toContain('git_conflict_apply_resolution');
    const content = await page.evaluate(() => {
      const applied = (window.__conflictCalls ?? []).find(
        (call) => call.command === 'git_conflict_apply_resolution',
      );
      const spec = (applied?.args as { spec?: { content?: string } } | undefined)?.spec;
      return spec?.content ?? '';
    });
    expect(content).toContain('ours');
    expect(content).not.toContain('<<<<<<<');
    await expectNoPageErrors(page);
  });

  test('批量采用的确认框在取消时不产生任何修改（任务书 T3.3 验收）', async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
    await page.addInitScript(EDITOR_MOCK_SCRIPT);
    await page.goto('/#/repo/1/conflict');
    await page.getByTestId('conflict-list-item-a.txt').click();
    await expect(page.getByTestId('conflict-editor')).toBeVisible();

    await page.getByTestId('editor-batch-ours').click();
    await expect(page.getByTestId('editor-batch-confirm')).toBeVisible();
    // 取消：不产生任何修改（卡片全部保持未解决、没有任何命令发出）
    await page.getByTestId('editor-batch-cancel').click();
    await expect(page.locator('article[data-conflict-card][data-resolved="true"]')).toHaveCount(0);
    expect(await page.evaluate(() => window.__conflictCalls ?? [])).toEqual([]);
    await expectNoPageErrors(page);

    // 确认路径：全部 8 块翻转为已解决
    await page.getByTestId('editor-batch-ours').click();
    await page.getByTestId('editor-batch-confirm').click();
    await expect(page.locator('article[data-conflict-card][data-resolved="true"]')).toHaveCount(8);
    await expectNoPageErrors(page);
  });

  test('二进制冲突走"采用一方"路径且不崩溃', async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
    await page.addInitScript(BINARY_MOCK_SCRIPT);
    await page.goto('/#/repo/1/conflict');
    await expect(page.getByTestId('conflict-page')).toBeVisible();
    await page.getByTestId('conflict-list-item-img.png').click();

    await expect(page.getByTestId('conflict-binary-panel')).toBeVisible();
    await page.getByTestId('binary-take-ours').click();
    await expect
      .poll(() => page.evaluate(() => window.__conflictCalls?.map((call) => call.command) ?? []))
      .toContain('git_conflict_take_side');
    await expectNoPageErrors(page);
  });
});

/** 性能验收场景：单文件 200 个冲突块（任务书：20 文件 / 200 块，操作 < 100ms）。 */
const PERF_MOCK_SCRIPT = `
  window.__errs = [];
  const blocks = [];
  for (let i = 0; i < 200; i += 1) {
    blocks.push({ type: "conflict", base: ["base-" + i], ours: ["ours-" + i], theirs: ["theirs-" + i] });
  }
  window.__TAURI_INTERNALS__ = {
    transformCallback: function () { return 1; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "plugin:event|listen") return Promise.resolve(1);
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_all" || command === "settings_get") return Promise.resolve(null);
      if (command === "settings_set") return Promise.resolve(null);
      if (command === "git_branch_list") return Promise.resolve([]);
      if (command === "workspace_status") return Promise.resolve({
        branch: { oid: "a1b2c3", head: "main", detached: false, upstream: null, ahead: null, behind: null },
        operation: "merge", staged: [], unstaged: [], untracked: [], conflicted: ["big.txt"], ignored: [], ignoredCount: null
      });
      if (command === "git_conflict_state") {
        return Promise.resolve({ opKind: "merge", opInProgress: true, currentStep: null, totalSteps: null,
          headName: null, intoBranch: "main",
          files: [{ path: "big.txt", kind: "text", base: null, ours: null, theirs: null, worktreeExists: true }],
          canContinue: false, canAbort: true, canSkip: false });
      }
      if (command === "git_conflict_file_detail") {
        return Promise.resolve({ path: "big.txt", kind: "text",
          base: null, ours: { size: 1, isBinary: false, encodingHint: "utf-8", content: "x" },
          theirs: { size: 1, isBinary: false, encodingHint: "utf-8", content: "y" },
          worktreeExists: true, eol: "lf", bom: false, trailingNewline: false, blocks: blocks });
      }
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

test.describe('冲突编辑器性能（T3.2 验收）', () => {
  test('200 个冲突块的块操作延迟 < 100ms', async ({ page }) => {
    await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
    await page.addInitScript(PERF_MOCK_SCRIPT);
    await page.goto('/#/repo/1/conflict');
    await page.getByTestId('conflict-list-item-big.txt').click();
    await expect(page.getByTestId('conflict-editor')).toBeVisible();
    await expect(page.getByTestId('conflict-adopt-ours-0')).toBeVisible();

    // 测量方法：performance.now 包裹"点击采用本方 → 该卡片 data-resolved 翻转"，
    // 取 10 次操作的最大值。卡片的解决状态是 React 状态 → DOM 属性，
    // 属性翻转即"界面已经反映了这次操作"。
    const durations = await page.evaluate(async () => {
      const results: number[] = [];
      for (let i = 0; i < 10; i += 1) {
        const button = document.querySelector(
          `[data-testid="conflict-adopt-ours-${i}"]`,
        ) as HTMLButtonElement | null;
        const card = button?.closest('[data-conflict-card]');
        if (button === null || card === null) throw new Error('card not found');
        const target: Element = card as Element;
        const done = new Promise<void>((resolve) => {
          const observer = new MutationObserver(() => {
            if (target.getAttribute('data-resolved') === 'true') {
              observer.disconnect();
              resolve();
            }
          });
          observer.observe(target, { attributes: true, attributeFilter: ['data-resolved'] });
        });
        const start = performance.now();
        button.click();
        await done;
        results.push(performance.now() - start);
      }
      return results;
    });
    const worst = Math.max(...durations);
    // 附到失败信息里，方便回归对比
    expect(
      worst,
      `10 次块操作延迟：${durations.map((ms) => ms.toFixed(1)).join(', ')} ms`,
    ).toBeLessThan(100);
    await expectNoPageErrors(page);
  });
});
