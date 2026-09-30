/**
 * M1 闭环交互级验收（T1.12 第 2 条）。
 *
 * 四条路径各跑一次**端到端**（同一份 mock 状态串起来），每条结束都断言
 * `window.__errs` 为空：
 *
 *   1. 打开仓库 → 分组展示 → 行级暂存 → 提交 → 历史更新；
 *   2. 放弃（discard）的**取消**与**确认**两条路径；
 *   3. 提交被 pre-commit 钩子拒绝 → 展示钩子输出 → 无半成品提交；
 *   4. `reset --hard` 之后从快照回滚 → 文件恢复。
 *
 * 为什么是 mock IPC 而不是真实仓库：本文件验证的是**界面与契约的闭环**
 * （点得到、看得到、传得对、状态跟着变），真实 git 语义由
 * `crates/services/tests/*.rs` 与 `crates/snapshot/tests/*.rs` 在真实仓库上保证。
 * 两者的分工写在 `docs/CODING_STYLE.md` §3.6。
 *
 * 为什么必须在真实浏览器里跑：抽屉、对话框的打开/关闭与 aria 语义决定
 * "用户能不能真的看到并点到"，而这些在 jsdom 里常常看起来是通的。
 */
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

/** 断言页面自身没有未捕获错误（每个用例的最后一道闸）。 */
async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

async function pinLanguage(page: Page): Promise<void> {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
}

/**
 * 一份把 M1 各条链路串起来的 mock。
 *
 * `hookRejects` 决定 `commit_execute` 是成功还是被钩子拒绝；
 * `restored` 由 `snapshot_restore` 置位，用来表现"回滚之后工作区变干净"。
 */
function mockScript(hookRejects: boolean): string {
  return `
  const files = [
    { path: "src/loop.ts", kind: "ordinary", indexStatus: ".", worktreeStatus: "M", isBinary: false, isLfs: false, isSubmodule: false, sizeBytes: 40 },
    { path: "src/other.ts", kind: "ordinary", indexStatus: ".", worktreeStatus: "M", isBinary: false, isLfs: false, isSubmodule: false, sizeBytes: 20 }
  ];
  const commits = [{ oid: "base-oid-0000", subject: "base" }];
  const snapshots = [
    { id: 2, label: "pre-commit", kind: "pre-commit", headOid: "2222222222222222222222222222222222222222", branch: "main", detached: false, createdAtMs: 1700000002000 }
  ];
  const listeners = [];
  let restored = false;
  window.__errs = [];
  window.__loop = { calls: [], files: files, commits: commits, hookRejects: ${hookRejects ? 'true' : 'false'} };
  window.__mockFiles = files;
  window.__mockCommits = commits;
  function emit(name, payload) {
    for (const listener of listeners) listener({ event: name, id: 0, payload: payload });
  }
  function report() {
    return {
      branch: { oid: "abc", head: "main", detached: false, upstream: null, ahead: 0, behind: 0 },
      operation: "none",
      staged: files.filter((f) => f.indexStatus !== "."),
      unstaged: files.filter((f) => f.worktreeStatus === "M"),
      untracked: [],
      conflicted: [],
      ignored: [],
      ignoredCount: null
    };
  }
  function stagedFiles() {
    return files.filter((f) => f.indexStatus !== ".");
  }
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "workspace_status") return Promise.resolve(report());
      if (command === "workspace_diff") {
        return Promise.resolve({
          files: [{
            path: (args.spec && args.spec.paths && args.spec.paths[0]) || "src/loop.ts",
            binary: false,
            truncated: false,
            added: 1,
            removed: 1,
            hunks: [{
              header: "@@ -1,3 +1,3 @@",
              oldStart: 1, oldLines: 3, newStart: 1, newLines: 3,
              lines: [
                { kind: "context", content: "const value = 1;", oldNo: 1, newNo: 1 },
                { kind: "removed", content: "const old = 2;", oldNo: 2, newNo: null },
                { kind: "added", content: "const next = 2;", oldNo: null, newNo: 2 }
              ]
            }]
          }]
        });
      }
      if (command === "workspace_stage" || command === "workspace_unstage") {
        const spec = args.spec || { kind: "files", paths: [] };
        const paths = spec.kind === "files" ? (spec.paths || []) : [spec.path];
        (window.__loop?.calls ?? []).push({ command: command, spec: spec, paths: paths });
        for (const path of paths) {
          const file = files.find((x) => x.path === path);
          if (!file) continue;
          if (command === "workspace_stage") {
            file.indexStatus = "M";
            // 行级 / 块级暂存之后工作区仍有改动：文件同时出现在两组里（真实行为）
            file.worktreeStatus = spec.kind === "files" ? "." : "M";
          } else {
            file.indexStatus = ".";
            file.worktreeStatus = "M";
          }
        }
        emit("repo:changed", { repoId: 1, kind: "workspace", paths: paths });
        return Promise.resolve(null);
      }
      if (command === "workspace_discard") {
        const spec = args.spec || { kind: "files", tracked: [], untracked: [] };
        (window.__loop?.calls ?? []).push({ command: command, spec: spec, paths: spec.kind === "files" ? [] : [spec.path] });
        if (spec.kind !== "files") {
          const file = files.find((x) => x.path === spec.path);
          if (file) file.worktreeStatus = ".";
        }
        emit("repo:changed", { repoId: 1, kind: "workspace", paths: [spec.path || ""] });
        return Promise.resolve(null);
      }
      if (command === "commit_message_hint") {
        return Promise.resolve({ recentMessages: commits.map((c) => c.subject), template: "feat: ", branchStyle: "feat" });
      }
      if (command === "commit_amend_context") {
        return Promise.resolve({ subject: commits[0].subject, body: null, headOid: commits[0].oid, pushed: false, pushedRefs: [] });
      }
      if (command === "commit_hooks_list") {
        return Promise.resolve([{ name: "pre-commit", executable: true, commitHook: true }]);
      }
      if (command === "commit_prepare") {
        (window.__loop?.calls ?? []).push({ command: command, args: { message: (args.spec || {}).message } });
        window.__loop.prepared = String((args.spec || {}).message || "").trim();
        return Promise.resolve({
          planId: "plan-loop", repoId: args.repoId,
          files: stagedFiles().map((f) => ({ path: f.path, indexStatus: f.indexStatus })),
          message: window.__loop.prepared + "\\n", description: null, author: null,
          sign: "auto", signOff: false, noVerify: false, amend: false,
          hooks: ["pre-commit"], equivalentCommand: 'git commit -m "' + window.__loop.prepared + '"',
          headOid: "base-oid-0000", indexFingerprint: "tree-1",
          createdAtMs: 1, expiresAtMs: 2, subject: window.__loop.prepared,
          subjectChars: window.__loop.prepared.length, warnings: []
        });
      }
      if (command === "commit_execute") {
        (window.__loop?.calls ?? []).push({ command: command, args: args });
        if (window.__loop.hookRejects) {
          // 钩子拒绝：git 的原始输出进 detail，钩子名进 hint（界面照着展示）
          return Promise.reject({
            code: "HOOK_REJECTED",
            message: "pre-commit hook rejected the commit",
            detail: "lint: 3 errors in src/loop.ts",
            hint: "pre-commit"
          });
        }
        const paths = stagedFiles().map((f) => f.path);
        commits.unshift({ oid: "new-oid-0001", subject: window.__loop.prepared });
        for (const file of files) file.indexStatus = ".";
        emit("repo:changed", { repoId: 1, kind: "refs", paths: paths });
        return Promise.resolve({ oid: "new-oid-0001", subject: window.__loop.prepared, snapshotId: 7, paths: paths });
      }
      if (command === "snapshot_list") return Promise.resolve(restored ? [] : snapshots);
      if (command === "snapshot_usage") return Promise.resolve({
        repoId: 1, snapshotCount: restored ? 0 : 1, backupBytes: 0,
        maxSnapshotBytes: 209715200, maxRepoBytes: 2147483648, orphanDirs: []
      });
      if (command === "snapshot_diff") {
        // 模拟"用户在终端里 reset --hard 过"：HEAD 与索引都变了
        return Promise.resolve({
          headChanged: true, indexChanged: true,
          currentHeadOid: "9999999999999999999999999999999999999999",
          currentIndexTreeOid: "8888888888888888888888888888888888888888",
          refMissing: false,
          // T3.8：未跟踪内容的三分类（这条闭环里没有未跟踪文件）
          untrackedRestorable: [], untrackedMissing: [], untrackedExtra: []
        });
      }
      if (command === "snapshot_restore") {
        (window.__loop?.calls ?? []).push({ command: command, args: args });
        restored = true;
        // 回滚把工作区带回快照时的样子：两个文件都不再是脏的
        for (const file of files) { file.indexStatus = "."; file.worktreeStatus = "."; }
        emit("repo:changed", { repoId: 1, kind: "refs", paths: files.map((f) => f.path) });
        return Promise.resolve({
          restoredSnapshotId: args.snapshotId,
          headOid: "2222222222222222222222222222222222222222",
          indexTreeOid: "7777777777777777777777777777777777777777",
          preRestoreSnapshotId: 3,
          untrackedPaths: [],
          untrackedRestored: 0,
          untrackedFailed: [],
          untrackedExtra: [],
          verified: true
        });
      }
      if (command === "plugin:event|unlisten") return Promise.resolve(null);
      if (command === "repo_recent_list") return Promise.resolve([
        { id: 1, path: "/tmp/repo", name: "repo", defaultBranch: "main", lastOpenedAt: 1, createdAt: 1, isOpen: true }
      ]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_get") return Promise.resolve(null);
      if (command === "settings_all") return Promise.resolve({});
      if (command === "logs_tail") return Promise.resolve([]);
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;
}

test('闭环 1：行级暂存 → 提交 → 历史更新', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(mockScript(false));

  // 打开仓库 → 分组展示
  await page.goto('/#/repo/1/status');
  await expect(page.getByRole('button', { name: /未暂存 2/ })).toBeVisible();

  // 行级暂存：点开文件 → 选一行 → 暂存
  await page.getByRole('button', { name: 'loop.ts', exact: true }).click();
  const diff = page.getByTestId('diff-view');
  await expect(diff).toBeVisible();
  await diff.getByRole('button', { name: '选择第 2 行' }).first().click();
  await expect(diff.getByText('已选 1 行')).toBeVisible();
  await diff.getByRole('button', { name: '暂存选中行' }).click();

  const staged = await page.evaluate(() =>
    (window.__loop?.calls ?? []).filter((c) => c.command === 'workspace_stage'),
  );
  expect(staged.at(-1)?.spec).toMatchObject({ kind: 'lines', path: 'src/loop.ts' });

  await page.getByRole('button', { name: '关闭 diff' }).click();
  // 部分暂存：文件同时出现在两组（索引有内容、工作区也有改动）
  await expect(page.getByRole('button', { name: /已暂存 1/ })).toBeVisible();
  await expect(page.getByRole('button', { name: /未暂存 2/ })).toBeVisible();

  // 提交：信息 → 预览 → 执行
  await page.goto('/#/repo/1/commit');
  await expect(page.getByTestId('commit-panel')).toBeVisible();
  await expect(page.getByText('已暂存 1 个文件')).toBeVisible();
  await page.getByLabel('提交信息').fill('feat: loop commit');
  await page.getByTestId('commit-submit').click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText('src/loop.ts')).toBeVisible();
  await dialog.getByRole('button', { name: '提交', exact: true }).click();

  // 历史更新：新提交进入提示里的最近列表，索引被清空
  await expect(page.getByText('feat: loop commit').first()).toBeVisible({ timeout: 10_000 });
  const state = await page.evaluate(() => ({
    commits: window.__mockCommits?.map((c) => c.subject),
    index: window.__mockFiles?.map((f) => f.indexStatus),
  }));
  expect(state.commits?.[0]).toBe('feat: loop commit');
  expect(state.index).toEqual(['.', '.']);

  await expectNoPageErrors(page);
});

test('闭环 2：放弃修改的取消与确认两条路径', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(mockScript(false));

  await page.goto('/#/repo/1/status');
  await page.getByRole('button', { name: 'loop.ts', exact: true }).click();
  const diff = page.getByTestId('diff-view');
  await diff.getByRole('button', { name: '选择第 2 行' }).first().click();

  // 路径一：取消。**什么都不能发生**
  await diff.getByRole('button', { name: '放弃选中行' }).click();
  const cancelDialog = page.getByRole('alertdialog');
  await expect(cancelDialog).toBeVisible();
  // 确认框必须说清影响（红线 R7 的 UI 闸门）
  await expect(cancelDialog.getByText(/已暂存的内容不受影响/)).toBeVisible();
  await cancelDialog.getByRole('button', { name: '取消' }).click();
  await expect(cancelDialog).toHaveCount(0);

  const afterCancel = await page.evaluate(() => ({
    discards: (window.__loop?.calls ?? []).filter((c) => c.command === 'workspace_discard').length,
    worktree: window.__mockFiles?.map((f) => f.worktreeStatus),
  }));
  expect(afterCancel.discards).toBe(0);
  expect(afterCancel.worktree).toEqual(['M', 'M']);

  // 路径二：确认
  await diff.getByRole('button', { name: '放弃选中行' }).click();
  const confirmDialog = page.getByRole('alertdialog');
  await confirmDialog.getByRole('button', { name: '放弃' }).click();

  await expect
    .poll(async () =>
      page.evaluate(
        () => (window.__loop?.calls ?? []).filter((c) => c.command === 'workspace_discard').length,
      ),
    )
    .toBe(1);
  const spec = await page.evaluate(
    () => (window.__loop?.calls ?? []).find((c) => c.command === 'workspace_discard')?.spec,
  );
  expect(spec).toMatchObject({ kind: 'lines', path: 'src/loop.ts' });

  await expectNoPageErrors(page);
});

test('闭环 3：钩子拒绝时展示输出且不留半成品提交', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(mockScript(true));

  await page.goto('/#/repo/1/status');
  // 先暂存一个块，让提交有内容可提交
  await page.getByRole('button', { name: 'loop.ts', exact: true }).click();
  await page.getByTestId('diff-view').getByRole('button', { name: '暂存此块' }).first().click();
  // 断言落在地上：后端确实收到了块级请求，且索引真的变了
  await expect
    .poll(async () =>
      page.evaluate(() => ({
        staged: (window.__loop?.calls ?? []).filter((c) => c.command === 'workspace_stage').length,
        index: window.__mockFiles?.[0]?.indexStatus,
      })),
    )
    .toEqual({ staged: 1, index: 'M' });

  await page.goto('/#/repo/1/commit');
  await page.getByLabel('提交信息').fill('feat: rejected commit');
  await page.getByTestId('commit-submit').click();
  await page.getByRole('dialog').getByRole('button', { name: '提交', exact: true }).click();

  // 确实发起了提交（否则下面的"没有半成品"就是空断言）
  await expect
    .poll(async () =>
      page.evaluate(
        () => (window.__loop?.calls ?? []).filter((c) => c.command === 'commit_execute').length,
      ),
    )
    .toBe(1);

  // 钩子的输出必须展示出来：**原文一字不改**（detail 就是 git 的原始输出），
  // 同时给出"跳过钩子重试"这个出口（跳过钩子必须由用户自己再确认一次）
  await expect(page.getByText('钩子输出').first()).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText(/lint: 3 errors in src\/loop\.ts/).first()).toBeVisible();
  await expect(page.getByRole('button', { name: '跳过钩子重试' })).toBeVisible();
  // 对话框不自动关闭：用户要能看着输出决定下一步
  await expect(page.getByRole('dialog')).toBeVisible();

  // 无半成品提交：历史没有新提交、索引内容还在（下次还能接着提交）
  const state = await page.evaluate(() => ({
    commits: window.__mockCommits?.map((c) => c.subject),
    index: window.__mockFiles?.map((f) => f.indexStatus),
  }));
  expect(state.commits).toEqual(['base']);
  expect(state.index?.[0]).toBe('M');

  await expectNoPageErrors(page);
});

test('闭环 4：reset --hard 之后从快照回滚，文件恢复', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(mockScript(false));

  await page.goto('/#/repo/1/snapshots');
  await expect(page.getByTestId('snapshots-page')).toBeVisible();

  await page.getByRole('button', { name: '回滚到这里' }).first().click();
  const dialog = page.getByRole('alertdialog');
  // 确认框先展示"将会发生什么"：HEAD 从哪里回到哪里
  await expect(dialog.getByText(/HEAD 将从 9999999 回到 2222222/)).toBeVisible();
  await dialog.getByRole('button', { name: '回滚', exact: true }).click();

  await expect(page.getByText('已回滚到 2222222').first()).toBeVisible({ timeout: 10_000 });

  // 文件恢复：工作区重新干净（回滚把两个文件都带回去了）
  await page.goto('/#/repo/1/status');
  await expect(page.getByText('没有未提交的修改')).toBeVisible({ timeout: 10_000 });

  await expectNoPageErrors(page);
});
