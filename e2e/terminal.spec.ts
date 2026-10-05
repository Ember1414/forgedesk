/**
 * 内嵌终端的 E2E（T5.2）。
 *
 * mock 方式与 dashboard-open.spec.ts 一致（`window.__TAURI_INTERNALS__` 注入）。
 * mock 的后端会"回显"用户输入（`mock: <输入>`）：这验证的是
 * 键入 → term_write → 事件路由 → xterm 渲染 的完整链路，xterm 与后端
 * PTY 的真实交互由 Windows 集成测试（crates/services/tests/terminal.rs）
 * 与三平台手工验收覆盖——浏览器 E2E 没有真 PTY，那是它的边界而不是缺陷。
 *
 * 覆盖：创建会话 / echo 回显 / resize / 搜索栏 / 退出横幅 / window.__errs 为空。
 */
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

async function pinLanguage(page: Page): Promise<void> {
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.language', 'zh-CN');
  });
}

async function expectNoPageErrors(page: Page): Promise<void> {
  const errors = await page.evaluate(() => window.__errs ?? []);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

const TERMINAL_MOCK = `
  var listeners = [];
  var eventsByName = {};
  var nextTermId = 0;
  var lineBuf = {};
  window.__termWrites = [];
  window.__echoLog = [];
  window.__termResize = [];
  window.__termClosed = [];
  window.__createdSessions = [];
  window.__termReports = [];
  window.__scanLevel = 'dangerous';  // 可被用例切换：null = 不识别
  // 把 UTF-8 文本编成 base64（与后端输出的 base64 载荷对偶）
  function b64(text) {
    var bytes = new TextEncoder().encode(text);
    var binary = '';
    for (var i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
    return btoa(binary);
  }
  function emit(event, payload) {
    var targets = eventsByName[event] || [];
    for (var i = 0; i < targets.length; i++) targets[i]({ event: event, id: 1, payload: payload });
  }
  window.__emitTermOutput = function (termId, text) {
    emit('term:output', { termId: termId, data: b64(text) });
  };
  window.__emitTermExit = function (termId, code) {
    emit('term:exit', { termId: termId, code: code });
  };
  var settingsStore = {};
  // reload 会重跑整段 init 脚本（内存表清零），用 localStorage 做跨 reload 的预置
  try {
    var presetLevel = window.localStorage.getItem('forgedesk.e2e.terminal.safety.level');
    if (presetLevel) settingsStore['terminal.safety.level'] = presetLevel;
  } catch (e) { /* ignore */ }
  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) { listeners.push(callback); return listeners.length; },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === 'settings_set') {
        settingsStore[args.key] = args.value;
        return Promise.resolve(null);
      }
      if (command === 'settings_get') return Promise.resolve(settingsStore[args.key] || null);
      if (command === 'settings_all') return Promise.resolve(settingsStore);
      if (command === 'plugin:event|listen') {
        var callback = listeners[args.handler - 1];
        (eventsByName[args.event] = eventsByName[args.event] || []).push(callback);
        return Promise.resolve(1);
      }
      if (command === 'plugin:event|unlisten') return Promise.resolve(null);
      if (command === 'term_shell_list') {
        return Promise.resolve([
          { id: 'default', program: 'mock-shell' },
          { id: 'powershell', program: 'powershell' }
        ]);
      }
      if (command === 'term_create') {
        var termId = 'term-' + (++nextTermId);
        window.__createdSessions.push({ termId: termId, request: args.request });
        // 模拟 shell 启动横幅（含中文，验证 UTF-8 往返）
        setTimeout(function () {
          emit('term:output', { termId: termId, data: b64('mock-shell 已就绪\\r\\n') });
        }, 30);
        return Promise.resolve({ termId: termId, program: 'mock-shell' });
      }
      if (command === 'term_write') {
        var text = new TextDecoder().decode(new Uint8Array(args.data));
        window.__termWrites.push({ termId: args.termId, text: text });
        // 回显用户输入：键入是逐字符到达的，行缓冲跨 write 累积，Enter 提交
        var buf = lineBuf[args.termId] || '';
        for (var ci = 0; ci < text.length; ci++) {
          var ch = text[ci];
          if (ch === '\\r' || ch === '\\n') {
            if (buf.length > 0) {
              (function (termId, line) {
                setTimeout(function () {
                  emit('term:output', { termId: termId, data: b64('mock: ' + line + '\\r\\n') });
                }, 20);
              })(args.termId, buf);
            }
            buf = '';
          } else if (ch >= ' ') {
            buf += ch;
          }
        }
        lineBuf[args.termId] = buf;
        return Promise.resolve(null);
      }
      if (command === 'term_scan_command') {
        var lineArg = args.line || '';
        if (lineArg.indexOf('git reset --hard') >= 0) {
          return Promise.resolve({ kind: 'reset_hard', level: window.__scanLevel, canonical: 'git reset --hard' });
        }
        return Promise.resolve(null);
      }
      if (command === 'term_report_command') {
        window.__termReports.push(args.request);
        return Promise.resolve(null);
      }
      if (command === 'term_resize') {
        window.__termResize.push({ termId: args.termId, cols: args.cols, rows: args.rows });
        return Promise.resolve(null);
      }
      if (command === 'term_close') {
        window.__termClosed.push(args.termId);
        return Promise.resolve(null);
      }
      if (command === 'term_list') return Promise.resolve([]);
      if (command === 'settings_get') return Promise.resolve(null);
      if (command === 'settings_all') return Promise.resolve({});
      if (command === 'app_version') return Promise.resolve({ version: '0.0.1', gitDescribe: null });
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
`;

test('创建会话：标签出现、启动横幅渲染（含中文）', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(TERMINAL_MOCK);
  await page.goto('/#/repo/1/terminal');
  await expect(page.getByRole('heading', { level: 1, name: '终端' })).toBeAttached();

  await expect(page.getByText('还没有终端。点上面的 + 新建一个。')).toBeVisible();
  await page.getByRole('button', { name: '新建终端' }).click();
  await page.getByRole('menuitem', { name: '默认 Shell' }).click();

  await expect(page.getByRole('tab', { name: '默认 Shell' })).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => window.__forgedeskTermText?.('term-1') ?? ''))
    .toContain('mock-shell 已就绪');
  await expectNoPageErrors(page);
});

test('键入命令：term_write 收到字节、回显经事件渲染到终端', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(TERMINAL_MOCK);
  await page.goto('/#/repo/1/terminal');
  await page.getByRole('button', { name: '新建终端' }).click();
  await page.getByRole('menuitem', { name: '默认 Shell' }).click();
  await expect
    .poll(() => page.evaluate(() => window.__forgedeskTermText?.('term-1') ?? ''))
    .toContain('mock-shell 已就绪');

  // xterm 的输入面是它自己的 DOM：聚焦后模拟真实键入
  await page.locator('[data-term-view] textarea.xterm-helper-textarea').focus();
  await page.keyboard.type('echo 中文 🎉');
  await page.keyboard.press('Enter');

  await expect
    .poll(async () =>
      ((await page.evaluate(() => window.__termWrites)) ?? []).map((w) => w.text).join(''),
    )
    .toContain('echo 中文 🎉');
  // 回显经事件 → xterm 缓冲渲染（canvas 渲染器不产 DOM 文本，走缓冲助手断言）
  await expect
    .poll(() => page.evaluate(() => window.__forgedeskTermText?.('term-1') ?? ''))
    .toContain('mock: echo 中文 🎉');

  await expectNoPageErrors(page);
});

test('窗口尺寸变化触发 term_resize', async ({ page }) => {
  await pinLanguage(page);
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.addInitScript(TERMINAL_MOCK);
  await page.goto('/#/repo/1/terminal');
  await page.getByRole('button', { name: '新建终端' }).click();
  await page.getByRole('menuitem', { name: '默认 Shell' }).click();
  await expect
    .poll(() => page.evaluate(() => window.__forgedeskTermText?.('term-1') ?? ''))
    .toContain('mock-shell 已就绪');

  await page.setViewportSize({ width: 1000, height: 700 });
  await expect
    .poll(async () => (await page.evaluate(() => window.__termResize?.length)) ?? 0, {
      timeout: 5000,
    })
    .toBeGreaterThan(0);

  await expectNoPageErrors(page);
});

test('Ctrl+F 打开搜索栏，可关闭', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(TERMINAL_MOCK);
  await page.goto('/#/repo/1/terminal');
  await page.getByRole('button', { name: '新建终端' }).click();
  await page.getByRole('menuitem', { name: '默认 Shell' }).click();
  await expect
    .poll(() => page.evaluate(() => window.__forgedeskTermText?.('term-1') ?? ''))
    .toContain('mock-shell 已就绪');

  // 点击菜单后焦点可能回到 body：先聚焦终端再按 Ctrl+F
  await page.locator('[data-term-view] textarea.xterm-helper-textarea').focus();
  await page.keyboard.press('Control+f');
  await expect(page.getByRole('textbox', { name: '搜索终端内容' })).toBeVisible();

  await page.getByRole('button', { name: '关闭搜索' }).click();
  await expect(page.getByRole('textbox', { name: '搜索终端内容' })).toHaveCount(0);

  await expectNoPageErrors(page);
});

test('会话退出：横幅出现，重新开始创建新会话，关闭移除标签', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(TERMINAL_MOCK);
  await page.goto('/#/repo/1/terminal');
  await page.getByRole('button', { name: '新建终端' }).click();
  await page.getByRole('menuitem', { name: '默认 Shell' }).click();
  await expect(page.getByRole('tab', { name: '默认 Shell' })).toBeVisible();

  await page.evaluate(() => {
    window.__emitTermExit?.('term-1', 0);
  });

  await expect(page.getByText('会话已结束（退出码 0）')).toBeVisible();

  // 重新开始：创建第二个会话
  await page.getByRole('button', { name: '重新开始' }).click();
  await expect(page.getByRole('tab', { name: '默认 Shell' }).nth(1)).toBeVisible();
  await expect
    .poll(async () => (await page.evaluate(() => window.__createdSessions?.length)) ?? 0)
    .toBe(2);

  await expectNoPageErrors(page);
});

test('T5.3 提示级：危险命令显示非阻塞提示条，输入不被阻断', async ({ page }) => {
  await pinLanguage(page);
  await page.addInitScript(TERMINAL_MOCK);
  await page.goto('/#/repo/1/terminal');
  await page.getByRole('button', { name: '新建终端' }).click();
  await page.getByRole('menuitem', { name: '默认 Shell' }).click();
  await expect
    .poll(() => page.evaluate(() => window.__forgedeskTermText?.('term-1') ?? ''))
    .toContain('mock-shell 已就绪');

  await page.locator('[data-term-view] textarea.xterm-helper-textarea').focus();
  await page.keyboard.type('git reset --hard');
  await page.keyboard.press('Enter');

  // 提示条出现（含规范化命令），并带图形入口链接
  await expect(page.getByRole('status')).toContainText('git reset --hard');
  await expect(page.getByRole('status').getByText('查看快照与回滚')).toBeVisible();

  // 不阻断：命令照常发给后端（含回车），提示条自动消失
  await expect
    .poll(async () =>
      ((await page.evaluate(() => window.__termWrites)) ?? []).map((w) => w.text).join(''),
    )
    .toContain('git reset --hard\r');
  await expect
    .poll(async () => (await page.evaluate(() => window.__termReports)) ?? [])
    .toHaveLength(1);
  await page.keyboard.type('echo more');
  await expect
    .poll(async () =>
      ((await page.evaluate(() => window.__termWrites)) ?? []).map((w) => w.text).join(''),
    )
    .toContain('echo more');
  // 提示条 2.5s 自动消失（发 Enter 前设置时钟不现实，直接等它离开 DOM）
  await expect(page.getByRole('status')).toHaveCount(0, { timeout: 5000 });

  await expectNoPageErrors(page);
});

test('T5.3 确认级：高危命令挂起，确认后写入，取消发 Ctrl+C', async ({ page }) => {
  await pinLanguage(page);
  // 确认级通过 localStorage 预置：mock 初始化时读它进设置表，
  // settingsStore 启动拉取（settings_all）即读到 confirm
  await page.addInitScript(() => {
    window.localStorage.setItem('forgedesk.e2e.terminal.safety.level', '"confirm"');
  });
  await page.addInitScript(TERMINAL_MOCK);
  await page.goto('/#/repo/1/terminal');
  await page.getByRole('button', { name: '新建终端' }).click();
  await page.getByRole('menuitem', { name: '默认 Shell' }).click();
  await expect
    .poll(() => page.evaluate(() => window.__forgedeskTermText?.('term-1') ?? ''))
    .toContain('mock-shell 已就绪');

  await page.locator('[data-term-view] textarea.xterm-helper-textarea').first().focus();
  await page.keyboard.type('git reset --hard');
  await page.keyboard.press('Enter');

  // 确认对话框出现（若识别器/设置链路异常，这里给出可读现场而不是裸超时）
  try {
    await expect
      .poll(async () => ({
        dialog: (await page.locator('[role=alertdialog]').count()) > 0,
      }))
      .toMatchObject({ dialog: true });
  } catch (error) {
    const diag = await page.evaluate(() => ({
      reports: (window.__termReports ?? []).length,
      writes: (window.__termWrites ?? []).map((w) => w.text.charCodeAt(w.text.length - 1)),
      settings: window.__TAURI_INTERNALS__ ? 'mock-live' : 'no-mock',
    }));
    throw new Error(`CONFIRM-FLOW ${JSON.stringify(diag)}`, { cause: error });
  }
  await expect(page.getByRole('alertdialog')).toBeVisible();
  const writesBefore = (await page.evaluate(() => (window.__termWrites ?? []).length)) ?? 0;

  // 确认：Enter 写入，行落地
  await page.getByRole('button', { name: '确认执行' }).click();
  await expect(page.getByRole('alertdialog')).toHaveCount(0);
  await expect
    .poll(async () =>
      ((await page.evaluate(() => window.__termWrites)) ?? [])
        .slice(writesBefore)
        .map((w) => w.text)
        .join(''),
    )
    .toContain('\r');

  // 取消路径：再试一次，这次取消 → 发送 Ctrl+C（0x03），没有第二个回车
  await page.keyboard.type('git reset --hard');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('alertdialog')).toBeVisible();
  const writesMid = (await page.evaluate(() => (window.__termWrites ?? []).length)) ?? 0;
  await page.getByRole('button', { name: '取消' }).click();
  await expect
    .poll(async () =>
      ((await page.evaluate(() => window.__termWrites)) ?? [])
        .slice(writesMid)
        .map((w) => w.text)
        .join(''),
    )
    .toContain(String.fromCharCode(3));

  await expectNoPageErrors(page);
});
