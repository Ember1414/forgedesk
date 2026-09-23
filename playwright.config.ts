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
  retries: 0,
  workers: 1,
  use: {
    // 用系统自带的 Edge（Chromium 内核）：免去 ~200MB 的浏览器下载；
    // M0 的断言都是 DOM 级交互，与"哪个 Chromium 发行版"无关。
    channel: 'msedge',
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
