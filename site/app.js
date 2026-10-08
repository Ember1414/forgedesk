/*
 * ForgeDesk 官网共享逻辑（零依赖原生 JS）。
 *
 * 数据源约定（与 docs/RELEASE.md、scripts/ci/make-updater-manifest.mjs 同一契约）：
 * - 发布清单：/updates/stable/windows-<target>.json —— 与应用内自动更新同一份数据，
 *   不走 GitHub Releases API（匿名限流 + 与清单两份真相源）。
 * - 校验和：/updates/stable/SHA256SUMS（发布流水线同源再放一份；GitHub 附件不带
 *   CORS 头，跨域取不到，同源是页面直显数值的唯一零依赖做法）。
 * - 产物命名由 rename-bundles.mjs 固定：ForgeDesk_<版本>_windows_<架构><扩展名>。
 *   命名约定若变更，本文件与 docs/RELEASE.md 必须同步改。
 *
 * 所有"没有数据"的状态都如实显示（无版本就不给下载按钮）——这是站点自检
 * （scripts/ci/check-site.mjs）钉住的行为，改动前先看它。
 */
(function () {
  'use strict';

  var CHANNEL = '/updates/stable';
  // GPG 公钥的公开地址：**唯一的真相源**——探测与链接都用它。
  // 曾经探测的是 SHA256SUMS.asc（存在）而链接指向 gpg-pubkey.asc（不存在），
  // 于是下载页上挂出一个必然拿到首页 HTML 的坏链接（v1.0.0 发布后实测发现）。
  var GPG_PUBKEY_URL = '/updates/gpg-pubkey.asc';
  var RELEASES_URL = 'https://github.com/Ember1414/forgedesk/releases';
  var API_RELEASES_URL = 'https://api.github.com/repos/Ember1414/forgedesk/releases';

  /*
   * 平台 → updater target 名（与发布流水线写出的清单名逐个对应）。
   *
   * macOS 是 **universal 构建**：同一个 .app.tar.gz 同时给 Apple Silicon 与 Intel，
   * 因此 aarch64 / x86_64 两份清单内容相同；页面向用户展示时只用 aarch64 那一份
   * （否则矩阵里会出现两行一模一样的 macOS 条目）。
   */
  var OS_META = {
    windows: { label: 'Windows', primary: 'windows-x86_64' },
    macos: { label: 'macOS', primary: 'darwin-aarch64' },
    linux: { label: 'Linux', primary: null },
  };

  /** 会去尝试拉取的全部清单：矩阵要把"这个版本有哪些平台"完整展示出来。 */
  var MANIFEST_TARGETS = ['windows-x86_64', 'windows-aarch64', 'darwin-aarch64', 'darwin-x86_64'];

  /*
   * 托管方（Cloudflare Pages）对**不存在的路径**返回的是 `200 + 首页 HTML` 兜底，
   * 而不是 404（实测：请求任意不存在的 .json 都拿到 200 text/html）。
   * 因此只判 `response.ok` 会把首页当成真实文件：清单会被当成 JSON 解析失败、
   * 校验和会被当成正文渲染、`.asc` 会被当成"存在"从而给出一个下载 HTML 的坏链接。
   * 这里统一把 HTML 兜底视同"没有这个文件"。
   */
  function isHtmlFallback(response) {
    var contentType = (response.headers && response.headers.get('content-type')) || '';
    return contentType.indexOf('text/html') === 0;
  }

  /* ---------- 平台识别（T7.9 第 8 点：userAgent 识别 + 手动切换兜底） ---------- */

  function storedOS() {
    try {
      var value = localStorage.getItem('forgedesk.os');
      return value === 'windows' || value === 'macos' || value === 'linux' ? value : null;
    } catch {
      return null;
    }
  }

  function detectOS() {
    if (navigator.userAgentData && typeof navigator.userAgentData.platform === 'string') {
      var platform = navigator.userAgentData.platform.toLowerCase();
      if (platform.indexOf('windows') !== -1) return 'windows';
      if (platform.indexOf('mac') !== -1) return 'macos';
    }
    var ua = navigator.userAgent;
    if (ua.indexOf('Windows NT') !== -1) return 'windows';
    if (ua.indexOf('Mac OS X') !== -1) return 'macos';
    if (ua.indexOf('Linux') !== -1 && ua.indexOf('Android') === -1) return 'linux';
    return 'windows';
  }

  function currentOS() {
    return storedOS() || detectOS();
  }

  function formatSize(bytes) {
    if (typeof bytes !== 'number' || !isFinite(bytes) || bytes <= 0) return null;
    var units = ['B', 'KB', 'MB', 'GB'];
    var value = bytes;
    var index = 0;
    while (value >= 1024 && index < units.length - 1) {
      value /= 1024;
      index += 1;
    }
    return (value >= 100 ? Math.round(value) : value.toFixed(1)) + ' ' + units[index];
  }

  /* 清单里的 platforms 按 target 合并成一个 release 视图；取不到就是 null。 */
  function loadRelease() {
    return Promise.all(
      MANIFEST_TARGETS.map(function (target) {
        return fetch(CHANNEL + '/' + target + '.json', { cache: 'no-store' }).then(
          function (response) {
            // 未发布时这里是 404，也可能被托管方返回 HTML 兜底页——都按"没有清单"处理
            if (!response.ok || isHtmlFallback(response)) return null;
            return response.json();
          },
        );
      }),
    ).then(function (manifests) {
      var release = null;
      manifests.forEach(function (manifest, index) {
        if (!manifest || !manifest.version) return;
        var target = MANIFEST_TARGETS[index];
        var entry = (manifest.platforms || {})[target];
        if (!entry || !entry.url) return;
        if (!release) {
          release = { version: manifest.version, pubDate: manifest.pub_date || '', targets: {} };
        }
        release.targets[target] = entry;
      });
      return release;
    });
  }

  function fetchChecksums() {
    return fetch(CHANNEL + '/SHA256SUMS', { cache: 'no-store' }).then(function (response) {
      if (!response.ok || isHtmlFallback(response)) throw new Error('no checksums');
      return response.text();
    });
  }

  /* ---------- 渲染：状态徽标 / 下载按钮 / 版本矩阵 ---------- */

  function renderStatus(el, release) {
    if (!el) return;
    el.textContent = release
      ? '最新版本 v' +
        release.version +
        (release.pubDate ? ' · ' + release.pubDate.slice(0, 10) : '')
      : '状态：尚无可用版本';
  }

  /** 平台 → 主推产物的说明文案（扩展名与平台的安装习惯一致）。 */
  var PRIMARY_FORMAT = { windows: '安装器 .exe', macos: '磁盘映像 .dmg' };

  /** 主下载按钮：识别到的平台有产物就给按钮，否则如实引导到 Releases。 */
  function renderCta(container, release) {
    if (!container) return;
    var os = currentOS();
    container.textContent = '';
    var target = OS_META[os].primary;
    var entry = target && release ? release.targets[target] : null;

    if (entry) {
      var button = document.createElement('a');
      button.className = 'btn primary';
      button.href = entry.url;
      var label = document.createElement('span');
      label.textContent = '下载 ' + OS_META[os].label + ' 版';
      var sub = document.createElement('span');
      sub.className = 'sub';
      var format = urlExtension(entry.url);
      sub.textContent = 'v' + release.version + ' · ' + (format || PRIMARY_FORMAT[os] || '');
      button.appendChild(label);
      button.appendChild(sub);
      container.appendChild(button);
    } else {
      var fallback = document.createElement('a');
      fallback.className = 'btn primary';
      fallback.href = RELEASES_URL;
      fallback.textContent =
        os === 'windows' ? '前往 Releases 下载' : OS_META[os].label + ' 版即将提供 · 前往 Releases';
      container.appendChild(fallback);
    }

    var secondary = document.createElement('a');
    secondary.className = 'btn secondary';
    secondary.href = 'download.html';
    secondary.textContent = '全部下载与校验';
    container.appendChild(secondary);
  }

  /** 从 URL 取扩展名（忽略查询串），用于展示"这一行是什么格式"。 */
  function urlExtension(url) {
    var clean = String(url).split(/[?#]/)[0];
    var dot = clean.lastIndexOf('.');
    return dot === -1 ? '' : clean.slice(dot + 1);
  }

  function artifactRows(release) {
    var rows = [];
    if (!release) return rows;
    Object.keys(release.targets).forEach(function (target) {
      var entry = release.targets[target];
      var base = entry.url.replace(/\/[^/]+$/, '');

      if (target === 'darwin-x86_64') {
        // universal 构建：x86_64 与 aarch64 清单指向同一个产物，矩阵只展示一行
        return;
      }

      if (target.indexOf('darwin') === 0) {
        var macPrefix = 'ForgeDesk_' + release.version + '_macos_universal';
        rows.push({
          platform: 'macOS universal',
          format: '磁盘映像 .dmg',
          size: null,
          url: base + '/' + macPrefix + '.dmg',
          checksumName: macPrefix + '.dmg',
        });
        rows.push({
          platform: 'macOS universal',
          format: '更新包 .app.tar.gz',
          size: formatSize(entry.size),
          url: entry.url,
          checksumName: macPrefix + '.app.tar.gz',
        });
        return;
      }

      var arch = target === 'windows-aarch64' ? 'arm64' : 'x64';
      var prefix = 'ForgeDesk_' + release.version + '_windows_' + arch;
      var platformLabel = 'Windows ' + arch;
      rows.push({
        platform: platformLabel,
        format: '安装器 .exe',
        size: formatSize(entry.size),
        url: entry.url,
        checksumName: prefix + '.exe',
      });
      rows.push({
        platform: platformLabel,
        format: '安装包 .msi',
        size: null,
        url: base + '/' + prefix + '.msi',
        checksumName: prefix + '.msi',
      });
      rows.push({
        platform: platformLabel,
        format: '便携版 .zip',
        size: null,
        url: base + '/' + prefix + '_portable.zip',
        checksumName: prefix + '_portable.zip',
      });
    });
    return rows;
  }

  /** 版本矩阵（T7.9：平台 × 格式 × 大小 × SHA256）。没有发布时整块保持隐藏。 */
  function renderMatrix(wrap, release, checksums) {
    if (!wrap) return;
    var rows = artifactRows(release);
    if (rows.length === 0) return;
    var table = document.createElement('table');
    table.className = 'matrix';
    var head = document.createElement('thead');
    var headRow = document.createElement('tr');
    ['平台', '格式', '大小', '下载', 'SHA256'].forEach(function (title) {
      var th = document.createElement('th');
      th.scope = 'col';
      th.textContent = title;
      headRow.appendChild(th);
    });
    head.appendChild(headRow);
    table.appendChild(head);
    var body = document.createElement('tbody');
    rows.forEach(function (row) {
      var tr = document.createElement('tr');
      var cells = [row.platform, row.format, row.size === null ? '—' : row.size, null, null];
      cells.forEach(function (value, index) {
        var td = document.createElement('td');
        if (index === 3) {
          var link = document.createElement('a');
          link.href = row.url;
          link.textContent = '下载';
          td.appendChild(link);
        } else if (index === 4) {
          if (checksums) {
            var sha = document.createElement('a');
            sha.href = CHANNEL + '/SHA256SUMS';
            sha.title = '在 SHA256SUMS 中查看 ' + row.checksumName;
            sha.textContent = row.checksumName.slice(0, 16) + '…';
            td.appendChild(sha);
          } else {
            td.textContent = '随发布提供';
          }
        } else {
          td.textContent = value;
        }
        tr.appendChild(td);
      });
      body.appendChild(tr);
    });
    table.appendChild(body);
    wrap.textContent = '';
    wrap.appendChild(table);
    wrap.hidden = false;
  }

  /* ---------- 校验和 / GPG 公钥 ---------- */

  function renderChecksums(pre, release) {
    if (!pre) return Promise.resolve(false);
    if (!release) return Promise.resolve(false);
    return fetchChecksums()
      .then(function (text) {
        // 只接受标准 sha256sum 行：宁可什么都不显示，也不展示疑似校验和的内容
        var lines = text
          .split('\n')
          .filter(function (line) {
            return /^[0-9a-f]{64} {2}\S+$/.test(line.trim());
          })
          .map(function (line) {
            return line.trim();
          });
        if (lines.length === 0) return false;
        pre.textContent = lines.join('\n');
        return true;
      })
      .catch(function () {
        return false;
      });
  }

  function probeByUrl(url) {
    return fetch(url, { cache: 'no-store' }).then(function (response) {
      // HTML 兜底 = 这个文件其实不存在（缺 GPG 签名时就是这种情形）
      if (!response.ok || isHtmlFallback(response)) throw new Error('missing');
      return true;
    });
  }

  /* ---------- 更新日志（T7.9 第 4 点：运行时从 GitHub Releases 拉取） ---------- */

  function renderChangelog(container) {
    if (!container) return;
    fetch(API_RELEASES_URL + '?per_page=10', {
      headers: { Accept: 'application/vnd.github+json' },
    })
      .then(function (response) {
        if (!response.ok) throw new Error('releases unavailable');
        return response.json();
      })
      .then(function (releases) {
        if (!Array.isArray(releases) || releases.length === 0) {
          showChangelogEmpty(container);
          return;
        }
        container.textContent = '';
        releases.forEach(function (release) {
          var article = document.createElement('article');
          article.className = 'doc-body';
          article.style.marginBottom = '28px';
          var heading = document.createElement('h2');
          heading.textContent = release.name || release.tag_name;
          var meta = document.createElement('p');
          meta.className = 'note';
          meta.textContent =
            release.tag_name +
            (release.published_at ? ' · ' + release.published_at.slice(0, 10) : '');
          var body = document.createElement('pre');
          body.style.whiteSpace = 'pre-wrap';
          body.textContent = release.body || '（无说明）';
          article.appendChild(heading);
          article.appendChild(meta);
          article.appendChild(body);
          container.appendChild(article);
        });
      })
      .catch(function () {
        showChangelogEmpty(container);
      });
  }

  function showChangelogEmpty(container) {
    container.textContent = '';
    var note = document.createElement('p');
    note.className = 'note';
    note.textContent = '尚未发布任何正式版本，或暂时无法读取 Releases。';
    var link = document.createElement('a');
    link.href = RELEASES_URL;
    link.textContent = '前往 GitHub Releases 查看';
    container.appendChild(note);
    container.appendChild(link);
  }

  /* ---------- 页面接线：按元素存在与否自动生效 ---------- */

  document.addEventListener('DOMContentLoaded', function () {
    // 导航当前页高亮
    var page = document.body.getAttribute('data-page');
    if (page) {
      var nav = document.querySelector('.site-nav a.item[data-nav="' + page + '"]');
      if (nav) nav.setAttribute('aria-current', 'page');
    }

    /*
     * ---------- 滚动相关的观感增强 ----------
     *
     * 全部是**渐进增强**：页面内容默认就完整可见，这里只负责"动起来"。
     * 因此每一步都要能安全跳过——jsdom（官网自检）没有 IntersectionObserver、
     * 没有 matchMedia，也不该因为这些缺失而让自检报错。
     */
    var reduceMotion = false;
    try {
      reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    } catch (error) {
      reduceMotion = false;
    }

    if (!reduceMotion && typeof window.IntersectionObserver === 'function') {
      // 先藏后显这件事交给 JS 加类：没有 JS / 不支持观察器时元素保持可见
      document.documentElement.classList.add('js-reveal');
      var revealObserver = new window.IntersectionObserver(
        function (entries) {
          entries.forEach(function (entry) {
            if (!entry.isIntersecting) return;
            entry.target.classList.add('is-visible');
            revealObserver.unobserve(entry.target);
          });
        },
        { rootMargin: '0px 0px -6% 0px', threshold: 0.08 },
      );
      Array.prototype.forEach.call(document.querySelectorAll('[data-reveal]'), function (element) {
        revealObserver.observe(element);
      });
    }

    // 阅读进度条与导航滚动态：同样由 JS 生成（无 JS 时页面不会多一根静止的线）
    var progress = document.createElement('div');
    progress.className = 'scroll-progress';
    progress.setAttribute('aria-hidden', 'true');
    document.body.appendChild(progress);

    var navBar = document.querySelector('.site-nav');
    var updateScrollChrome = function () {
      var doc = document.documentElement;
      var scrollable = doc.scrollHeight - window.innerHeight;
      var ratio = scrollable > 0 ? Math.min(1, Math.max(0, window.scrollY / scrollable)) : 0;
      progress.style.width = (ratio * 100).toFixed(2) + '%';
      if (navBar) navBar.classList.toggle('is-scrolled', window.scrollY > 8);
    };
    updateScrollChrome();
    window.addEventListener('scroll', updateScrollChrome, { passive: true });
    window.addEventListener('resize', updateScrollChrome);

    // 复制按钮：<button data-copy-target="#元素id">
    Array.prototype.forEach.call(
      document.querySelectorAll('[data-copy-target]'),
      function (button) {
        button.addEventListener('click', function () {
          var target = document.querySelector(button.getAttribute('data-copy-target'));
          if (!target) return;
          var done = function (text) {
            button.textContent = text;
            setTimeout(function () {
              button.textContent = '复制';
            }, 2000);
          };
          if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(target.textContent).then(
              function () {
                done('已复制');
              },
              function () {
                done('复制失败，请手动选择');
              },
            );
          } else {
            done('复制失败，请手动选择');
          }
        });
      },
    );

    // OS 手动切换（覆盖 userAgent 识别，存 localStorage）
    var switcher = document.getElementById('os-switch');
    if (switcher) {
      var paint = function () {
        var os = currentOS();
        Array.prototype.forEach.call(switcher.querySelectorAll('button'), function (button) {
          button.setAttribute(
            'aria-pressed',
            button.getAttribute('data-os') === os ? 'true' : 'false',
          );
        });
      };
      paint();
      switcher.addEventListener('click', function (event) {
        var button = event.target.closest('button[data-os]');
        if (!button) return;
        try {
          localStorage.setItem('forgedesk.os', button.getAttribute('data-os'));
        } catch {
          /* 存不进就用会话内值 */
        }
        paint();
        refresh();
      });
    }

    var statusEl = document.getElementById('status');
    var ctaEl = document.getElementById('download-cta');
    var matrixWrap = document.getElementById('matrix-wrap');
    var matrixEmpty = document.getElementById('matrix-empty');
    var matrixNote = document.getElementById('matrix-note');
    var checksumPre = document.getElementById('checksum-lines');
    var checksumDetails = document.getElementById('checksums');

    var needsRelease = Boolean(statusEl || ctaEl || matrixWrap || checksumPre);

    function refresh() {
      var pending = needsRelease ? loadRelease() : Promise.resolve(null);
      return pending
        .then(function (release) {
          renderStatus(statusEl, release);
          renderCta(ctaEl, release);
          if (matrixWrap) {
            // 版本矩阵只要有清单就渲染（SHA256 列在没有校验和文件时如实降级为文字）
            if (release) {
              if (matrixEmpty) matrixEmpty.hidden = true;
              if (matrixNote) matrixNote.hidden = false;
            }
            renderChecksums(checksumPre, release).then(function (ok) {
              renderMatrix(matrixWrap, release, ok);
              if (checksumDetails && ok) checksumDetails.hidden = false;
            });
          } else {
            renderChecksums(checksumPre, release).then(function (ok) {
              if (checksumDetails && ok) checksumDetails.hidden = false;
            });
          }
          // GPG 公钥：**探测的必须是链接要指向的那个文件**。曾经探测
          // SHA256SUMS.asc（它存在）却链接到 gpg-pubkey.asc（不存在），
          // 结果是一个点了只会拿到首页 HTML 的坏链接。href 也从同一常量写入，
          // 让"探测什么"与"链接到哪"不可能再分叉。
          var gpgSection = document.getElementById('gpg-key');
          if (gpgSection) {
            probeByUrl(GPG_PUBKEY_URL)
              .then(function () {
                var link = gpgSection.querySelector('a');
                if (link) {
                  link.href = GPG_PUBKEY_URL;
                }
                gpgSection.hidden = false;
                // 占位说明写的是"公钥尚未发布"：公钥在了就必须收起它，
                // 否则页面上会同时出现"可下载公钥"与"尚未发布"两句互相打脸的话
                var placeholder = document.getElementById('gpg-placeholder');
                if (placeholder) {
                  placeholder.hidden = true;
                }
              })
              .catch(function () {
                /* 保持隐藏：占位文案已在静态内容里 */
              });
          }
        })
        .catch(function () {
          /* 网络失败：保持静态兜底文案 */
        });
    }

    void refresh();

    renderChangelog(document.getElementById('changelog-body'));
  });
})();
