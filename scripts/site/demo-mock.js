/*
 * 官网截图用的演示宿主（普通脚本，不是模块——它会被原样注入页面）。
 *
 * 与 e2e 的做法完全一致：应用脚本运行前注入 `window.__TAURI_INTERNALS__`，
 * 把 IPC 命令路由到内存夹具。因此截图里是**真实界面**（真实组件、真实样式、
 * 真实提交图渲染），只有数据是这份演示夹具。
 *
 * 数据是虚构的（作者写 EMBER、路径写 D:/work/...）：截图会出现在公开网站上，
 * 不该带上任何真实仓库、真实提交或个人信息。
 */
(function () {
  var DEMO_COMMITS = [
    ['feat: 提交面板支持逐块暂存', ['HEAD -> main', 'origin/main'], 0, false],
    ['fix: 冲突面板在 CRLF 文件上的合并标记', [], 0, false],
    ['docs: 手册补上变基安全网一章', ['tag: v1.0.0'], 1, false],
    ['refactor: 快照清单改为惰性加载', [], 2, false],
    ['feat: 分支比较支持只看未合并', ['feature/branch-compare'], 2, false],
    ['test: 快照回滚的破坏性矩阵补两条', [], 1, false],
    ['chore(deps): 升级 tauri-plugin-updater', [], 0, false],
    ['perf: 10 万提交的图布局预热', [], 0, false],
    ['feat: 合并 main 到 feature/pty', [], 0, true],
    ['fix: 终端在中文输入法下的回车', [], 1, false],
    ['feat: 交互式变基的拖拽排序', ['feature/rebase-panel'], 1, false],
    ['docs: 插件 SDK 的清单字段说明', [], 2, false],
    ['fix: Windows 路径规范化导致的状态误报', [], 2, false],
    ['feat: 凭据只存系统钥匙串', [], 1, false],
    ['refactor: 审计导出走统一分页查询', [], 0, false],
  ];

  var listeners = [];
  window.__errs = [];

  function zeroPad(value) {
    return String(value).padStart(40, '0');
  }
  function sig() {
    return { name: 'EMBER', email: 'ember@forgedesk.invalid', time: 1791400000 };
  }

  var commits = DEMO_COMMITS.map(function (item, index) {
    return {
      oid: zeroPad(index + 1),
      subject: item[0],
      refs: item[1],
      lane: item[2],
      isMerge: item[3],
    };
  });

  var fullCommits = commits.map(function (commit) {
    return {
      oid: commit.oid,
      parents: [],
      author: sig(),
      committer: sig(),
      refs: commit.refs,
      signature: 'good',
      subject: commit.subject,
      body: null,
    };
  });
  var rows = commits.map(function (commit, index) {
    return {
      oid: commit.oid,
      row: index,
      lane: commit.lane,
      colorIndex: commit.lane % 8,
      isMerge: commit.isMerge,
      hidden: false,
      collapsed: [],
    };
  });
  var edges = [];
  for (var i = 1; i < commits.length; i++) {
    var kind = commits[i].isMerge
      ? 'merge'
      : commits[i].lane === commits[i - 1].lane
        ? 'straight'
        : 'branch';
    edges.push({
      fromOid: commits[i].oid,
      toOid: commits[i - 1].oid,
      fromLane: commits[i].lane,
      toLane: commits[i - 1].lane,
      kind: kind,
    });
  }

  var recent = [
    {
      id: 1,
      path: 'D:/work/forgedesk',
      name: 'forgedesk',
      defaultBranch: 'main',
      lastOpenedAt: 1791400000,
      createdAt: 1790000000,
      isOpen: true,
    },
    {
      id: 2,
      path: 'D:/work/site',
      name: 'site',
      defaultBranch: 'main',
      lastOpenedAt: 1791300000,
      createdAt: 1789000000,
      isOpen: false,
    },
    {
      id: 3,
      path: 'D:/work/design-tokens',
      name: 'design-tokens',
      defaultBranch: 'main',
      lastOpenedAt: 1791200000,
      createdAt: 1788000000,
      isOpen: false,
    },
  ];

  function entry(path, indexStatus, worktreeStatus) {
    return {
      kind: 'ordinary',
      path: path,
      indexStatus: indexStatus,
      worktreeStatus: worktreeStatus,
      isBinary: false,
      isLfs: false,
      isSubmodule: false,
      sizeBytes: 4820,
    };
  }

  function statusReport() {
    return {
      branch: {
        oid: commits[0].oid,
        head: 'main',
        detached: false,
        upstream: 'origin/main',
        ahead: 2,
        behind: 0,
      },
      operation: 'none',
      staged: [
        entry('src/features/commit/CommitPanel.tsx', 'M', '.'),
        entry('src/lib/ipc/commit.ts', 'M', '.'),
      ],
      unstaged: [
        entry('site/assets.css', '.', 'M'),
        entry('docs/manual/02-history-branches-sync.md', '.', 'M'),
      ],
      untracked: [entry('site/images/history.webp', '?', '?')],
      conflicted: [],
      ignored: [],
      ignoredCount: 14,
    };
  }

  window.__TAURI_INTERNALS__ = {
    transformCallback: function (callback) {
      listeners.push(callback);
      return listeners.length;
    },
    unregisterListener: function () {},
    invoke: function (command, args) {
      if (command === 'repo_open') {
        // 走真实入口打开仓库：顶栏的"当前仓库"因此有值，截图里不会出现
        // "未打开仓库 + 一堆仓库内容"这种自相矛盾的状态
        return Promise.resolve({
          recordId: 1,
          repository: {
            workdir: 'D:/work/forgedesk',
            gitDir: 'D:/work/forgedesk/.git',
            isBare: false,
            isEmpty: false,
            head: 'main',
            detached: false,
          },
          audit: { findings: [], hasDanger: false, maxSeverity: null },
          gitVersion: '2.50.0',
          gitVersionSupported: true,
          needsGitUpgrade: false,
        });
      }
      if (command === 'git_commit_detail') {
        var oid = String((args && args.oid) || commits[0].oid);
        var index = 0;
        for (var ci = 0; ci < commits.length; ci++) {
          if (commits[ci].oid === oid) index = ci;
        }
        var commit = fullCommits[index];
        return Promise.resolve({
          meta: {
            oid: oid,
            shortOid: oid.slice(-7),
            parents: index === 0 ? [] : [commits[index - 1].oid],
            author: commit.author,
            committer: commit.committer,
            subject: commit.subject,
            body: null,
            signature: 'good',
          },
          refs: commit.refs,
          stats: { filesChanged: 2, insertions: 41, deletions: 7 },
          files: [
            {
              path: 'src/features/history/GraphCanvas.tsx',
              oldPath: null,
              kind: 'modified',
              binary: false,
              additions: 32,
              deletions: 6,
              truncated: false,
            },
            {
              path: 'src/features/history/commitMeta.ts',
              oldPath: null,
              kind: 'modified',
              binary: false,
              additions: 9,
              deletions: 1,
              truncated: false,
            },
          ],
          isMerge: false,
          isHead: index === 0,
          isPushed: index === 0,
          webUrl: null,
        });
      }
      if (command === 'repo_recent_list') return Promise.resolve(recent);
      if (command === 'workspace_status') return Promise.resolve(statusReport());
      if (command === 'git_log_page') {
        return Promise.resolve({
          commits: fullCommits,
          layout: { rows: rows, edges: edges, laneCount: 3 },
          nextCursor: null,
        });
      }
      if (command === 'git_log_authors') {
        return Promise.resolve([
          { name: 'EMBER', email: 'ember@forgedesk.invalid', commitCount: commits.length },
        ]);
      }
      if (command === 'git_branch_list') {
        return Promise.resolve([
          {
            name: 'main',
            isRemote: false,
            isHead: true,
            target: commits[0].oid,
            upstream: 'origin/main',
            ahead: 2,
            behind: 0,
            upstreamGone: false,
          },
          {
            name: 'feature/rebase-panel',
            isRemote: false,
            isHead: false,
            target: commits[10].oid,
            upstream: null,
            ahead: null,
            behind: null,
            upstreamGone: false,
          },
          {
            name: 'origin/main',
            isRemote: true,
            isHead: false,
            target: commits[0].oid,
            upstream: null,
            ahead: null,
            behind: null,
            upstreamGone: false,
          },
        ]);
      }
      if (command === 'git_tag_list') {
        return Promise.resolve([
          {
            name: 'v1.0.0',
            target: commits[2].oid,
            commit: commits[2].oid,
            annotated: true,
            message: 'ForgeDesk 1.0.0',
            createdAt: 1790000000,
          },
        ]);
      }
      if (command === 'commit_message_hint') {
        return Promise.resolve({
          recentMessages: ['feat: 提交面板支持逐块暂存'],
          template: 'feat: ',
          branchStyle: 'main',
        });
      }
      if (command === 'commit_hooks_list') {
        return Promise.resolve([{ name: 'pre-commit', executable: true, commitHook: true }]);
      }
      if (command === 'commit_amend_context') {
        return Promise.resolve({
          subject: commits[0].subject,
          body: null,
          headOid: commits[0].oid,
          pushed: true,
          pushedRefs: ['origin/main'],
        });
      }
      if (command === 'update_check') return Promise.resolve({ configured: false, update: null });
      if (command === 'app_version')
        return Promise.resolve({ version: '1.0.0', gitDescribe: null });
      if (command === 'settings_all') return Promise.resolve({});
      if (command === 'settings_get') return Promise.resolve(null);
      if (command === 'logs_tail') return Promise.resolve([]);
      if (command === 'plugin:event|unlisten') return Promise.resolve(null);
      void args;
      return Promise.resolve(null);
    },
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();
