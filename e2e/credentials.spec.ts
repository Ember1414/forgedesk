import { expect, test, type Page } from '@playwright/test';

import { pinChineseLanguage } from './helpers';

/**
 * 凭据与 SSH 面板的端到端验收（T2.7）。
 *
 * # 这一层要证明什么（单测证明不了的部分）
 *
 * 1. **明文只经一次 IPC**：真实运行里 token 的唯一去处是系统凭据库。这里记录 IPC 载荷，
 *    断言"发过一次、之后输入框清空、DOM 里再也搜不到它"——单测能测组件，测不了
 *    "整页渲染完之后它还在不在"。
 * 2. **面板在真实的 IPC 事件循环里能用**：状态、列表、SSH 盘点三路查询并发到达时
 *    界面不崩（`window.__errs` 为空），且各自渲染到正确的区域。
 * 3. **删除是一次确认过的动作**：点删除先出对话框，确认后才发命令。
 */
const MOCK_SCRIPT = `
  window.__errs = [];
  window.__credCalls = [];
  var credentials = [
    { key: { provider: "github", host: "github.com", login: "octocat" }, kind: "pat", createdAtMs: 1700000000000 }
  ];
  var inventory = {
    directory: "C:\\\\Users\\\\octocat\\\\.ssh",
    keys: [
      {
        publicPath: "C:\\\\Users\\\\octocat\\\\.ssh\\\\id_ed25519.pub",
        privatePath: "C:\\\\Users\\\\octocat\\\\.ssh\\\\id_ed25519",
        keyType: "ssh-ed25519",
        comment: "octocat@example.com"
      }
    ],
    agent: { kind: "ready", keys: [{ bits: 256, fingerprint: "SHA256:abc", comment: "id_ed25519" }] }
  };
  window.__TAURI_INTERNALS__ = {
    transformCallback: function () { return 1; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      window.__credCalls.push({ command: command, args: args });
      if (command === "settings_all" || command === "settings_get") return Promise.resolve(null);
      if (command === "app_version") return Promise.resolve({ version: "0.0.1", gitDescribe: null });
      if (command === "credentials_status") return Promise.resolve({
        backend: "systemKeyring", mode: "systemKeyring", count: credentials.length, vaultExists: false
      });
      if (command === "credentials_list") return Promise.resolve(credentials);
      if (command === "credentials_ssh_inventory") return Promise.resolve(inventory);
      if (command === "credentials_save") return Promise.resolve({
        key: { provider: args.provider, host: args.host, login: args.login }, kind: args.kind, createdAtMs: 1700000001000
      });
      if (command === "credentials_delete") return Promise.resolve(null);
      if (command === "credential_test_remote") return Promise.resolve({ refs: 7 });
      if (command === "plugin:event|unlisten") return Promise.resolve(null);
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

/** 打开"代码托管账号"设置页（面板挂在这里）。 */
async function openAccounts(page: Page): Promise<void> {
  await page.addInitScript(MOCK_SCRIPT);
  await pinChineseLanguage(page);
  await page.goto('/#/settings/github');
  await expect(page.getByRole('heading', { level: 1, name: '代码托管账号' })).toBeVisible();
}

async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, `未捕获错误：${JSON.stringify(errors)}`).toEqual([]);
}

/** mock 记录里某个命令的调用（没有则返回空数组）。 */
async function callsOf(page: Page, command: string) {
  const calls = await page.evaluate(() => window.__credCalls ?? []);
  return calls.filter((call) => call.command === command);
}

test('账号页同时展示已保存凭据与本地 SSH 盘点（__errs 为空）', async ({ page }) => {
  await openAccounts(page);

  // 凭据：列表（不含密文）与状态徽标
  const list = page.getByTestId('credentials-list');
  await expect(list).toBeVisible();
  await expect(list).toContainText('github:github.com');
  await expect(list).toContainText('octocat');
  await expect(list).toContainText('访问令牌');
  await expect(page.getByTestId('credentials-status')).toContainText('系统凭据库');

  // SSH：密钥清单 + agent 状态 + 指纹（三路查询并发到达时各自渲染到正确区域）
  await expect(page.getByTestId('ssh-keys')).toContainText('id_ed25519.pub');
  await expect(page.getByTestId('ssh-key-state-id_ed25519.pub')).toContainText('公私钥齐全');
  await expect(page.getByTestId('ssh-agent-status')).toContainText('已加载 1 把密钥');
  await expect(page.getByTestId('ssh-agent-keys')).toContainText('SHA256:abc');

  await expectNoPageErrors(page);
});

test('保存凭据：明文只发一次，保存后从界面上消失', async ({ page }) => {
  const token = 'ghp_e2e_must_not_survive';
  await openAccounts(page);
  await expect(page.getByTestId('credentials-list')).toBeVisible();

  await page.getByTestId('credentials-login').fill('octocat');
  await page.getByTestId('credentials-secret').fill(token);
  await page.getByTestId('credentials-save').click();

  // 1) 载荷正确，且**只有一次**（界面不能重发）
  await expect
    .poll(async () => (await callsOf(page, 'credentials_save')).length)
    .toBeGreaterThan(0);
  const saves = await callsOf(page, 'credentials_save');
  expect(saves).toHaveLength(1);
  expect(saves[0]?.args?.secret).toBe(token);
  expect(saves[0]?.args?.provider).toBe('github');
  expect(saves[0]?.args?.login).toBe('octocat');

  // 2) 令牌不再留在界面上（红线 R8 的界面侧）
  await expect(page.getByTestId('credentials-secret')).toHaveValue('');
  await expect(page.getByTestId('credentials-login')).toHaveValue('');
  expect(await page.content()).not.toContain(token);

  await expectNoPageErrors(page);
});

test('删除凭据需要先确认，确认后才发命令', async ({ page }) => {
  await openAccounts(page);
  await expect(page.getByTestId('credentials-list')).toBeVisible();

  await page.getByTestId('credentials-delete-octocat').click();

  const dialog = page.getByTestId('credentials-delete-dialog');
  await expect(dialog).toBeVisible();
  // 对话框要写清"删的是哪一条"，否则用户只能靠记忆
  await expect(dialog).toContainText('github:github.com:octocat');
  expect(await callsOf(page, 'credentials_delete')).toHaveLength(0);

  await page.getByTestId('credentials-delete-confirm').click();

  await expect.poll(async () => (await callsOf(page, 'credentials_delete')).length).toBe(1);
  const deletes = await callsOf(page, 'credentials_delete');
  expect(deletes[0]?.args?.host).toBe('github.com');
  expect(deletes[0]?.args?.login).toBe('octocat');

  await expectNoPageErrors(page);
});

test('测试连接：把主机拼成地址并展示远端引用条数', async ({ page }) => {
  await openAccounts(page);
  await expect(page.getByTestId('credentials-list')).toBeVisible();

  await page.getByTestId('credentials-probe-octocat').click();

  await expect(page.getByText('连接成功：远端有 7 条引用').first()).toBeVisible();
  const probes = await callsOf(page, 'credential_test_remote');
  expect(probes).toHaveLength(1);
  expect(probes[0]?.args?.url).toBe('https://github.com');

  await expectNoPageErrors(page);
});
