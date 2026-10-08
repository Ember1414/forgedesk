#!/usr/bin/env node
/**
 * 官网站点生成器（T7.9 第 3/5 点）：把 `docs/` 的用户手册、隐私说明与许可证
 * 渲染成 site/ 下的静态 HTML——**构建时读取 docs**，保证"同源内容、一处维护"。
 *
 * # 为什么不用 VitePress / Astro（任务书原方案）
 *
 * 站点的核心数据源是**发布清单**（与应用内自动更新同一份），页面只需要把它读出来；
 * 为此引入一套静态站生成器，只会多一条需要维护、且可能与清单漂移的链路。
 * 折中方案：手写页面保持零构建静态 HTML，本脚本只负责"docs → HTML"这一段
 * **构建时转换**（只在部署前由 pages.yml / release.yml / check-site 运行），
 * 运行时依旧零依赖。产物不入库（见 .gitignore），真相源永远是 docs/。
 *
 * # Markdown 支持范围（刻意最小化）
 *
 * 手册与隐私文档只用到了：标题、段落、有序/无序列表、引用、表格、围栏代码块、
 * 行内代码、粗体、斜体、链接。本转换器只实现这些——它能覆盖**仓库内受控的**
 * 文档，不追求通用 Markdown 兼容；遇到不认识的语法按普通段落渲染，不会崩。
 *
 * 用法：node scripts/ci/build-site.mjs
 */
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const SITE = join(repoRoot, 'site');
const MANUAL = join(repoRoot, 'docs', 'manual');
const GITHUB_BLOB = 'https://github.com/Ember1414/forgedesk/blob/main';

// ---------------------------------------------------------------- 极简 Markdown → HTML

/** 行内语法：转义 HTML 之后依次替换行内代码 / 粗体 / 斜体 / 链接 / 图片。 */
function renderInline(text, resolveLink) {
  const escaped = text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
  const withCode = escaped.replace(/`([^`]+)`/g, '<code>$1</code>');
  const withImages = withCode.replace(
    /!\[([^\]]*)\]\(([^)\s]+)\)/g,
    (match, alt, url) => `<a href="${resolveLink(url)}">${alt || url}</a>`,
  );
  return withImages
    .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (match, text, url) => {
      const href = resolveLink(url);
      const external = /^https?:/.test(href);
      // 手册互链的原文常直接写文件名（01-xxx.md，常包在行内代码里）：
      // 展示给用户时去掉 .md 扩展名
      const label =
        url.endsWith('.md') && text.includes(url.split('/').pop())
          ? text.replace(/\.md(<\/code>)?$/, '$1')
          : text;
      return `<a href="${href}"${external ? ' target="_blank" rel="noopener"' : ''}>${label}</a>`;
    })
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/(^|[^*])\*([^*]+)\*/g, '$1<em>$2</em>');
}

function renderMarkdown(markdown, resolveLink) {
  const lines = markdown.split(/\r?\n/);
  const out = [];
  let paragraph = [];
  let list = null; // 'ul' | 'ol'
  let inCode = false;
  let codeLines = [];
  let tableBuffer = [];

  const flushParagraph = () => {
    if (paragraph.length > 0) {
      out.push(`<p>${renderInline(paragraph.join(' '), resolveLink)}</p>`);
      paragraph = [];
    }
  };
  const flushList = () => {
    if (list !== null) {
      out.push(`</${list}>`);
      list = null;
    }
  };
  const flushTable = () => {
    if (tableBuffer.length === 0) return;
    const rows = tableBuffer
      .filter(
        (row) => !/^\s*\|?[\s:|-]+\|?\s*$/.test(row) || row.replace(/[^|-]/g, '').length === 0,
      )
      .map((row) =>
        row
          .replace(/^\s*\|/, '')
          .replace(/\|\s*$/, '')
          .split('|')
          .map((cell) => renderInline(cell.trim(), resolveLink)),
      );
    if (rows.length > 0) {
      const [head, ...body] = rows;
      out.push('<div class="table-wrap"><table class="matrix"><thead><tr>');
      for (const cell of head) out.push(`<th>${cell}</th>`);
      out.push('</tr></thead><tbody>');
      for (const row of body) {
        out.push('<tr>');
        for (const cell of row) out.push(`<td>${cell}</td>`);
        out.push('</tr>');
      }
      out.push('</tbody></table></div>');
    }
    tableBuffer = [];
  };

  for (const rawLine of lines) {
    const line = rawLine.replace(/\s+$/, '');

    if (inCode) {
      if (/^```/.test(line)) {
        out.push(`<pre><code>${codeLines.join('\n')}</code></pre>`);
        codeLines = [];
        inCode = false;
      } else {
        codeLines.push(rawLine);
      }
      continue;
    }
    if (/^```/.test(line)) {
      flushParagraph();
      flushList();
      flushTable();
      inCode = true;
      continue;
    }

    // 表格行（含分隔行）先缓冲，遇到非表格行再整体落盘
    if (/^\s*\|.*\|\s*$/.test(line)) {
      flushParagraph();
      flushList();
      tableBuffer.push(line);
      continue;
    }
    flushTable();

    if (line.trim() === '') {
      flushParagraph();
      flushList();
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      flushParagraph();
      flushList();
      const level = String(heading[1].length);
      out.push(`<h${level}>${renderInline(heading[2], resolveLink)}</h${level}>`);
      continue;
    }
    if (/^(-{3,}|\*{3,})$/.test(line.trim())) {
      flushParagraph();
      flushList();
      out.push('<hr />');
      continue;
    }
    const quote = /^>\s?(.*)$/.exec(line);
    if (quote) {
      flushParagraph();
      flushList();
      out.push(`<blockquote>${renderInline(quote[1], resolveLink)}</blockquote>`);
      continue;
    }
    const unordered = /^[-*]\s+(.*)$/.exec(line.trim());
    if (unordered) {
      flushParagraph();
      if (list !== 'ul') {
        flushList();
        out.push('<ul>');
        list = 'ul';
      }
      out.push(`<li>${renderInline(unordered[1], resolveLink)}</li>`);
      continue;
    }
    const ordered = /^\d+[.)]\s+(.*)$/.exec(line.trim());
    if (ordered) {
      flushParagraph();
      if (list !== 'ol') {
        flushList();
        out.push('<ol>');
        list = 'ol';
      }
      out.push(`<li>${renderInline(ordered[1], resolveLink)}</li>`);
      continue;
    }
    paragraph.push(line.trim());
  }
  if (inCode) {
    out.push(`<pre><code>${codeLines.join('\n')}</code></pre>`);
  }
  flushParagraph();
  flushList();
  flushTable();
  return out.join('\n');
}

// ---------------------------------------------------------------- 页面外壳

function escapeHtml(text) {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

const MARK_SVG =
  '<svg width="17" height="17" viewBox="0 0 1024 1024" fill="#ffffff" aria-hidden="true">' +
  '<path d="M512 198 L560 262 L512 326 L464 262 Z" />' +
  '<path d="M400 306 L424 340 L400 374 L376 340 Z" />' +
  '<path d="M624 306 L648 340 L624 374 L600 340 Z" />' +
  '<rect x="256" y="512" width="512" height="104" rx="30" />' +
  '<rect x="432" y="616" width="160" height="120" rx="16" />' +
  '<rect x="320" y="736" width="384" height="88" rx="26" /></svg>';

/** 生成一整页（与手写页面同一导航/页脚；prefix 为资源路径前缀）。 */
function shell({ title, description, path, body, dataPage, prefix }) {
  const item = (nav, href, label) =>
    `<a class="item" data-nav="${nav}" href="${prefix}${href}">${label}</a>`;
  const link = (href, label) => `<a href="${prefix}${href}">${label}</a>`;
  const external = (href, label) => `<a href="${href}" target="_blank" rel="noopener">${label}</a>`;
  // 星标按钮保持与手写页一致的外观（class="star" + 图标）；生成页与手写页
  // 的导航必须是同一种东西，否则用户从首页点进手册会看到导航"变了个样"
  const star = `<a class="star" href="https://github.com/Ember1414/forgedesk" title="在 GitHub 上给 ForgeDesk 点星" target="_blank" rel="noopener"><svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M12 2l2.9 6.3 6.9.8-5.1 4.7 1.4 6.8L12 17.2 5.9 20.6l1.4-6.8L2.2 9.1l6.9-.8L12 2z" /></svg>Star</a>`;
  return `<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <meta name="color-scheme" content="light dark" />
    <meta name="description" content="${escapeHtml(description)}" />
    <title>${escapeHtml(title)}</title>
    <link rel="canonical" href="https://forgedesk.pages.dev${path}" />
    <meta property="og:type" content="website" />
    <meta property="og:url" content="https://forgedesk.pages.dev${path}" />
    <meta property="og:title" content="${escapeHtml(title)}" />
    <meta property="og:description" content="${escapeHtml(description)}" />
    <meta property="og:image" content="https://forgedesk.pages.dev/og.png" />
    <meta name="twitter:card" content="summary" />
    <link rel="icon" href="/favicon.png" />
    <link rel="stylesheet" href="/assets.css" />
  </head>
  <body data-page="${dataPage}">
    <header class="site-nav">
      <a class="brand" href="${prefix}index.html">
        <span class="mark" aria-hidden="true">${MARK_SVG}</span>
        ForgeDesk
      </a>
      ${item('download', 'download.html', '下载')}
      ${item('changelog', 'changelog.html', '更新日志')}
      ${item('docs', 'docs/index.html', '文档')}
      ${item('about', 'about.html', '关于')}
      ${star}
    </header>

    <main class="page doc-body">
${body}
    </main>

    <footer class="site-footer">
      <div class="inner">
        <p>
          ${link('index.html', '首页')} ·
          ${link('download.html', '下载')} ·
          ${link('docs/index.html', '用户手册')} ·
          ${external('https://github.com/Ember1414/forgedesk/blob/main/docs/FAQ.md', '常见问题')} ·
          ${external('https://github.com/Ember1414/forgedesk/issues', '问题反馈')} ·
          ${external('https://github.com/Ember1414/forgedesk/blob/main/SECURITY.md', '安全策略')} ·
          ${link('privacy.html', '隐私')} ·
          ${link('license.html', '许可证')} ·
          ${link('about.html', '关于')}
        </p>
        <p>
          ForgeDesk is an independent project. It is not affiliated with, endorsed by, or
          sponsored by the Git project, the Software Freedom Conservancy, GitHub, Inc., or the
          Tauri project.
        </p>
      </div>
    </footer>

    <script defer src="/app.js"></script>
  </body>
</html>
`;
}

/** 手册内相对链接的解析：手册互链 → 生成页；出手册目录的 → GitHub。 */
function manualLinkResolver() {
  return (url) => {
    if (/^https?:/i.test(url)) return url;
    if (url.startsWith('#')) return url;
    // 手册内的 .md 互链（同目录）：指向生成出来的同目录 HTML（README 例外 → index）
    const target = url.replace(/^\.\//, '');
    if (target.endsWith('.md') && !target.startsWith('../')) {
      const htmlName = target === 'README.md' ? 'index.html' : target.replace(/\.md$/, '.html');
      return htmlName;
    }
    return `${GITHUB_BLOB}/docs/manual/${target}`;
  };
}

/** docs/ 根下文档的相对链接（../X.md 之类）：一律指向 GitHub。 */
const docsRootLinkResolver = (url) => {
  if (/^https?:/i.test(url)) return url;
  if (url.startsWith('#')) return url;
  const cleaned = url.replace(/^\.\//, '');
  return `${GITHUB_BLOB}/docs/${cleaned}`;
};

// ---------------------------------------------------------------- 生成

function readOrExit(path, label) {
  if (!existsSync(path)) {
    console.error(`站点生成失败：缺少${label}（${path}）。`);
    process.exit(1);
  }
  return readFileSync(path, 'utf8');
}

const manualFiles = existsSync(MANUAL)
  ? readdirSync(MANUAL)
      .filter((name) => name.endsWith('.md'))
      .sort()
  : [];
if (manualFiles.length === 0) {
  console.error(`站点生成失败：${MANUAL} 下没有 Markdown 文件。`);
  process.exit(1);
}

mkdirSync(join(SITE, 'docs'), { recursive: true });

// 1) 用户手册：目录页（README → index.html）+ 每篇文章一页（同源内容，互链改生成页互链）
for (const file of manualFiles) {
  const markdown = readOrExit(join(MANUAL, file), `用户手册文件 ${file}`);
  // README.md 是手册目录：生成 index.html（导航与外链都指向 docs/index.html）
  const htmlName = file === 'README.md' ? 'index.html' : file.replace(/\.md$/, '.html');
  const titleMatch = /^#\s+(.+)$/m.exec(markdown);
  const title = titleMatch ? titleMatch[1] : `ForgeDesk 手册 · ${file}`;
  writeFileSync(
    join(SITE, 'docs', htmlName),
    shell({
      title: `${title} · ForgeDesk`,
      description: 'ForgeDesk 用户手册：只描述已经能用的功能。',
      path: `/docs/${htmlName}`,
      dataPage: 'docs',
      prefix: '../',
      body: renderMarkdown(markdown, manualLinkResolver()),
    }),
  );
  console.log(`  生成 docs/${htmlName}`);
}

// 2) 隐私说明（docs/PRIVACY.md 同源渲染）
const privacy = readOrExit(join(repoRoot, 'docs', 'PRIVACY.md'), '隐私文档');
writeFileSync(
  join(SITE, 'privacy.html'),
  shell({
    title: '隐私说明 · ForgeDesk',
    description: 'ForgeDesk 的数据清单与全部对外请求说明。无遥测、无 AI。',
    path: '/privacy',
    dataPage: 'privacy',
    prefix: '',
    body: renderMarkdown(privacy, docsRootLinkResolver),
  }),
);
console.log('  生成 privacy.html');

// 3) 许可证（LICENSE 原文，纯文本按 pre 渲染避免被当 Markdown 误解析）
const license = readOrExit(join(repoRoot, 'LICENSE'), '许可证文件');
writeFileSync(
  join(SITE, 'license.html'),
  shell({
    title: '许可证 · ForgeDesk',
    description: 'ForgeDesk 的开源许可证全文。',
    path: '/license',
    dataPage: 'license',
    prefix: '',
    body: `<h1>许可证</h1>\n<p class="note">以下为仓库 LICENSE 原文（构建时同步，避免两处维护）。</p>\n<pre>${escapeHtml(
      license,
    )}</pre>`,
  }),
);
console.log('  生成 license.html');

// 4) GPG 公钥随站发布（站点是**生成物**，真相源是 docs/keys/forgedesk-release.pub）
//
// 下载页探测 /updates/gpg-pubkey.asc 并在取到时给出下载入口。此前这个路径
// 从来没有文件（探测必失败 → 入口永远隐藏，页面上只剩"公钥尚未随站发布"的说明）。
// 这里按"生成页"的同一条规矩办：从真相源复制到站点目录，构建时同步。
// 源缺失时**删除**站点里的旧副本——宁可不提供公钥，也不能让用户拿旧密钥
// 去验新签名（轮换后最危险的状态）。
const pubkeySource = join(repoRoot, 'docs', 'keys', 'forgedesk-release.pub');
const pubkeyTarget = join(SITE, 'updates', 'gpg-pubkey.asc');
if (existsSync(pubkeySource)) {
  mkdirSync(join(SITE, 'updates'), { recursive: true });
  writeFileSync(pubkeyTarget, readFileSync(pubkeySource));
  console.log('  生成 updates/gpg-pubkey.asc（来自 docs/keys/forgedesk-release.pub）');
} else {
  rmSync(pubkeyTarget, { force: true });
  console.log('  docs/keys/forgedesk-release.pub 不存在：跳过公钥（站点不提供 GPG 入口）');
}

console.log('站点生成完成。');
