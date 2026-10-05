import { expect, test, type Page } from '@playwright/test';

/**
 * 诊断 UI 的交互级验收（T5.6）。
 *
 * 表面：远端同步条的拉取（与生产一致的失败路径——`job:failed` 携带
 * 已脱敏 stderr，错误 Toast 出现后异步诊断挂到同一条上）。
 *
 * mock 的 `system_diagnose_error` 返回与后端规则引擎相同形状的报告
 * （后端匹配逻辑由 `crates/diagnostics/tests/rules.rs` 的 51 组 fixture 覆盖，
 * 这里验证的是**界面半条链路**：诊断挂载 → 卡片渲染 → 修复动作分派）。
 *
 * 覆盖任务书 E2E 验收：① non-fast-forward → 卡片 → 先抓取再重试 → 成功；
 * ② 认证失败 → 卡片 → 检查 SSH 配置 → 跳设置页；③ "这个诊断不对" → 反馈。
 */
const DIAG_MOCK = `
  window.__errs = [];
  window.__syncCalls = [];
  window.__diagCalls = [];
  window.__openUrls = [];
  window.__scanReports = { push: true, ssh: true };
  window.__syncJobs = 0;
  const callbacks = {};
  const eventOf = {};
  let nextCallbackId = 1;
  const branches = [
    { name: "main", isRemote: false, isHead: true, target: "a1b2c3", upstream: "origin/main", ahead: 0, behind: 3, upstreamGone: false }
  ];
  const STDERR_PUSH = "! [rejected] main -> main (fetch first)\\nerror: failed to push some refs (non-fast-forward)";
  const STDERR_SSH = "git@github.com: Permission denied (publickey).\\nfatal: Could not read from remote repository.";
  window.__emitJob = function (event, payload) {
    for (const key of Object.keys(eventOf)) {
      if (eventOf[key] === event) {
        callbacks[key]({ event: event, id: Number(key), payload: payload });
      }
    }
  };
  function report(kind) {
    if (kind === "ssh") {
      return { primary: { id: "ssh-publickey-denied", confidence: 0.92,
        titleKey: "diag.ssh-publickey-denied.title", explanationKey: "diag.ssh-publickey-denied.explanation",
        causes: ["diag.ssh-publickey-denied.cause1"],
        fixes: [{ id: "check-ssh", labelKey: "diag.ssh-publickey-denied.fix_check_ssh",
          action: { kind: "guide", value: "/settings/advanced" } }] },
        alternatives: [], rawSummary: STDERR_SSH };
    }
    return { primary: { id: "push-non-fast-forward", confidence: 0.95,
      titleKey: "diag.push-non-fast-forward.title", explanationKey: "diag.push-non-fast-forward.explanation",
      causes: ["diag.push-non-fast-forward.cause1"],
      fixes: [
        { id: "fetch-then-retry", labelKey: "diag.push-non-fast-forward.fix_fetch_then_retry",
          action: { kind: "command", command: "git_fetch", args: {} } },
        { id: "use-force-with-lease", labelKey: "diag.push-non-fast-forward.fix_use_force_with_lease",
          action: { kind: "dangerous", command: "git_push", args: { force_with_lease: true } } }
      ] },
      alternatives: [], rawSummary: STDERR_PUSH };
  }
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { const id = nextCallbackId; nextCallbackId += 1; callbacks[id] = callback; return id; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === "plugin:event|listen") { eventOf[args.handler] = args.event; return Promise.resolve(nextCallbackId); }
      if (command === "plugin:event|unlisten") return Promise.resolve(null);
      if (command === "repo_recent_list") return Promise.resolve([{ record: { id: 1, path: "/tmp/repo", name: "repo" }, isOpen: true }]);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "settings_all" || command === "settings_get") return Promise.resolve(null);
      if (command === "settings_set") return Promise.resolve(null);
      if (command === "git_branch_list") return Promise.resolve(branches);
      if (command === "git_branch_compare") return Promise.resolve({ ahead: 0, behind: 3, onlyInA: [] });
      if (command === "workspace_status") return Promise.resolve({
        branch: { oid: "a1b2c3", head: "main", detached: false, upstream: "origin/main", ahead: 0, behind: 3 },
        operation: "none", staged: [], unstaged: [], untracked: [], conflicted: [], ignored: [], ignoredCount: null
      });
      if (command === "git_pull") {
        window.__syncJobs += 1;
        window.__syncCalls.push({ command: command, args: args });
        return Promise.resolve({ jobId: "job-pull" });
      }
      if (command === "git_fetch") {
        window.__syncJobs += 1;
        window.__syncCalls.push({ command: command, args: args });
        const jobId = "job-" + window.__syncJobs;
        setTimeout(function () {
          window.__emitJob("job:done", { jobId: jobId, result: { remote: "origin", updates: [] } });
        }, 30);
        return Promise.resolve({ jobId: jobId });
      }
      if (command === "system_diagnose_error") {
        window.__diagCalls.push(args.stderr);
        const kind = String(args.stderr).indexOf("publickey") >= 0 ? "ssh" : "push";
        return Promise.resolve(window.__scanReports[kind] ? report(kind) : { primary: null, alternatives: [], rawSummary: args.stderr });
      }
      if (command === "system_open_url") {
        window.__openUrls.push(args.url);
        return Promise.resolve(null);
      }
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

async function openRepo(page: Page): Promise<void> {
  await page.goto('/#/repo/1/status');
  await expect(page.getByTestId('sync-bar')).toBeVisible();
}

async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

test('① 拉取产生 non-fast-forward：诊断卡片出现，先抓取再重试成功', async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
  await page.addInitScript(DIAG_MOCK);
  await openRepo(page);

  await page.getByTestId('sync-pull').click();
  // 拉取任务失败（携带已脱敏 stderr）
  await page.evaluate(() => {
    window.__emitJob?.('job:failed', {
      jobId: 'job-pull',
      error: {
        code: 'NETWORK',
        message: 'the remote has commits you do not have',
        detail:
          '! [rejected] main -> main (fetch first)\\nerror: failed to push some refs (non-fast-forward)',
      },
    });
  });

  // 诊断卡片挂到错误 Toast 上：标题 + 置信度 + 修复按钮
  await expect(page.getByText('推送被拒绝：远端有新提交')).toBeVisible();
  await expect(page.getByText('高置信度')).toBeVisible();

  // 先抓取再重试：git_fetch 真的发出去，随后成功
  await page.getByRole('button', { name: '先抓取再重试（推荐）' }).click();
  await expect
    .poll(() => page.evaluate(() => (window.__syncCalls ?? []).map((c) => c.command)))
    .toContain('git_fetch');
  await expect(page.getByText(/修复动作已完成/)).toBeVisible();

  await expectNoPageErrors(page);
});

test('② 认证失败：诊断卡片提供检查 SSH 配置，跳转设置页', async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
  await page.addInitScript(DIAG_MOCK);
  await openRepo(page);

  await page.getByTestId('sync-pull').click();
  await page.evaluate(() => {
    window.__emitJob?.('job:failed', {
      jobId: 'job-pull',
      error: {
        code: 'SSH_KEY_REJECTED',
        message: 'the remote rejected the public key',
        detail:
          'git@github.com: Permission denied (publickey).\\nfatal: Could not read from remote repository.',
      },
    });
  });

  await expect(page.getByText('SSH 公钥被拒绝')).toBeVisible();
  await page.getByRole('button', { name: '检查 SSH 配置' }).click();

  // guide 动作：页面内跳转（不离开应用）
  await expect(page).toHaveURL(/#\/settings\/advanced/);
  await expectNoPageErrors(page);
});

test('③ "这个诊断不对"生成预填的反馈链接（需用户点击，不自动打开）', async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
  await page.addInitScript(DIAG_MOCK);
  await openRepo(page);

  await page.getByTestId('sync-pull').click();
  await page.evaluate(() => {
    window.__emitJob?.('job:failed', {
      jobId: 'job-pull',
      error: {
        code: 'NETWORK',
        message: 'the remote has commits you do not have',
        detail:
          '! [rejected] main -> main (fetch first)\\nerror: failed to push some refs (non-fast-forward)',
      },
    });
  });
  await expect(page.getByText('推送被拒绝：远端有新提交')).toBeVisible();

  await page.getByRole('button', { name: '这个诊断不对' }).click();

  await expect
    .poll(async () => (await page.evaluate(() => window.__openUrls)) ?? [])
    .toContain(
      'https://github.com/Ember1414/forgedesk/issues/new?labels=diagnostics&title=%5Bdiagnostics%5D%20wrong%20diagnosis',
    );

  await expectNoPageErrors(page);
});
