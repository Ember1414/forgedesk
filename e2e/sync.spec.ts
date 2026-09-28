import { expect, test, type Page } from '@playwright/test';

/**
 * T2.6 交互级验收：远端同步条（Fetch / Pull / Push）。
 *
 * # 为什么必须走事件，而不是只看按钮
 *
 * 三个同步操作都是**长任务**：命令立即返回 `jobId`，进度与结果经 `job:*` 事件
 * 到达。真实 GitHub 联调由用户在有网机器上跑（见 docs/acceptance 的检查表），
 * 这里用 mock 的 Tauri IPC 把**产品自己那半条链路**跑通：
 *
 *   点按钮 → invoke 带什么参数 → 事件到达 → 界面正确落到进度条 / 冲突对话框 /
 *   被拒对话框 → 修复动作真的再发一次命令 → `__errs` 为空。
 *
 * mock 里的 DTO 字段一律 **camelCase**：那是 docs/API.md §1 的契约，
 * Rust 侧曾经漏过 `serde(rename_all = "camelCase")`，结果是"界面看着正常、
 * ahead/behind 与上游名全是空的"。这份 spec 用真实形状盯住它。
 */
const MOCK_SCRIPT = `
  window.__errs = [];
  window.__syncCalls = [];
  window.__syncJobs = 0;
  const callbacks = {};
  const eventOf = {};
  let nextCallbackId = 1;
  const branches = [
    { name: "main", isRemote: false, isHead: true, target: "a1b2c3", upstream: "origin/main", ahead: 2, behind: 3, upstreamGone: false }
  ];
  // 按**事件名**投递：真实运行时里 Rust 只回调订阅了该事件的处理器，
  // 一律广播会让同一份载荷被进度/完成/失败三个处理器各读一遍（假失败）。
  window.__emitJob = function (event, payload) {
    for (const key of Object.keys(eventOf)) {
      if (eventOf[key] === event) {
        callbacks[key]({ event: event, id: Number(key), payload: payload });
      }
    }
  };
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { const id = nextCallbackId; nextCallbackId += 1; callbacks[id] = callback; return id; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "plugin:event|listen") { eventOf[args.handler] = args.event; return Promise.resolve(nextCallbackId); }
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_all" || command === "settings_get") return Promise.resolve(null);
      if (command === "settings_set") return Promise.resolve(null);
      if (command === "git_branch_list") return Promise.resolve(branches);
      if (command === "git_branch_compare") return Promise.resolve({ ahead: 2, behind: 3, onlyInA: [] });
      if (command === "workspace_status") return Promise.resolve({
        branch: { oid: "a1b2c3", head: "main", detached: false, upstream: "origin/main", ahead: 2, behind: 3 },
        operation: "none", staged: [], unstaged: [], untracked: [], conflicted: [], ignored: [], ignoredCount: null
      });
      if (command === "git_fetch" || command === "git_pull" || command === "git_push") {
        window.__syncJobs += 1;
        window.__syncCalls.push({ command: command, args: args });
        return Promise.resolve({ jobId: "job-" + window.__syncJobs });
      }
      if (command === "job_cancel") return Promise.resolve(true);
      if (command === "plugin:event|unlisten") return Promise.resolve(null);
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

/** 后端在被拒时给出的三条修复动作（形状由 `services/sync.rs` 决定）。 */
const REJECTED_ACTION_SCRIPT = `
  window.__emitJob("job:failed", {
    jobId: "job-1",
    error: {
      code: "PUSH_REJECTED",
      message: "the push was rejected because the remote has commits you do not have",
      detail: "fetch first",
      actions: [
        { id: "fetch-first", labelKey: "errors:actions.pushFetchFirst", command: "git_fetch" },
        { id: "force-with-lease", labelKey: "errors:actions.pushForceWithLease", command: "noop" },
        { id: "cancel", labelKey: "errors:actions.cancel", command: "noop" }
      ]
    }
  });
`;

async function openRepo(page: Page): Promise<void> {
  await page.goto('/#/repo/1/status');
  await expect(page.getByTestId('sync-bar')).toBeVisible();
}

/** 等到 mock 收到第 n 个同步命令（界面把任务真的发出去了）。 */
async function expectSyncCalls(page: Page, count: number): Promise<void> {
  await expect.poll(() => page.evaluate(() => window.__syncCalls?.length ?? 0)).toBe(count);
}

async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(MOCK_SCRIPT);
});

test('同步条显示上游与领先/落后，推送是长任务且进度事件驱动进度条', async ({ page }) => {
  await openRepo(page);

  // 上游与 ahead/behind 都来自 git_branch_list + git_branch_compare
  await expect(page.getByTestId('sync-status')).toContainText('origin/main');
  await expect(page.getByTestId('sync-ahead')).toHaveText('↑2');
  await expect(page.getByTestId('sync-behind')).toHaveText('↓3');

  await page.getByTestId('sync-push').click();
  await expectSyncCalls(page, 1);

  // 有上游时不带 --set-upstream（git 自己推到上游）
  const pushSpec = await page.evaluate(() => window.__syncCalls?.[0]?.args?.spec);
  expect(pushSpec).toEqual({});

  // 进度事件 → 进度条出现在同步条上，并带上阶段文案
  await page.evaluate(() => {
    window.__emitJob?.('job:progress', {
      jobId: 'job-1',
      phase: 'writing',
      current: 3,
      total: 6,
      message: 'Writing objects: 50% (3/6)',
    });
  });
  const strip = page.getByTestId('sync-progress');
  await expect(strip).toBeVisible();
  await expect(strip).toContainText('写入对象');
  await expect(strip).toContainText('50%');

  // 展开"详细日志"能看到 git 的原始输出
  await page.getByTestId('sync-detail-toggle').click();
  await expect(page.getByTestId('sync-detail')).toContainText('Writing objects');

  // 完成事件 → 进度收尾，成功提示出现在 toast 里
  await page.evaluate(() => {
    window.__emitJob?.('job:done', {
      jobId: 'job-1',
      result: { remote: 'origin', push: { remote: 'origin', updates: [], rejections: [] } },
    });
  });
  await expect(page.getByTestId('sync-progress')).toHaveCount(0);
  // 成功提示出现在 toast 里（`first()`：无障碍实时区域里还有同样一句话）
  await expect(page.getByText('已推送到 origin').first()).toBeVisible();

  await expectNoPageErrors(page);
});

test('推送被拒：三条修复路径可见，force-with-lease 会带上标志重推', async ({ page }) => {
  await openRepo(page);

  await page.getByTestId('sync-push').click();
  await expectSyncCalls(page, 1);

  await page.evaluate(REJECTED_ACTION_SCRIPT);

  const dialog = page.getByTestId('sync-rejected-dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('推送被拒绝');
  await expect(dialog).toContainText('fetch first');
  // 三条路径的文案来自后端的 i18n key（errors:actions.*）
  await expect(page.getByTestId('sync-rejected-fetch-first')).toContainText('先拉取');
  await expect(page.getByTestId('sync-rejected-force')).toContainText('覆盖远端');
  await expect(dialog.getByRole('button', { name: '取消' })).toBeVisible();

  await page.getByTestId('sync-rejected-force').click();
  await expectSyncCalls(page, 2);

  const lastSpec = await page.evaluate(() => window.__syncCalls?.[1]?.args?.spec);
  expect(lastSpec?.forceWithLease).toBe(true);
  // 对话框必须关掉：否则用户点完还以为没生效
  await expect(dialog).toHaveCount(0);

  await expectNoPageErrors(page);
});

test('拉取冲突：列出冲突文件并跳到冲突页', async ({ page }) => {
  await openRepo(page);

  await page.getByTestId('sync-pull').click();
  await expectSyncCalls(page, 1);

  // 缺省策略是"仅快进"
  const pullSpec = await page.evaluate(() => window.__syncCalls?.[0]?.args?.spec);
  expect(pullSpec?.strategy).toBe('fastForwardOnly');

  await page.evaluate(() => {
    window.__emitJob?.('job:done', {
      jobId: 'job-1',
      result: {
        remote: 'origin',
        pull: {
          fetch: { remote: 'origin', updates: [] },
          strategy: 'fastForwardOnly',
          upToDate: false,
          merge: { kind: 'conflicted', oid: null, conflicts: ['src/a.ts', 'src/b.ts'] },
        },
      },
    });
  });

  const dialog = page.getByTestId('sync-conflict-dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('拉取产生了冲突');
  await expect(page.getByTestId('sync-conflict-files')).toContainText('src/a.ts');
  await expect(page.getByTestId('sync-conflict-files')).toContainText('src/b.ts');

  await page.getByTestId('sync-conflict-guide').click();
  await expect(page).toHaveURL(/#\/repo\/1\/conflict/);

  await expectNoPageErrors(page);
});
