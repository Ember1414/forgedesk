#!/usr/bin/env node
/**
 * 官网截图采集（2026-10-08）。
 *
 * # 这是什么
 *
 * 官网首页需要**界面预览图**。这里用与 e2e 完全相同的做法把界面真正跑起来：
 * 注入 `window.__TAURI_INTERNALS__`（演示宿主，见 `scripts/site/demo-mock.js`）
 * → 走真实入口打开仓库 → 访问真实路由 → 截整屏。因此图里是**真实的界面**
 * （真实组件、真实样式、真实提交图渲染），只有数据是那份虚构的演示夹具。
 *
 * # 用法
 *
 *   pnpm site:screenshots          # 自动复用或启动 dev server，产出 site/images/*.webp
 *
 * 目标文件（会覆盖）：
 *   site/images/dashboard.webp      仪表盘与最近仓库（亮色）
 *   site/images/history.webp        提交历史 + 提交详情（亮色）
 *   site/images/history-dark.webp   提交历史（暗色）
 *   site/images/status.webp         工作区与提交面板（亮色）
 *
 * # 为什么输出 webp
 *
 * 界面截图是"大色块 + 细文字"，PNG 在 1600px 宽下通常 400–700 KB，webp 约 1/4。
 * 官网是零依赖静态站，图片是唯一的"重"资源，值得在这里换格式。
 */
import { mkdirSync, readFileSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';

import { chromium } from '@playwright/test';
import sharp from 'sharp';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const outputDir = join(repoRoot, 'site', 'images');
const baseURL = process.env.SITE_SCREENSHOT_BASE ?? 'http://localhost:1420';
const SHOT_WIDTH = 1600; // 输出宽度（视口 1440 × DPR 2 = 2880，再缩到 1600 保持清晰）

/** 演示宿主脚本（普通脚本，原样注入页面）。 */
const mockScript = readFileSync(join(repoRoot, 'scripts', 'site', 'demo-mock.js'), 'utf8');

/** 等 dev server 就绪；没在跑就自己拉起来（退出时收掉）。 */
async function ensureDevServer() {
  const reachable = async () => {
    try {
      const response = await fetch(baseURL, { signal: AbortSignal.timeout(1500) });
      return response.ok || response.status < 500;
    } catch {
      return false;
    }
  };
  if (await reachable()) {
    console.log(`复用已在运行的 dev server：${baseURL}`);
    return null;
  }
  console.log('启动 dev server（pnpm dev）…');
  const child = spawn('pnpm', ['dev'], { cwd: repoRoot, stdio: 'ignore', shell: true });
  for (let attempt = 0; attempt < 60; attempt += 1) {
    await new Promise((tick) => setTimeout(tick, 1000));
    if (await reachable()) {
      console.log(`dev server 就绪：${baseURL}`);
      return child;
    }
  }
  child.kill();
  throw new Error('dev server 在 60 秒内没有就绪');
}

/**
 * 走界面上的真实入口打开演示仓库。
 *
 * 为什么要多这一步：直接访问 `#/repo/1/history` 时应用的"当前仓库"是空的，
 * 顶栏会显示"未打开仓库"——一屏之内自相矛盾（右边是仓库内容，上面说没打开）。
 * 真实入口（顶栏仓库切换器）会走 `repo_open`，状态因此是一致的。
 */
async function openDemoRepo(page) {
  await page.goto(`${baseURL}/#/`, { waitUntil: 'domcontentloaded' });
  await page
    .getByRole('heading', { name: '仪表盘' })
    .waitFor({ timeout: 20000 })
    .catch(() => {});
  await page
    .getByRole('button', { name: /当前仓库|未打开仓库/ })
    .first()
    .click({ timeout: 5000 })
    .catch(() => {});
  await page
    .getByRole('menuitem')
    .first()
    .click({ timeout: 5000 })
    .catch(() => {});
  await page.waitForURL(/#\/repo\/\d+\//, { timeout: 10000 }).catch(() => {});
}

/** 场景定义：怎么准备画面、用什么主题、存成哪个文件。 */
const SCENES = [
  {
    name: 'dashboard',
    theme: 'light',
    description: '仪表盘与最近仓库',
    async prepare(page) {
      await page.goto(`${baseURL}/#/`, { waitUntil: 'domcontentloaded' });
    },
    async ready(page) {
      await page
        .getByRole('heading', { name: '仪表盘' })
        .waitFor({ timeout: 20000 })
        .catch(() => {});
    },
  },
  {
    name: 'history',
    theme: 'light',
    description: '提交历史 + 提交详情（亮色）',
    async prepare(page) {
      await openDemoRepo(page);
      await page.goto(`${baseURL}/#/repo/1/history`, { waitUntil: 'domcontentloaded' });
      // 选中第一条提交：右侧详情面板因此有内容，而不是一句"选中后显示在这里"
      await page
        .locator('[role="option"]')
        .first()
        .click({ timeout: 8000 })
        .catch(() => {});
    },
    async ready(page) {
      await page
        .locator('[role="option"]')
        .first()
        .waitFor({ timeout: 20000 })
        .catch(() => {});
    },
  },
  {
    name: 'history-dark',
    theme: 'dark',
    description: '提交历史 + 提交详情（暗色）',
    async prepare(page) {
      await openDemoRepo(page);
      await page.goto(`${baseURL}/#/repo/1/history`, { waitUntil: 'domcontentloaded' });
      await page
        .locator('[role="option"]')
        .first()
        .click({ timeout: 8000 })
        .catch(() => {});
    },
    async ready(page) {
      await page
        .locator('[role="option"]')
        .first()
        .waitFor({ timeout: 20000 })
        .catch(() => {});
    },
  },
  {
    name: 'status',
    theme: 'light',
    description: '工作区与提交面板',
    async prepare(page) {
      await openDemoRepo(page);
      await page.goto(`${baseURL}/#/repo/1/status`, { waitUntil: 'domcontentloaded' });
    },
    async ready(page) {
      await page
        .getByRole('heading', { name: '工作区' })
        .waitFor({ timeout: 20000 })
        .catch(() => {});
    },
  },
];

const devServer = await ensureDevServer();
mkdirSync(outputDir, { recursive: true });

const browser = await chromium.launch({ channel: process.env.PW_CHANNEL ?? 'msedge' });
try {
  for (const scene of SCENES) {
    const context = await browser.newContext({
      viewport: { width: 1440, height: 900 },
      deviceScaleFactor: 2,
      locale: 'zh-CN',
    });
    // 这个回调在**页面上下文**里执行（Playwright 会把它序列化后注入），
    // 所以 `window` 在这里合法；写成 `globalThis` 是为了过 lint 的 node 环境检查。
    await context.addInitScript(() => {
      globalThis.localStorage.setItem('forgedesk.language', 'zh-CN');
    });
    await context.addInitScript(
      `window.localStorage.setItem('forgedesk.theme', '${scene.theme}');`,
    );
    await context.addInitScript(mockScript);

    const page = await context.newPage();
    await scene.prepare(page);
    await scene.ready(page);
    // 把指针挪到角落：点击/悬停留下的浮层（如提交行的悬停卡片）会盖住内容，
    // 而截图要的是"用户看到的那一屏"，不是"鼠标停在哪就展示哪"
    await page.mouse.move(4, 4);
    // 等布局、悬停浮层收起与入场动画稳定：截图不是测试，宁可多给 800ms
    await page.waitForTimeout(800);

    const rawPath = join(outputDir, `.${scene.name}.raw.png`);
    await page.screenshot({ path: rawPath, fullPage: false });

    const target = join(outputDir, `${scene.name}.webp`);
    const info = await sharp(readFileSync(rawPath))
      .resize({ width: SHOT_WIDTH, withoutEnlargement: true })
      .webp({ quality: 86, effort: 5 })
      .toFile(target);
    rmSync(rawPath, { force: true });

    console.log(
      `✓ ${scene.description} → site/images/${scene.name}.webp（${Math.round(info.size / 1024)} KB）`,
    );
    await context.close();
  }
} finally {
  await browser.close();
  if (devServer) devServer.kill();
}

console.log('截图采集完成。');
