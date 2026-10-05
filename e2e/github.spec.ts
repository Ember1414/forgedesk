/**
 * 代码托管（GitHub 集成）的交互级验收（M4 / T4.12 第 4 项）。
 *
 * # 范围
 *
 * 契约测试（`crates/services/tests/contract_tests.rs`）钉住的是"请求打成什么样、
 * 响应怎么解析、错误码怎么映射"；本文件钉住的是**界面上的那条链路**：
 * 登录 → 仓库列表 → 打开 PR → 查看 diff → 行内评论 → 提交 review。
 * 两者缺一不可：契约全绿也可能点不出那条路（这正是 T2.8 的 stash 崩溃漏网的原因）。
 *
 * # 为什么全部走 mock
 *
 * M4 的验收标准里"PR 全流程在真实 GitHub 上验证"必须由人类用真实账号执行
 * （见 `docs/acceptance/M4.md` §4），**不允许**测试替用户向真实仓库写数据。
 * 这里只验证前端在正确响应下的行为，以及错误码（AUTH_REQUIRED）下的引导。
 */
import { expect, test } from '@playwright/test';

const MOCK_SCRIPT = `
  const PR_NUMBER = 7;
  const account = {
    id: 'acc-1',
    provider: 'github',
    host: 'github.com',
    login: 'octocat',
    scopes: ['repo'],
    createdAt: 1700000000000
  };
  const repo = {
    id: 11,
    owner: 'octo',
    name: 'demo',
    fullName: 'octo/demo',
    description: 'fixture repository',
    htmlUrl: 'https://github.com/octo/demo',
    defaultBranch: 'main',
    private: false,
    fork: false,
    stars: 12,
    pushedAt: '2026-09-30T10:00:00Z'
  };
  const summary = {
    number: PR_NUMBER,
    title: 'Add feature flag',
    state: 'open',
    draft: false,
    merged: false,
    author: 'octocat',
    headLabel: 'octocat:feature',
    baseLabel: 'octo:main',
    htmlUrl: 'https://github.com/octo/demo/pull/7',
    createdAt: '2026-09-29T10:00:00Z',
    updatedAt: '2026-09-30T10:00:00Z'
  };
  const detail = {
    number: PR_NUMBER,
    title: 'Add feature flag',
    state: 'open',
    draft: false,
    merged: false,
    author: 'octocat',
    headLabel: 'octocat:feature',
    baseLabel: 'octo:main',
    headSha: '1234567890123456789012345678901234567890',
    htmlUrl: 'https://github.com/octo/demo/pull/7',
    bodyHtml: '<p>启用 <code>beta</code> 开关。</p>',
    changedFiles: 1,
    additions: 1,
    deletions: 0,
    mergeable: true,
    mergeableState: 'clean',
    createdAt: '2026-09-29T10:00:00Z',
    updatedAt: '2026-09-30T10:00:00Z'
  };
  const file = {
    filename: 'src/app.ts',
    previousFilename: null,
    status: 'modified',
    additions: 1,
    deletions: 0,
    changes: 1,
    patch: null,
    hunks: [
      {
        oldStart: 10,
        oldLines: 1,
        newStart: 10,
        newLines: 2,
        header: 'export const a = 1;',
        lines: [
          { kind: 'context', content: 'const a = 1;', oldNo: 10, newNo: 10 },
          { kind: 'added', content: 'const b = 2;', oldNo: null, newNo: 11 }
        ]
      }
    ]
  };

  window.__errs = [];
  window.__githubCalls = [];
  window.__githubAuthRequired = false;
  const listeners = [];

  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      window.__githubCalls.push({ command: command, args: args });
      if (command === 'account_list') {
        return Promise.resolve(window.__githubAuthRequired ? [] : [account]);
      }
      if (command === 'repo_remote_list') {
        if (window.__githubAuthRequired) {
          return Promise.reject({ code: 'AUTH_REQUIRED', message: 'no account is connected' });
        }
        return Promise.resolve({ items: [repo], nextPage: null });
      }
      if (command === 'repo_remote_starred') return Promise.resolve({ items: [], nextPage: null });
      if (command === 'repo_remote_search') return Promise.resolve({ items: [repo], nextPage: null });
      if (command === 'repo_account_binding_get') return Promise.resolve(null);
      if (command === 'repo_rate_limit_state') return Promise.resolve(null);
      // 设置页会同时挂上凭据面板与 SSH 面板：它们的 DTO 都是**非空**形状，
      // 返回 null 会当场炸掉整个页面（DTO 契约见 docs/API.md「凭据」节）
      if (command === 'credentials_status') {
        return Promise.resolve({
          backend: 'systemKeyring',
          mode: 'systemKeyring',
          count: 0,
          vaultExists: false
        });
      }
      if (command === 'credentials_list') return Promise.resolve([]);
      if (command === 'credentials_ssh_inventory') {
        return Promise.resolve({
          directory: '/home/octocat/.ssh',
          keys: [],
          agent: { kind: 'notRunning' }
        });
      }
      if (command === 'repo_pull_list') {
        if (window.__githubAuthRequired) {
          return Promise.reject({ code: 'AUTH_REQUIRED', message: 'no account is connected' });
        }
        return Promise.resolve({ items: [summary], nextPage: null });
      }
      if (command === 'repo_pull_get') return Promise.resolve(detail);
      if (command === 'repo_pull_reviews') {
        return Promise.resolve([
          { id: 1, author: 'hubot', state: 'APPROVED', body: '看起来不错', submittedAt: '2026-09-30T09:00:00Z' }
        ]);
      }
      if (command === 'repo_pull_comments_list') {
        return Promise.resolve([
          { id: 20, author: 'hubot', body: '记得补测试', createdAt: '2026-09-30T09:30:00Z' }
        ]);
      }
      if (command === 'repo_pull_comment_create') {
        return Promise.resolve({ id: 21, author: 'octocat', body: (args && args.body) || '', createdAt: null });
      }
      if (command === 'repo_pull_review_submit') return Promise.resolve(null);
      if (command === 'repo_pull_files') return Promise.resolve({ items: [file], nextPage: null });
      if (command === 'repo_pull_review_comments_list') return Promise.resolve([]);
      if (command === 'repo_pull_review_comment_create') {
        return Promise.resolve({
          id: 30,
          inReplyTo: null,
          author: 'octocat',
          body: (args && args.body) || '',
          path: args && args.path,
          side: args && args.side,
          line: args && args.line,
          createdAt: null
        });
      }
      if (command === 'repo_recent_list') {
        return Promise.resolve([{ record: { id: 1, path: '/tmp/repo', name: 'repo' }, isOpen: true }]);
      }
      if (command === 'app_version') return Promise.resolve({ version: '0.0.1', gitDescribe: null });
      if (command === 'settings_get') return Promise.resolve(null);
      if (command === 'plugin:event|unlisten') return Promise.resolve(null);
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('forgedesk.language', 'zh-CN'));
  await page.addInitScript(MOCK_SCRIPT);
});

test('登录状态 → 仓库列表 → 打开 PR → 查看 diff → 行内评论 → 提交 review（T4.12）', async ({
  page,
}) => {
  // 1) 登录（mock）：账号页列出已登录账号（令牌不出现在界面上）
  await page.goto('/#/settings/github');
  await expect(page.getByTestId('account-items')).toBeVisible();
  await expect(page.getByTestId('account-item')).toContainText('octocat');

  // 2) 仓库列表
  await page.goto('/#/github/repos');
  const reposPage = page.getByTestId('remote-repos-page');
  await expect(reposPage).toBeVisible();
  await expect(page.getByTestId('repos-items')).toContainText('demo');

  // 3) PR 列表：先给一个 owner/repo，再断言请求参数（页面自己拼 host=github.com）
  await page.goto('/#/github/pull-requests');
  await expect(page.getByTestId('pull-requests-page')).toBeVisible();
  await page.getByTestId('prs-repo-input').fill('octo/demo');
  await page.getByTestId('prs-repo-go').click();

  const item = page.getByTestId('prs-item-7');
  await expect(item).toContainText('Add feature flag');
  const listCall = await page.evaluate(() =>
    (window.__githubCalls ?? []).filter((call) => call.command === 'repo_pull_list').at(-1),
  );
  expect(listCall).toMatchObject({
    args: { host: 'github.com', owner: 'octo', repo: 'demo', stateFilter: 'open', page: 1 },
  });

  // 4) 打开详情：合并条件、评审结论、时间线评论都要能读到
  await item.click();
  const detail = page.getByTestId('pull-detail');
  await expect(detail).toBeVisible();
  await expect(page.getByTestId('pull-stats')).toContainText('1');
  await expect(page.getByTestId('pull-mergeable')).toBeVisible();
  await expect(page.getByTestId('pull-reviews')).toContainText('hubot：已批准');
  await expect(page.getByTestId('pull-comment-list')).toContainText('记得补测试');
  // 描述是后端消毒过的 HTML，界面按富文本渲染（不是把标签当文字显示出来）
  await expect(page.getByTestId('pull-body')).toContainText('启用');

  // 5) 查看 diff：文件默认折叠，展开后逐行可见；点"新增行"开行内评论
  const toggle = page.getByTestId('pull-file-toggle').first();
  await expect(page.getByTestId('pull-file-diff')).toHaveCount(0);
  await toggle.click();
  await expect(page.getByTestId('pull-file-diff')).toBeVisible();
  const lines = page.getByTestId('pull-line');
  await expect(lines).toHaveCount(2);
  await lines.nth(1).click();

  const composer = page.getByTestId('pull-inline-composer');
  await expect(composer).toBeVisible();
  // 锚点必须说清"评论挂在哪一行、哪一侧"（用户点错行的代价是评论跑到别处）
  await expect(composer).toContainText('src/app.ts');
  await expect(composer).toContainText('第 11 行');
  await page.getByTestId('pull-inline-input').fill('这行建议加注释');
  await page.getByTestId('pull-inline-post').click();

  await expect(page.getByText('行内评论已发表').first()).toBeVisible();
  const inlineCall = await page.evaluate(() =>
    (window.__githubCalls ?? [])
      .filter((call) => call.command === 'repo_pull_review_comment_create')
      .at(-1),
  );
  expect(inlineCall).toMatchObject({
    args: {
      host: 'github.com',
      owner: 'octo',
      repo: 'demo',
      number: 7,
      path: 'src/app.ts',
      side: 'RIGHT',
      line: 11,
      body: '这行建议加注释',
    },
  });

  // 6) 时间线评论
  await page.getByTestId('pull-comment-input').fill('整体没问题');
  await page.getByTestId('pull-comment-post').click();
  await expect(page.getByText('评论已发表').first()).toBeVisible();
  const timelineCall = await page.evaluate(() =>
    (window.__githubCalls ?? [])
      .filter((call) => call.command === 'repo_pull_comment_create')
      .at(-1),
  );
  expect(timelineCall).toMatchObject({ args: { number: 7, body: '整体没问题' } });

  // 7) 提交 review：先选结论，再提交（结论与正文一起送到后端）
  await page.getByTestId('pull-review-APPROVE').click();
  await page.getByTestId('pull-review-body').fill('LGTM');
  await page.getByTestId('pull-review-submit').click();
  await expect(page.getByText(/Review 已提交/).first()).toBeVisible();
  const reviewCall = await page.evaluate(() =>
    (window.__githubCalls ?? [])
      .filter((call) => call.command === 'repo_pull_review_submit')
      .at(-1),
  );
  expect(reviewCall).toMatchObject({ args: { number: 7, event: 'APPROVE', body: 'LGTM' } });

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});

test('未登录时给出登录引导，而不是把内部错误摊给用户（M4 验收第 9 条）', async ({ page }) => {
  await page.addInitScript(() => {
    window.__githubAuthRequired = true;
  });

  await page.goto('/#/github/repos');
  await expect(page.getByTestId('remote-repos-page')).toBeVisible();

  // 引导而非报错：页面说清"需要登录"并给出入口，且不渲染任何后端原始错误字段
  await expect(page.getByTestId('repos-sign-in-go')).toBeVisible();
  await expect(page.getByText('还没有登录账号')).toBeVisible();

  // 同一个错误码在 PR 页也要走同一条引导（不能有的页面报错、有的页面静默）
  await page.goto('/#/github/pull-requests');
  await expect(page.getByTestId('pull-requests-page')).toBeVisible();
  await page.getByTestId('prs-repo-input').fill('octo/demo');
  await page.getByTestId('prs-repo-go').click();
  await expect(page.getByText('还没有登录账号')).toBeVisible();

  const errs = await page.evaluate(() => window.__errs ?? []);
  expect(errs, JSON.stringify(errs)).toEqual([]);
});
