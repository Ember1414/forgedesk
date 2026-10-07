import { defineConfig } from '@playwright/test';

// E2E configuration (M0 / OPS-7). CI wiring follows the ADR-002 quota strategy.
// - Chromium only: M0 interactions are DOM-level, independent of the WebView brand.
//   Cross-engine checks (WebView2/WKWebView/WebKitGTK) need the desktop runtime;
//   see docs/acceptance/M0-interaction.md for what could not be verified.
// - workers: 1: specs share one dev server and localStorage (theme/language
//   persistence), so running them in parallel would pollute each other.
// - reuseExistingServer: the dev server usually runs during local development.
export default defineConfig({
  testDir: './e2e',
  timeout: 30000,
  // 元素可见性的等待窗口从默认 5s 提到 10s。
  //
  // 为什么：dev server 是按需编译的，第一次访问最重的路由（提交图 + 虚拟列表）
  // 要现场转换整张模块图，冷启动时偶尔超过 5s，表现为
  // "element(s) not found / waiting for navigation to finish" —— 页面最终渲染正常，
  // 断言却已经超时（T2.6 与 T2.7 各复现过一次，且失败的用例每次不同）。
  // 10s 仍然是个会失败的窗口（元素真出不来就红），但不再把"编译慢"当成产品缺陷。
  expect: { timeout: 10000 },
  retries: 0,
  workers: 1,
  use: {
    // 本地默认用系统自带的 Edge（Chromium 内核）：免去 ~200MB 的浏览器下载；
    // M0 的断言都是 DOM 级交互，与"哪个 Chromium 发行版"无关。
    //
    // CI（Linux）没有 Edge，用 `PW_CHANNEL=chromium` 切到 Playwright 自带的 Chromium
    // （工作流里会先 `playwright install --with-deps chromium`）。
    channel: process.env['PW_CHANNEL'] ?? 'msedge',
    baseURL: 'http://localhost:1420',
    viewport: { width: 1280, height: 800 },
  },
  webServer: {
    command: 'pnpm dev',
    url: 'http://localhost:1420',
    reuseExistingServer: true,
    timeout: 60000,
  },
});
