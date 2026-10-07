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
  var RELEASES_URL = 'https://github.com/Ember1414/forgedesk/releases';
  var API_RELEASES_URL = 'https://api.github.com/repos/Ember1414/forgedesk/releases';

  var OS_META = {
    windows: { label: 'Windows', targets: ['windows-x86_64', 'windows-aarch64'] },
    macos: { label: 'macOS', targets: [] },
    linux: { label: 'Linux', targets: [] },
  };

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
    var targets = ['windows-x86_64', 'windows-aarch64'];
    return Promise.all(
      targets.map(function (target) {
        return fetch(CHANNEL + '/' + target + '.json', { cache: 'no-store' }).then(
          function (response) {
            // 未发布时这里是 404，也可能被托管方返回 HTML 兜底页——都按"没有清单"处理
            if (!response.ok) return null;
            return response.json();
          },
        );
      }),
    ).then(function (manifests) {
      var release = null;
      manifests.forEach(function (manifest, index) {
        if (!manifest || !manifest.version) return;
        var target = targets[index];
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
      if (!response.ok) throw new Error('no checksums');
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

  /** 主下载按钮：识别到的平台有产物就给按钮，否则如实引导到 Releases。 */
  function renderCta(container, release) {
    if (!container) return;
    var os = currentOS();
    container.textContent = '';
    var target = os === 'windows' ? 'windows-x86_64' : null;
    var entry = target && release ? release.targets[target] : null;

    if (entry) {
      var button = document.createElement('a');
      button.className = 'btn primary';
      button.href = entry.url;
      var label = document.createElement('span');
      label.textContent = '下载 Windows 版';
      var sub = document.createElement('span');
      sub.className = 'sub';
      sub.textContent = 'v' + release.version + ' · 安装器 .exe';
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

  function artifactRows(release) {
    var rows = [];
    if (!release) return rows;
    Object.keys(release.targets).forEach(function (target) {
      var entry = release.targets[target];
      var base = entry.url.replace(/\/[^/]+$/, '');
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
      if (!response.ok) throw new Error('missing');
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
          // GPG 公钥：发布了 .asc 才展示下载入口，否则如实说明"随首次发布提供"
          var gpgSection = document.getElementById('gpg-key');
          if (gpgSection) {
            probeByUrl(CHANNEL + '/SHA256SUMS.asc')
              .then(function () {
                gpgSection.hidden = false;
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
