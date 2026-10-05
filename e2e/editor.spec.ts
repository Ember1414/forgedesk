import { expect, test, type Page } from '@playwright/test';

/**
 * 编辑器 E2E（T5.7）：mock `fs_*` 命令，跑"打开 → 编辑 → 保存 →
 * 外部修改三选一"的产品链路（Monaco 真组件 + mock IPC）。
 *
 * 后端的路径安全/EOL 语义由 `crates/services/src/workspace_fs.rs` 的
 * 单测覆盖；这里验证的是界面半条链路与三选一绝不静默覆盖。
 */
const EDITOR_MOCK = `
  window.__errs = [];
  window.__fsWrites = [];
  window.__emitRepoChanged = function (paths, kind) {
    for (const key of Object.keys(eventOf)) {
      if (eventOf[key] === 'repo:changed') {
        callbacks[key]({ event: 'repo:changed', id: Number(key), payload: { repoId: 1, kind: kind || 'workspace', paths: paths } });
      }
    }
  };
  const files = {
    "src/main.rs": { disk: "fn main() {}\\n", eol: "lf", hasBom: false },
  };
  const callbacks = {};
  const eventOf = {};
  let nextCallbackId = 1;
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
      if (command === "workspace_status") return Promise.resolve({
        branch: { oid: "a1b2c3", head: "main", detached: false, upstream: null, ahead: 0, behind: 0 },
        operation: "none", staged: [], unstaged: [], untracked: [], conflicted: [], ignored: [], ignoredCount: null
      });
      if (command === "git_branch_list") return Promise.resolve([{ name: "main", isRemote: false, isHead: true, target: "a1b2c3", upstream: null, ahead: 0, behind: 0, upstreamGone: false }]);
      if (command === "fs_tree") {
        if ((args.path || "") === "") {
          return Promise.resolve([
            { name: "src", relPath: "src", kind: "dir", size: 0 },
          ]);
        }
        return Promise.resolve([
          { name: "main.rs", relPath: "src/main.rs", kind: "file", size: 13 },
        ]);
      }
      if (command === "fs_read") {
        const file = files[args.path];
        if (!file) return Promise.reject({ code: "NOT_FOUND", message: "the file does not exist" });
        return Promise.resolve({ content: file.disk, eol: file.eol, hasBom: file.hasBom, size: file.disk.length, isBinary: false, truncated: false });
      }
      if (command === "fs_write") {
        window.__fsWrites.push({ path: args.path, content: args.content, eol: args.eol, hasBom: args.hasBom });
        files[args.path].disk = args.content;
        setTimeout(function () {
          window.__emitRepoChanged([args.path], "workspace");
        }, 20);
        return Promise.resolve({ writtenBytes: args.content.length });
      }
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

async function openEditor(page: Page): Promise<void> {
  await page.goto('/#/repo/1/editor');
  await expect(page.getByRole('heading', { level: 1, name: '编辑器' })).toBeAttached();
  // 文件树根
  await expect(page.getByRole('button', { name: 'src' })).toBeVisible();
}

async function expandSrcAndOpenMain(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'src' }).click();
  await page.getByRole('button', { name: 'main.rs' }).click();
  // Monaco 装载（lazy chunk）：等待标签与编辑区出现
  await expect(page.getByText('src/main.rs').first()).toBeVisible();
}

test('打开文件 → 编辑 → 保存：fs_write 收到内容，保存后无三选一', async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
  await page.addInitScript(EDITOR_MOCK);
  await openEditor(page);
  await expandSrcAndOpenMain(page);

  // Monaco 编辑：真实键入依赖隐藏 textarea 的 IME 链路（Playwright 下不稳定），
  // 改用 Monaco 公共 API 编辑 model——与"手敲"等价（都走 onChange → setState）。
  await expect.poll(() => page.evaluate(() => window.__forgedeskEditor !== undefined)).toBe(true);
  await page.evaluate(() => {
    const model = window.__forgedeskEditor?.getModel?.();
    if (!model) {
      throw new Error('monaco model not ready');
    }
    model.applyEdits([{ range: model.getFullModelRange(), text: 'fn main() {} // changed\n' }]);
  });

  await page.getByRole('button', { name: '保存', exact: true }).click();

  await expect
    .poll(async () =>
      ((await page.evaluate(() => window.__fsWrites)) ?? []).map((w) => w.content).join(''),
    )
    .toContain('// changed');

  // 保存触发的 repo:changed 回声：磁盘与编辑器一致 → 不弹三选一
  await expect(page.getByText('文件在外部被修改：')).toHaveCount(0);
  await expectNoPageErrors(page);
});

test('外部修改 → 三选一出现（绝不静默覆盖）', async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
  await page.addInitScript(EDITOR_MOCK);
  await openEditor(page);
  await expandSrcAndOpenMain(page);

  // 直接改 mock 里的磁盘内容并广播变化
  await page.evaluate(() => {
    const internals = window.__TAURI_INTERNALS__;
    void internals?.invoke('fs_write', {
      path: 'src/main.rs',
      content: 'fn external() {}\\n',
      eol: 'lf',
      hasBom: false,
    });
  });

  // 三选一提示出现
  await expect(page.getByText('文件在外部被修改：')).toBeVisible({ timeout: 10000 });
  await expect(page.getByRole('button', { name: '重新加载（丢弃我的编辑）' })).toBeVisible();
  await expect(page.getByRole('button', { name: '保留我的编辑' })).toBeVisible();
  await expect(page.getByRole('button', { name: '并排对比' })).toBeVisible();

  // 保留我的编辑：提示消失，磁盘基线被替换
  await page.getByRole('button', { name: '保留我的编辑' }).click();
  await expect(page.getByText('文件在外部被修改：')).toHaveCount(0);

  await expectNoPageErrors(page);
});

async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}
