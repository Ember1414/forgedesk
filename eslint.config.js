import js from '@eslint/js';
import reactHooks from 'eslint-plugin-react-hooks';
import globals from 'globals';
import tseslint from 'typescript-eslint';

/** 兼容 eslint-plugin-react-hooks 不同大版本的 flat config 导出差异。 */
const reactHooksRecommended =
  reactHooks.configs['recommended-latest'] ?? reactHooks.configs.recommended;

/**
 * 架构护栏：业务代码一律通过 src/lib/ipc 访问 Tauri，
 * 禁止在 feature / ui 层直接 import 底层 API（AGENTS.md §6「前后端严格分层」）。
 */
const tauriImportGuard = [
  'error',
  {
    patterns: [
      {
        group: [
          '@tauri-apps/api/core',
          '@tauri-apps/api/event',
          '@tauri-apps/api/window',
          '@tauri-apps/api/webview',
          '@tauri-apps/plugin-*',
        ],
        message: '禁止直接调用 Tauri 底层 API，请统一通过 src/lib/ipc 暴露的封装。',
      },
    ],
  },
];

export default tseslint.config(
  {
    ignores: [
      'dist/**',
      'coverage/**',
      'playwright-report/**',
      'test-results/**',
      'node_modules/**',
      // Rust 构建产物（Tauri 会在里面生成 __global-api-script.js 与二进制资源）。
      // ESLint 不会自动读取 .gitignore，必须显式排除，否则本地构建过一次后 lint 必然失败。
      'target/**',
      '**/target/**',
      // Rust 源码由 cargo clippy 负责，ESLint 不参与
      'src-tauri/**',
      // Tauri 生成的权限 schema
      'src-tauri/gen/**',
      '.cargo/**',
    ],
  },

  { files: ['**/*.{js,mjs,cjs}'], languageOptions: { globals: { ...globals.node } } },

  {
    // 官网（site/）是直接跑在浏览器里的零构建脚本：全局对象与 Node 脚本不同。
    // 它不参与应用构建（无 tsconfig 覆盖），因此必须在 JS 层单独声明环境。
    files: ['site/**/*.js'],
    languageOptions: { globals: { ...globals.browser } },
  },

  js.configs.recommended,
  ...tseslint.configs.recommended,

  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2023,
      sourceType: 'module',
      globals: { ...globals.browser },
    },
    plugins: { 'react-hooks': reactHooks },
    rules: {
      ...(reactHooksRecommended?.rules ?? {}),
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrorsIgnorePattern: '^_' },
      ],
      '@typescript-eslint/consistent-type-imports': ['error', { prefer: 'type-imports' }],
      'no-restricted-imports': tauriImportGuard,
      eqeqeq: ['error', 'always', { null: 'ignore' }],
      'no-console': ['warn', { allow: ['warn', 'error'] }],
    },
  },

  {
    files: ['src/lib/ipc/**/*.ts'],
    rules: { 'no-restricted-imports': 'off' },
  },
);
