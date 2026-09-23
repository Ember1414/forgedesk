import { fileURLToPath, URL } from 'node:url';

import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
// 使用 vitest/config 的 defineConfig：它带有 test 字段的类型定义，
// 同时完全兼容 Vite 配置。
import { defineConfig } from 'vitest/config';

const isTauri = process.env['TAURI_ENV_PLATFORM'] !== undefined;
const isWindowsTarget = process.env['TAURI_ENV_PLATFORM'] === 'windows';

export default defineConfig({
  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },

  // Tauri 期望固定端口，且不要在终端清屏（会刷掉 Rust 侧日志）
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // exactOptionalPropertyTypes 下不能显式赋 undefined，故用条件展开
    ...(isTauri ? { host: '127.0.0.1' } : {}),
    watch: {
      ignored: ['**/src-tauri/**', '**/target/**'],
    },
  },

  envPrefix: ['VITE_', 'TAURI_ENV_'],

  build: {
    // 各平台 WebView 引擎不同：Windows 用 WebView2(Chromium)，macOS/Linux 用 WebKit
    target: isWindowsTarget ? 'chrome110' : 'safari15',

    // 注意：不要写 'esbuild'。Vite 8 使用 rolldown 内核，已不再内置 esbuild，
    // 显式指定 'esbuild' 会触发已废弃的 vite:esbuild-transpile 插件并报
    // "Cannot find package 'esbuild'"。用 true 让其走默认压缩器（oxc）。
    minify: isTauri,

    sourcemap: !isTauri,
    chunkSizeWarningLimit: 1500,
  },

  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
    // 只统计业务代码的覆盖率：测试自身、类型声明与开发用预览页不计入
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov'],
      include: ['src/**/*.{ts,tsx}'],
      exclude: [
        'src/**/*.{test,spec}.{ts,tsx}',
        'src/test/**',
        'src/vite-env.d.ts',
        'src/ui/__dev__/**',
        'src/main.tsx',
      ],
    },
  },
});
