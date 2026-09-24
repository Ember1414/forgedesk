import { expect, test, type Page } from '@playwright/test';

// T1.4 交互级验收：在浏览器里通过 mock 的 Tauri IPC 跑完整流程。
//
// mock 方式：在应用脚本运行前定义 window.__TAURI_INTERNALS__（Tauri v2 的
// invoke 与 event 都经它路由）。workspace_status 返回内存夹具；
// stage/unstage/discard 真实地修改夹具并投递 repo:changed——
// 因此"批量暂存 → 计数变化"走的是与真实后端完全相同的失效→重取链路。

const MOCK_SCRIPT = `
  const files = [];
  for (let i = 0; i < 5; i++) files.push({ path: "src/staged-" + i + ".ts", kind: "ordinary", indexStatus: "A", worktreeStatus: ".", isBinary: false, isLfs: false, isSubmodule: false, sizeBytes: 12 });
  for (let i = 0; i < 3; i++) files.push({ path: "src/dirty-" + i + ".ts", kind: "ordinary", indexStatus: ".", worktreeStatus: "M", isBinary: false, isLfs: false, isSubmodule: false, sizeBytes: 20 });
  files.push({ path: "untracked.txt", kind: "untracked", indexStatus: "?", worktreeStatus: "?", isBinary: false, isLfs: false, isSubmodule: false, sizeBytes: 6 });
  files.push({ path: "assets/logo.png", kind: "untracked", indexStatus: "?", worktreeStatus: "?", isBinary: true, isLfs: false, isSubmodule: false, sizeBytes: 999 });
  const listeners = [];
  function group(name) { return files.filter((f) => f.kind === name); }
  function report() {
    return {
      branch: { oid: "abc", head: "main", detached: false, upstream: "origin/main", ahead: 0, behind: 0 },
      operation: "none",
      staged: group("ordinary").filter((f) => f.indexStatus !== "."),
      unstaged: group("ordinary").filter((f) => f.worktreeStatus !== "."),
      untracked: files.filter((f) => f.kind === "untracked"),
      conflicted: [],
      ignored: [],
      ignoredCount: null,
    };
  }
  function emitRepoChanged(paths) {
    for (const listener of listeners) listener({ event: "repo:changed", id: 0, payload: { repoId: 1, paths } });
  }
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "workspace_status") return Promise.resolve(report());
      if (command === "workspace_stage") {
        for (const path of args.paths) {
          const f = files.find((x) => x.path === path);
          if (f) { f.indexStatus = f.kind === "untracked" ? "A" : "M"; f.worktreeStatus = "."; f.kind = "ordinary"; }
        }
        emitRepoChanged(args.paths);
        return Promise.resolve(null);
      }
      if (command === "workspace_unstage") {
        for (const path of args.paths) {
          const f = files.find((x) => x.path === path);
          if (f) { f.indexStatus = "."; f.worktreeStatus = f.kind === "untracked" ? "?" : "M"; }
        }
        emitRepoChanged(args.paths);
        return Promise.resolve(null);
      }
      if (command === "workspace_discard") {
        for (const path of args.untracked) { const i = files.findIndex((x) => x.path === path); if (i >= 0) files.splice(i, 1); }
        for (const path of args.tracked) { const f = files.find((x) => x.path === path); if (f) { f.worktreeStatus = "."; } }
        emitRepoChanged(args.tracked.concat(args.untracked));
        return Promise.resolve(null);
      }
      if (command === "workspace_reveal") return Promise.resolve(null);
      if (command === "plugin:event|unlisten") return Promise.resolve(null);
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_get") return Promise.resolve(null);
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

test.beforeEach(async ({ page }) => {
  // 断言走 zh-CN 文案：先于应用脚本写入语言偏好（否则跟随浏览器语言渲染成英文）
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(MOCK_SCRIPT);
});
test('打开仓库 → 看到分组 → 批量暂存 → 计数变化 → __errs 为空', async ({ page }) => {
  await page.goto('/#/repo/1/status');
  await expect(page.getByRole('heading', { name: '工作区' })).toBeVisible();

  // 分组与计数（未暂存 3、未跟踪 2、已暂存 5）
  const toolbar = toolbarOf(page);
  await expect(page.getByRole('button', { name: '未暂存' }).first()).toBeVisible();
  await expect(page.getByRole('button', { name: '未跟踪' }).first()).toBeVisible();
  await expect(page.getByRole('button', { name: '已暂存' }).first()).toBeVisible();

  // 全选 → 批量暂存 → 全部进入已暂存分组（走真实的 repo:changed 失效链路）
  await toolbar.getByRole('button', { name: '全选' }).first().click();
  await toolbar.getByRole('button', { name: /暂存 \(/ }).click();

  await expect(page.getByRole('button', { name: /已暂存 10/ })).toBeVisible({ timeout: 10_000 });
  await expect(page.getByRole('button', { name: '未暂存' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: '未跟踪' })).toHaveCount(0);

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

function toolbarOf(page: Page) {
  return page.getByTestId('workspace-toolbar');
}
test('放弃走确认对话框并列出路径', async ({ page }) => {
  await page.goto('/#/repo/1/status');
  await expect(page.getByRole('button', { name: '未跟踪' }).first()).toBeVisible();

  await page.getByRole('checkbox', { name: 'untracked.txt' }).check();
  await page.getByTestId('workspace-toolbar').getByRole('button', { name: '放弃' }).first().click();

  await expect(page.getByText('放弃这些修改？')).toBeVisible();
  await expect(page.locator('ul').getByText('untracked.txt')).toBeVisible();
  await page.getByRole('button', { name: '放弃', exact: true }).last().click();
  await expect(page.getByText('放弃这些修改？')).toHaveCount(0);
});

test('10000 个变更文件的首屏渲染（性能基准）', async ({ page }) => {
  await page.addInitScript(`
    const original = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = function (command, args) {
      if (command === "workspace_status") {
        return original(command, args).then((report) => {
          const big = [...report.unstaged];
          for (let i = 0; i < 10000; i++) {
            big.push({ path: "huge/dir-" + Math.floor(i / 100) + "/file-" + i + ".ts", kind: "ordinary", indexStatus: ".", worktreeStatus: "M", isBinary: false, isLfs: false, isSubmodule: false, sizeBytes: i });
          }
          return { ...report, unstaged: big };
        });
      }
      return original(command, args);
    };
  `);

  const start = Date.now();
  await page.goto('/#/repo/1/status');
  await expect(page.getByText('file-0.ts')).toBeVisible({ timeout: 5000 });
  const elapsed = Date.now() - start;
  // eslint-disable-next-line no-console -- 基准数据走测试输出
  console.log('T1.4 基准：10000 文件首屏渲染 ' + elapsed + 'ms（含 dev server 与 mock IPC）');
  expect(elapsed).toBeLessThan(1000);
});
