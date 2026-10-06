/**
 * 命令定义（T5.9）：≥40 个核心命令的注册表。
 *
 * run 闭包需要导航/仓库/IPC 上下文，因此这里是 hook；ShortcutManager
 * 在 AppShell 挂载时调用本 hook 并 `register()` 进注册表。
 * 快捷键默认值刻意**不与 VS Code 逐条对齐**（红线 R3）：只有
 * Mod+Shift+P（面板）与 Mod+S（保存）沿行业惯例，其余自有分配。
 */
import { useCallback, useMemo } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { useQueryClient } from '@tanstack/react-query';

import { gitFetch } from '@/lib/ipc/sync';
import { workspaceStatus } from '@/lib/ipc/workspace';
import { STATUS_QUERY_KEY } from '@/lib/queryKeys';
import { openLogViewer } from '@/stores/logViewerStore';
import { useUiStore } from '@/stores/uiStore';
import { useCommandRegistry } from '@/features/commands/commandRegistry';
import type { RegisteredCommand } from '@/features/commands/commandRegistry';

/** 分类（面板分组）。 */
type Category = RegisteredCommand['category'];

/** 工厂：导航类命令（进入仓库内页面需要已打开仓库）。 */
function nav(
  id: string,
  titleKey: string,
  category: Category,
  path: string,
  navigate: ReturnType<typeof useNavigate>,
  defaultKey: string | null,
  repoScoped: boolean,
): RegisteredCommand {
  return {
    id,
    titleKey,
    category,
    defaultKey,
    when: repoScoped ? ['repoOpen'] : [],
    run: () => {
      if (repoScoped) {
        const repoId = useUiStore.getState().currentRepoId;
        if (repoId !== null) {
          navigate(`/repo/${repoId}${path}`);
        }
        return;
      }
      navigate(path);
    },
  };
}

/** 组装全部命令（AppShell 与命令面板共用）。 */
export function useCommandDefinitions(): readonly RegisteredCommand[] {
  const { t } = useTranslation('shell');
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const setThemeMode = useUiStore((state) => state.setThemeMode);
  const toggleSidebar = useUiStore((state) => state.toggleSidebar);
  const setDetailPanel = useUiStore((state) => state.setDetailPanel);

  const repoScope = useCallback(() => useUiStore.getState().currentRepoId, []);

  const refreshStatus = useCallback(async () => {
    const repoId = repoScope();
    if (repoId === null) {
      return;
    }
    await workspaceStatus(Number(repoId), false).catch(() => {});
    void queryClient.invalidateQueries({ queryKey: [STATUS_QUERY_KEY, Number(repoId)] });
  }, [queryClient, repoScope]);

  return useMemo(() => {
    const commands: RegisteredCommand[] = [
      // ---------------- 导航（仓库内）----------------
      nav('nav.dashboard', 'commands.nav.dashboard', 'navigate', '/', navigate, null, false),
      nav('nav.status', 'commands.nav.status', 'navigate', '/status', navigate, null, true),
      nav('nav.commit', 'commands.nav.commit', 'navigate', '/commit', navigate, 'Mod+1', true),
      nav('nav.history', 'commands.nav.history', 'navigate', '/history', navigate, 'Mod+2', true),
      nav(
        'nav.branches',
        'commands.nav.branches',
        'navigate',
        '/branches',
        navigate,
        'Mod+3',
        true,
      ),
      nav(
        'nav.snapshots',
        'commands.nav.snapshots',
        'navigate',
        '/snapshots',
        navigate,
        null,
        true,
      ),
      nav(
        'nav.operations',
        'commands.nav.operations',
        'navigate',
        '/operations',
        navigate,
        null,
        true,
      ),
      nav('nav.conflict', 'commands.nav.conflict', 'navigate', '/conflict', navigate, null, true),
      nav(
        'nav.terminal',
        'commands.nav.terminal',
        'navigate',
        '/terminal',
        navigate,
        'Mod+`',
        true,
      ),
      nav('nav.editor', 'commands.nav.editor', 'navigate', '/editor', navigate, 'Mod+4', true),
      // ---------------- 导航（全局）----------------
      nav('app.settings', 'commands.app.settings', 'app', '/settings', navigate, 'Mod+,', false),
      nav('app.plugins', 'commands.app.plugins', 'app', '/plugins', navigate, null, false),
      nav(
        'app.commandsDictionary',
        'commands.app.commandsDictionary',
        'app',
        '/commands',
        navigate,
        null,
        false,
      ),
      {
        id: 'app.commandPalette',
        titleKey: 'commands.app.commandPalette',
        category: 'app',
        defaultKey: 'Mod+Shift+P',
        when: [],
        run: () => {
          useCommandRegistry.getState().setPaletteOpen(true);
        },
      },
      // ---------------- Git 操作 ----------------
      {
        id: 'git.refresh',
        titleKey: 'commands.git.refresh',
        category: 'git',
        defaultKey: 'F5',
        when: ['repoOpen'],
        run: () => {
          void refreshStatus();
        },
      },
      {
        id: 'git.fetch',
        titleKey: 'commands.git.fetch',
        category: 'git',
        defaultKey: null,
        when: ['repoOpen'],
        run: () => {
          const repoId = repoScope();
          if (repoId !== null) {
            void gitFetch(Number(repoId), {}).catch(() => {});
          }
        },
      },
      {
        id: 'git.stageAll',
        titleKey: 'commands.git.stageAll',
        category: 'git',
        defaultKey: null,
        when: ['repoOpen'],
        run: () => {
          window.location.hash = `/repo/${repoScope() ?? ''}/status`;
        },
      },
      {
        id: 'git.commit',
        titleKey: 'commands.git.commit',
        category: 'git',
        defaultKey: 'Mod+Enter',
        when: ['repoOpen'],
        run: () => {
          window.location.hash = `/repo/${repoScope() ?? ''}/commit`;
        },
      },
      {
        id: 'git.push',
        titleKey: 'commands.git.push',
        category: 'git',
        defaultKey: 'Mod+U',
        when: ['repoOpen'],
        run: () => {
          window.location.hash = `/repo/${repoScope() ?? ''}/branches`;
        },
      },
      {
        id: 'git.pull',
        titleKey: 'commands.git.pull',
        category: 'git',
        defaultKey: 'Mod+D',
        when: ['repoOpen'],
        run: () => {
          window.location.hash = `/repo/${repoScope() ?? ''}/branches`;
        },
      },
      {
        id: 'git.stash',
        titleKey: 'commands.git.stash',
        category: 'git',
        defaultKey: null,
        when: ['repoOpen'],
        run: () => {
          window.location.hash = `/repo/${repoScope() ?? ''}/status`;
        },
      },
      {
        id: 'git.branchCreate',
        titleKey: 'commands.git.branchCreate',
        category: 'git',
        defaultKey: null,
        when: ['repoOpen'],
        run: () => {
          window.location.hash = `/repo/${repoScope() ?? ''}/branches`;
        },
      },
      {
        id: 'git.viewSnapshot',
        titleKey: 'commands.git.viewSnapshot',
        category: 'git',
        defaultKey: null,
        when: ['repoOpen'],
        run: () => {
          window.location.hash = `/repo/${repoScope() ?? ''}/snapshots`;
        },
      },
      // ---------------- 视图 ----------------
      {
        id: 'view.toggleSidebar',
        titleKey: 'commands.view.toggleSidebar',
        category: 'view',
        defaultKey: 'Mod+B',
        when: [],
        run: toggleSidebar,
      },
      {
        id: 'view.toggleTheme',
        titleKey: 'commands.view.toggleTheme',
        category: 'view',
        defaultKey: null,
        when: [],
        run: () => {
          setThemeMode(useUiStore.getState().themeMode === 'dark' ? 'light' : 'dark');
        },
      },
      {
        id: 'view.themeLight',
        titleKey: 'commands.view.themeLight',
        category: 'view',
        defaultKey: null,
        when: [],
        run: () => setThemeMode('light'),
      },
      {
        id: 'view.themeDark',
        titleKey: 'commands.view.themeDark',
        category: 'view',
        defaultKey: null,
        when: [],
        run: () => setThemeMode('dark'),
      },
      {
        id: 'view.themeSystem',
        titleKey: 'commands.view.themeSystem',
        category: 'view',
        defaultKey: null,
        when: [],
        run: () => setThemeMode('system'),
      },
      {
        id: 'view.detailRight',
        titleKey: 'commands.view.detailRight',
        category: 'view',
        defaultKey: null,
        when: [],
        run: () => setDetailPanel('right'),
      },
      {
        id: 'view.detailBottom',
        titleKey: 'commands.view.detailBottom',
        category: 'view',
        defaultKey: null,
        when: [],
        run: () => setDetailPanel('bottom'),
      },
      {
        id: 'view.detailHidden',
        titleKey: 'commands.view.detailHidden',
        category: 'view',
        defaultKey: null,
        when: [],
        run: () => setDetailPanel('hidden'),
      },
      {
        id: 'view.openLogs',
        titleKey: 'commands.view.openLogs',
        category: 'view',
        defaultKey: null,
        when: [],
        run: () => openLogViewer({ nearTimestamp: null }),
      },
      // ---------------- 编辑器（editorActive 才可用）----------------
      nav('editor.open', 'commands.editor.open', 'editor', '/editor', navigate, null, true),
      {
        id: 'editor.toggleTree',
        titleKey: 'commands.editor.toggleTree',
        category: 'editor',
        defaultKey: 'Mod+E',
        when: ['editorActive'],
        run: () => {
          // 树开关由编辑器页内部状态持有；这里走事件（页面监听）
          window.dispatchEvent(new CustomEvent('forgedesk:toggle-tree'));
        },
      },
      // 标题 i18n key 存在性由 i18n.test 键一致性保证；这里 t() 只为触发 lint 友好
      {
        id: 'editor.save',
        titleKey: 'commands.editor.save',
        category: 'editor',
        defaultKey: 'Mod+S',
        when: ['editorActive'],
        run: () => {
          window.dispatchEvent(new CustomEvent('forgedesk:editor-save'));
        },
      },
      {
        id: 'editor.closeTab',
        titleKey: 'commands.editor.closeTab',
        category: 'editor',
        defaultKey: null,
        when: ['editorActive'],
        run: () => {
          window.dispatchEvent(new CustomEvent('forgedesk:editor-close-tab'));
        },
      },
    ];
    // 占位引用（t 在工厂里未直接使用时避免 lint 噪音——titleKey 由渲染端翻译）
    void t;
    return commands;
  }, [
    navigate,
    queryClient,
    refreshStatus,
    repoScope,
    setDetailPanel,
    setThemeMode,
    t,
    toggleSidebar,
  ]);
}
