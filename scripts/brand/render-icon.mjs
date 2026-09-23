#!/usr/bin/env node
/**
 * 把原创矢量图标 (docs/brand/icon-source.svg) 栅格化为 1024x1024 PNG，
 * 供 `pnpm tauri icon` 生成各平台图标文件（.ico / .icns / 各尺寸 PNG）。
 *
 * 为什么单独做这一步：tauri icon 只接受 PNG 输入，而我们的设计真源是矢量图。
 * 每次改动矢量源后重新运行本脚本，保证图标始终可复现、可审计。
 *
 * 用法：node scripts/brand/render-icon.mjs
 */
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import sharp from 'sharp';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const BRAND_DIR = join(repoRoot, 'docs', 'brand');
const SVG_PATH = join(BRAND_DIR, 'icon-source.svg');
const PNG_PATH = join(BRAND_DIR, 'icon-1024.png');

const SIZE = 1024;

const svg = readFileSync(SVG_PATH, 'utf8');

if (!svg.includes('width="1024"') || !svg.includes('height="1024"')) {
  throw new Error('icon-source.svg 的画布必须是 1024x1024，否则各平台图标比例会出错。');
}

mkdirSync(BRAND_DIR, { recursive: true });

const png = await sharp(Buffer.from(svg), { density: 384 })
  .resize(SIZE, SIZE, { fit: 'contain', background: { r: 0, g: 0, b: 0, alpha: 0 } })
  .png({ compressionLevel: 9 })
  .toBuffer();

writeFileSync(PNG_PATH, png);

const meta = await sharp(png).metadata();
console.log(`已生成 ${PNG_PATH}`);
console.log(`  尺寸: ${meta.width}x${meta.height}  通道: ${meta.channels}  透明: ${meta.hasAlpha}`);
console.log('下一步: pnpm tauri icon docs/brand/icon-1024.png');
