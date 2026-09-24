/**
 * 提交链路的交互级验收（M1 / T1.7）。
 *
 * 覆盖任务验收要求的完整路径：**写消息 → 预览 → 提交 → 状态更新**，
 * 并断言"界面填的东西原样传到后端"（提交信息与开关）以及"后端给的计划原样显示"
 * （文件清单、等价命令、钩子）。
 *
 * 为什么这条路径必须在真实浏览器里跑：预览是一个 Radix Dialog，它的打开/关闭、
 * 焦点与 aria 语义都会影响"用户能不能真的看到并点到"。jsdom 里
 * `getByRole('dialog')` 能过、真机上点不到的情况并不罕见。
 *
 * 用 mock IPC 而不是真实仓库：本用例验证的是**界面与契约**；真实 git 行为由
 * `crates/services/tests/commit.rs` 的集成测试覆盖（它跑在真实仓库上）。
 */
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

/** mock 里一份暂存文件的最小形状。 */
interface MockFile {
  readonly path: string;
  readonly indexStatus: string;
}

/** 生成 mock 脚本；`files` 决定起始的索引状态，`headPushed` 决定是否"已在远端"。 */
function mockScript(files: readonly MockFile[], headPushed = false): string {
  const entries = files.map((file) => ({
    kind: 'ordinary',
    worktreeStatus: '.',
    isBinary: false,
    isLfs: false,
    isSubmodule: false,
    sizeBytes: 12,
    ...file,
  }));

  return `
  const files = ${JSON.stringify(entries)};
  // 提交历史只保留 HEAD 一条：amend 的断言要的是"提交数不变、oid 变化"
  const commits = [{ oid: "old-oid-0000", subject: "base" }];
  const headPushed = ${headPushed ? 'true' : 'false'};
  const listeners = [];
  window.__errs = [];
  window.__commitCalls = [];
  window.__mockFiles = files;
  window.__mockCommits = commits;
  function emit(name, payload) {
    for (const listener of listeners) listener({ event: name, id: 0, payload: payload });
  }
  function report() {
    return {
      branch: { oid: "abc", head: "feat/login", detached: false, upstream: null, ahead: 0, behind: 0 },
      operation: "none",
      staged: files.filter((f) => f.indexStatus !== "."),
      unstaged: [],
      untracked: [],
      conflicted: [],
      ignored: [],
      ignoredCount: null
    };
  }
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "workspace_status") return Promise.resolve(report());
      if (command === "commit_message_hint") {
        return Promise.resolve({ recentMessages: ["feat: previous"], template: "feat: ", branchStyle: "feat" });
      }
      if (command === "commit_amend_context") {
        return Promise.resolve({
          subject: commits[0].subject,
          body: null,
          headOid: commits[0].oid,
          pushed: headPushed,
          pushedRefs: headPushed ? ["origin/main"] : []
        });
      }
      if (command === "commit_hooks_list") {
        return Promise.resolve([
          { name: "pre-commit", executable: true, commitHook: true },
          { name: "pre-push", executable: false, commitHook: false }
        ]);
      }
      if (command === "commit_prepare") {
        window.__commitCalls.push({ command: command, args: args });
        const spec = args.spec || {};
        const subject = String(spec.message || "").trim();
        window.__lastPrepare = {
          subject: subject,
          amend: !!spec.amend,
          amendMode: spec.amendMode || "includeStaged"
        };
        return Promise.resolve({
          planId: "plan-e2e",
          repoId: args.repoId,
          files: files.filter((f) => f.indexStatus !== ".").map((f) => ({ path: f.path, indexStatus: f.indexStatus })),
          message: subject + "\\n",
          description: spec.description || null,
          author: null,
          sign: spec.sign || "auto",
          signOff: !!spec.signOff,
          noVerify: !!spec.noVerify,
          amend: !!spec.amend,
          amendMode: spec.amendMode || "includeStaged",
          headPushed: !!spec.amend && headPushed,
          hooks: ["pre-commit"],
          equivalentCommand: "git commit -m \\"" + subject + "\\"",
          headOid: "abc",
          indexFingerprint: "tree-1",
          createdAtMs: 1,
          expiresAtMs: 2,
          subject: subject,
          subjectChars: subject.length,
          warnings: []
        });
      }
      if (command === "commit_execute") {
        window.__commitCalls.push({ command: command, args: args });
        const last = window.__lastPrepare || { subject: "", amend: false, amendMode: "includeStaged" };
        const paths = files.map((file) => file.path);
        if (last.amend) {
          // amend 替换 HEAD：提交数不变、oid 变化。真实的 git 语义由
          // crates/services/tests/commit.rs 在真实仓库上保证，这里只复刻形状。
          commits[0] = { oid: "amended-oid-0001", subject: last.subject };
          // "只改信息"不动索引；"并入"才把索引清空
          if (last.amendMode === "includeStaged") {
            for (const file of files) file.indexStatus = ".";
          }
        } else {
          commits.unshift({ oid: "new-oid-0001", subject: last.subject });
          for (const file of files) file.indexStatus = ".";
        }
        emit("repo:changed", { repoId: 1, paths: paths });
        return Promise.resolve({ oid: commits[0].oid, subject: commits[0].subject, snapshotId: null, paths: paths });
      }
      if (command === "plugin:event|unlisten") return Promise.resolve(null);
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_get") return Promise.resolve(null);
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;
}

/** 断言页面自身没有未捕获错误（每次交互结束都要过的一道闸）。 */
async function expectNoPageErrors(page: Page) {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

test('写消息 → 预览 → 提交 → 状态更新（__errs 为空）', async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(
    mockScript([
      { path: 'src/a.ts', indexStatus: 'M' },
      { path: 'src/b.ts', indexStatus: 'A' },
    ]),
  );

  await page.goto('/#/repo/1/commit');
  await expect(page.getByTestId('commit-panel')).toBeVisible();

  // 已暂存数量与提示都来自后端（提示是纯本地规则，不是 AI 生成）
  await expect(page.getByText('已暂存 2 个文件')).toBeVisible();
  await expect(page.getByRole('button', { name: '用「feat: 」开头' })).toBeVisible();

  await page.getByLabel('提交信息').fill('feat: e2e commit');

  // 预览：文件清单、等价命令、钩子列表都在这一屏
  await page.getByTestId('commit-submit').click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText('src/a.ts')).toBeVisible();
  await expect(dialog.getByText('src/b.ts')).toBeVisible();
  await expect(dialog.getByText('git commit -m "feat: e2e commit"')).toBeVisible();
  await expect(dialog.getByText('pre-commit')).toBeVisible();

  await dialog.getByRole('button', { name: '提交', exact: true }).click();

  // Radix Toast 会同时渲染可见元素与 aria-live 通知元素，因此这里取第一个
  await expect(page.getByText('已提交：feat: e2e commit').first()).toBeVisible();

  // 契约：界面填的信息与开关原样传到后端；执行时用的是预览里的那个计划 id
  const calls = await page.evaluate(() => window.__commitCalls ?? []);
  expect(calls[0]).toMatchObject({
    command: 'commit_prepare',
    args: { repoId: 1, spec: { message: 'feat: e2e commit', amend: false, noVerify: false } },
  });
  expect(calls.at(-1)).toMatchObject({
    command: 'commit_execute',
    args: { planId: 'plan-e2e' },
  });

  // 状态更新：索引被清空，面板随之显示"还没有暂存任何改动"
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText('还没有暂存任何改动')).toBeVisible();

  await expectNoPageErrors(page);
});

test('没有暂存内容时提交按钮禁用并说明原因', async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(mockScript([]));

  await page.goto('/#/repo/1/commit');

  await expect(page.getByText('还没有暂存任何改动')).toBeVisible();
  await expect(page.getByTestId('commit-submit')).toBeDisabled();
  // 只禁用不解释是"界面看起来坏了"的经典来源：原因与出口都要在
  await expect(page.getByText('提交只包含已暂存的内容')).toBeVisible();

  await expectNoPageErrors(page);
});

test('amend 模式下没有暂存内容也能提交（只改信息）', async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(mockScript([]));

  await page.goto('/#/repo/1/commit');
  await expect(page.getByText('还没有暂存任何改动')).toBeVisible();

  await page.getByRole('checkbox', { name: 'Amend 上一次提交' }).check();
  await page.getByLabel('提交信息').fill('fix: message only');

  const submit = page.getByTestId('commit-submit');
  await expect(submit).toBeEnabled();
  await submit.click();

  await expect(page.getByRole('dialog')).toBeVisible();
  const calls = await page.evaluate(() => window.__commitCalls ?? []);
  expect(calls.at(-1)).toMatchObject({
    command: 'commit_prepare',
    args: { spec: { amend: true, message: 'fix: message only' } },
  });

  await expectNoPageErrors(page);
});

test('amend 只改信息：提交数不变、oid 变化、暂存内容原样保留', async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(mockScript([{ path: 'src/a.ts', indexStatus: 'M' }], true));

  await page.goto('/#/repo/1/commit');
  await expect(page.getByText('已暂存 1 个文件')).toBeVisible();

  await page.getByRole('checkbox', { name: 'Amend 上一次提交' }).check();
  // 勾选后自动填入上一次提交的信息（在输入为空时）
  await expect(page.getByLabel('提交信息')).toHaveValue('base');
  // "可能已推送"必须给出后果说明，而不是只给一个开关
  await expect(page.getByText(/force-with-lease/)).toBeVisible();

  await page.getByRole('radio', { name: '只改提交信息' }).click();
  await page.getByLabel('提交信息').fill('fix: typo');

  await page.getByTestId('commit-submit').click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: '提交', exact: true }).click();

  const calls = await page.evaluate(() => window.__commitCalls ?? []);
  expect(calls[0]).toMatchObject({
    command: 'commit_prepare',
    args: { spec: { amend: true, amendMode: 'messageOnly', message: 'fix: typo' } },
  });

  const state = await page.evaluate(() => ({
    commits: window.__mockCommits,
    files: window.__mockFiles,
  }));
  expect(state.commits, 'amend 不增加提交数').toHaveLength(1);
  expect(state.commits?.[0]?.oid, '提交被替换，oid 必须变化').not.toBe('old-oid-0000');
  expect(state.commits?.[0]?.subject).toBe('fix: typo');
  expect(state.files?.[0]?.indexStatus, '只改信息不动索引').toBe('M');
  await expect(page.getByText('已暂存 1 个文件')).toBeVisible();

  await expectNoPageErrors(page);
});

test('amend 并入暂存内容：索引随之变干净', async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(mockScript([{ path: 'src/a.ts', indexStatus: 'M' }]));

  await page.goto('/#/repo/1/commit');
  await expect(page.getByText('已暂存 1 个文件')).toBeVisible();

  await page.getByRole('checkbox', { name: 'Amend 上一次提交' }).check();
  await page.getByLabel('提交信息').fill('feat: fold in');
  // 默认模式就是"并入暂存内容"（= git 的行为），无需额外选择
  await page.getByTestId('commit-submit').click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: '提交', exact: true }).click();

  const calls = await page.evaluate(() => window.__commitCalls ?? []);
  expect(calls[0]).toMatchObject({
    command: 'commit_prepare',
    args: { spec: { amend: true, amendMode: 'includeStaged' } },
  });

  const state = await page.evaluate(() => window.__mockCommits);
  expect(state).toHaveLength(1);
  await expect(page.getByText('还没有暂存任何改动')).toBeVisible();

  await expectNoPageErrors(page);
});
