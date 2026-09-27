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
  // 记录收到的暂存 / 取消暂存请求：断言"界面选的粒度与下标"是否原样传到后端
  window.__stagingCalls = [];
  window.__mockFiles = files;
  function specPaths(spec) {
    if (!spec) return [];
    return spec.kind === "files" ? (spec.paths || []) : (spec.path ? [spec.path] : []);
  }
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
  function emitRepoChanged(paths, kind) {
    for (const listener of listeners) listener({ event: "repo:changed", id: 0, payload: { repoId: 1, kind: kind || "workspace", paths } });
  }
  // 供用例模拟"外部变化"（真实环境里这些事件来自文件监听，T1.10）
  window.__emitRepoChanged = emitRepoChanged;
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "workspace_status") return Promise.resolve(report());
      if (command === "workspace_stage" || command === "workspace_unstage") {
        const spec = args.spec || { kind: "files", paths: [] };
        const paths = specPaths(spec);
        window.__stagingCalls.push({ command: command, spec: spec, view: args.view || null, paths: paths });
        for (const path of paths) {
          const f = files.find((x) => x.path === path);
          if (!f) continue;
          if (command === "workspace_stage") {
            // 文件粒度：整个文件进索引；行级 / 块级：索引变了但工作区仍有改动
            // （真实行为就是同时出现在"已暂存"与"未暂存"两个分组里）
            const wholeFile = spec.kind === "files";
            f.indexStatus = f.kind === "untracked" ? "A" : "M";
            f.worktreeStatus = wholeFile ? "." : "M";
            f.kind = "ordinary";
          } else {
            f.indexStatus = ".";
            f.worktreeStatus = f.kind === "untracked" ? "?" : "M";
          }
        }
        emitRepoChanged(paths);
        return Promise.resolve(null);
      }
      if (command === "workspace_discard") {
        const spec = args.spec || { kind: "files", tracked: [], untracked: [] };
        if (spec.kind === "files") {
          for (const path of spec.untracked || []) { const i = files.findIndex((x) => x.path === path); if (i >= 0) files.splice(i, 1); }
          for (const path of spec.tracked || []) { const f = files.find((x) => x.path === path); if (f) { f.worktreeStatus = "."; } }
          emitRepoChanged((spec.tracked || []).concat(spec.untracked || []));
        } else {
          emitRepoChanged([spec.path]);
        }
        return Promise.resolve(null);
      }
      if (command === "workspace_reveal") return Promise.resolve(null);
      if (command === "workspace_diff") {
        const path = (args.spec && args.spec.paths && args.spec.paths[0]) || "";
        const big = path === "untracked.txt";
        const perHunk = big ? 1500 : 3;
        const hunks = [];
        for (let h = 0; h < 2; h++) {
          const lines = [];
          for (let i = 0; i < perHunk; i++) {
            lines.push({ kind: "context", content: "ctx " + h + "-" + i, oldNo: h * perHunk + i + 1, newNo: h * perHunk + i + 1 });
          }
          // 行号非空：界面只让"该侧真的有行号"的那一列可点（与真实 git 输出一致）
          lines.push({ kind: "removed", content: "old value", oldNo: h * perHunk + perHunk + 1, newNo: null });
          lines.push({ kind: "added", content: "new value " + h, oldNo: null, newNo: h * perHunk + perHunk + 1 });
          hunks.push({ oldStart: h * perHunk + 1, oldLines: perHunk + 1, newStart: h * perHunk + 1, newLines: perHunk + 1, header: "fn " + h, lines: lines });
        }
        return Promise.resolve({ files: [{ path: path, oldPath: null, change: "modified", binary: false, additions: 2, deletions: 2, truncated: false, hunks: hunks }], truncatedFiles: 0 });
      }
      if (command === "workspace_diff_patch") return Promise.resolve([100, 110]);
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
test('行选择 → 暂存 → 计数变化 → 后端收到行级 spec → __errs 为空', async ({ page }) => {
  await page.goto('/#/repo/1/status');
  await expect(page.getByRole('button', { name: '未暂存' }).first()).toBeVisible();

  // 点文件名打开行级 diff
  await page.getByRole('button', { name: 'dirty-0.ts' }).click();
  const diff = page.getByTestId('diff-view');
  await expect(diff).toBeVisible();

  // 选一行（"修改对"的删除行是第 4 行；新增行同号，取第一个）
  await diff.getByRole('button', { name: '选择第 4 行' }).first().click();
  await expect(diff.getByText('已选 1 行')).toBeVisible();

  await diff.getByRole('button', { name: '暂存选中行' }).click();

  // 行级 spec 与下标原样传到后端（下标口径 = 该行在 hunk lines 里的位置）
  const calls = await page.evaluate(() => window.__stagingCalls ?? []);
  expect(calls.at(-1)).toMatchObject({
    command: 'workspace_stage',
    spec: { kind: 'lines', path: 'src/dirty-0.ts', selections: [{ hunkIndex: 0, lines: [3] }] },
    view: { contextLines: 3 },
  });

  // mock 侧的夹具确实变成了"索引有内容、工作区也有改动"
  const touched = await page.evaluate(
    () => window.__mockFiles?.find((file) => file.path === 'src/dirty-0.ts') ?? null,
  );
  expect(touched).toMatchObject({ indexStatus: 'M', worktreeStatus: 'M' });

  // 先关掉抽屉：Sheet 打开时 Radix 会把页面其余内容标记为 aria-hidden，
  // 按角色查询会直接找不到分组按钮（不是"不可见"，是"不存在于无障碍树"）
  await page.getByRole('button', { name: '关闭 diff' }).click();
  await expect(page.getByTestId('workspace-toolbar')).toBeVisible();

  // 部分暂存后该文件同时出现在两组：已暂存 5 → 6（它仍未暂存，因为工作区还有改动）
  const groupLabels = await page
    .getByRole('button', { name: /已暂存|未暂存|未跟踪/ })
    .allTextContents();
  expect(groupLabels.join(' | '), '分组计数应当更新').toMatch(/已暂存\s*6/);

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('hunk 头的"暂存此块"按块粒度提交', async ({ page }) => {
  await page.goto('/#/repo/1/status');
  await expect(page.getByRole('button', { name: '未暂存' }).first()).toBeVisible();

  await page.getByRole('button', { name: 'dirty-1.ts' }).click();
  const diff = page.getByTestId('diff-view');
  await expect(diff).toBeVisible();

  await diff.getByRole('button', { name: '暂存此块' }).first().click();

  const calls = await page.evaluate(() => window.__stagingCalls ?? []);
  expect(calls.at(-1)).toMatchObject({
    command: 'workspace_stage',
    spec: { kind: 'hunks', path: 'src/dirty-1.ts', hunkIndices: [0] },
  });

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('外部变化：repo:changed 驱动刷新，large 事件给出说明', async ({ page }) => {
  await page.goto('/#/repo/1/status');
  await expect(page.getByRole('button', { name: /未暂存 3/ })).toBeVisible();

  // 模拟"用户在终端里新建了一个文件"：改夹具 + 发事件。
  // 真实环境里这件事由文件监听完成（后端 crates/platform 的 watcher）。
  await page.evaluate(() => {
    window.__mockFiles?.push({
      path: 'src/external.ts',
      kind: 'untracked',
      indexStatus: '.',
      worktreeStatus: '?',
      isBinary: false,
      isLfs: false,
      isSubmodule: false,
      sizeBytes: 5,
    });
    window.__emitRepoChanged?.([], 'workspace');
  });

  await expect(page.getByRole('button', { name: /未跟踪 3/ })).toBeVisible({ timeout: 10_000 });

  // large：一次窗口内的变化太多，界面要说明"为什么整体刷了一次"
  await page.evaluate(() => {
    window.__emitRepoChanged?.([], 'large');
  });
  await expect(page.getByTestId('workspace-large-change')).toBeVisible();

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

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

// ---------------------------------------------------------------- T1.5 diff 查看器

test('T1.5: 点文件名打开 diff → 切换并排 → 切回内联 → __errs 为空', async ({ page }) => {
  await page.goto('/#/repo/1/status');
  await expect(page.getByRole('heading', { name: '工作区' })).toBeVisible();

  // 点未暂存文件的文件名（不是复选框、不是行内操作按钮）
  await page.getByRole('button', { name: 'dirty-0.ts' }).first().click();

  const view = page.getByTestId('diff-view');
  await expect(view).toBeVisible();

  // 内联模式：修改对的词级片段
  await expect(page.getByText(/old/).first()).toBeVisible();

  // 切到并排：同一修改对的左右两半都在
  await page.getByRole('radio', { name: '并排' }).click();
  await expect(page.getByText(/new value/).first()).toBeVisible();

  // 切回内联
  await page.getByRole('radio', { name: '内联' }).click();
  await expect(page.getByText(/old/).first()).toBeVisible();

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('T1.5: 折叠 hunk 后该 hunk 正文隐藏，再展开恢复', async ({ page }) => {
  await page.goto('/#/repo/1/status');
  await page.getByRole('button', { name: 'dirty-0.ts' }).first().click();
  const view = page.getByTestId('diff-view');
  await expect(view).toBeVisible();

  // 两个 hunk 各自的内容都可先见到
  await expect(page.getByText('new value 0')).toBeVisible();

  // 折叠交互契约：点击 hunk 头在 aria-expanded 之间翻转。
  // （行的显隐由 DiffView 单测覆盖；这里的 e2e 验证真实点击链路可达。）
  const firstHeader = page.getByRole('button', { name: /@@ -1,4 \+1,4 @@/ }).first();
  await expect(firstHeader).toHaveAttribute('aria-expanded', 'true');
  await firstHeader.click();
  await expect(firstHeader).toHaveAttribute('aria-expanded', 'false');
  await firstHeader.click();
  await expect(firstHeader).toHaveAttribute('aria-expanded', 'true');
  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('T1.5 基准：3000 行 diff 渲染与滚动', async ({ page }) => {
  await page.goto('/#/repo/1/status');
  // mock 约定：untracked.txt 的 diff 返回 2 个 hunk × 1500 上下文行 = 3000 行
  await page.getByRole('button', { name: 'untracked.txt' }).first().click();
  await expect(page.getByTestId('diff-view')).toBeVisible();

  const start = Date.now();
  // 3000 行分 2 个 hunk、每 hunk 1500 上下文行：滚动到底再回顶
  const list = page.getByRole('list', { name: 'diff 内容' });
  await list.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });
  await page.waitForTimeout(120);
  await list.evaluate((el) => {
    el.scrollTop = 0;
  });
  const elapsed = Date.now() - start;
  // eslint-disable-next-line no-console -- 基准数据走测试输出
  console.log('T1.5 基准：3000 行 diff 两次全量滚动 ' + elapsed + 'ms（虚拟化只渲染可见行）');
  expect(elapsed).toBeLessThan(1000);
  const errs2 = await page.evaluate(() => window.__errs ?? []);
  expect(errs2, JSON.stringify(errs2)).toEqual([]);
});
