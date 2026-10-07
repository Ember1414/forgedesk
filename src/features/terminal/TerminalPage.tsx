/**
 * 终端页面（T5.2）：多标签 xterm 终端。
 *
 * # 会话与标签的生命周期
 *
 * 后端会话（进程）由 `term_create` 创建，标签（投影）由 terminalStore 保存；
 * 两者在"+"菜单的选择动作里一一对应。标签关闭必须同时调 `term_close`——
 * 只关标签不杀进程会留下一个没人能寻址的孤儿 shell。
 *
 * # 仓库边界（任务书：不允许静默切错目录）
 *
 * 每个会话创建时的 cwd 固定为**创建它的那个仓库**的根（后端还做了
 * 逃逸校验）；本页只渲染当前仓库的标签——切到别的仓库看到的是那个仓库
 * 自己的终端，绝不会把 A 仓库的会话显示在 B 仓库下面。
 *
 * 会话持久化：应用重启后不恢复会话（shell 进程已死，假恢复比不恢复更糟）；
 * "恢复上次命令"由右键菜单的"插入上次命令"承担（manager 的命令历史）。
 */
import { useCallback, useEffect, useState } from 'react';

import { useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';

import { Plus } from 'lucide-react';

import { applyThemeToAll, ensureTerminalListeners } from '@/features/terminal/manager';
import { TerminalView } from '@/features/terminal/TerminalView';
import { termClose, termCreate, termShellList } from '@/lib/ipc';
import type { TermShell } from '@/lib/ipc';
import {
  DEFAULT_TERMINAL_FONT_SIZE,
  DEFAULT_TERMINAL_LINE_HEIGHT,
  TERMINAL_FONT_SIZE_KEY,
  TERMINAL_LINE_HEIGHT_KEY,
  useSettingsStore,
} from '@/stores/settingsStore';
import { useTerminalStore } from '@/stores/terminalStore';
import { Button } from '@/ui/components/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/ui/components/dropdown-menu';
import { Input } from '@/ui/components/input';

/** shell 选项的 i18n key（id → `terminal.shells.<id>`；未知 id 原样展示）。 */
function shellLabelKey(id: string): string {
  return `terminal.shells.${id}`;
}

export function TerminalPage() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const tabs = useTerminalStore((state) => state.tabs);
  const activeTermId = useTerminalStore((state) => state.activeTermId);
  const addTab = useTerminalStore((state) => state.addTab);
  const removeTab = useTerminalStore((state) => state.removeTab);
  const setActive = useTerminalStore((state) => state.setActive);
  const renameTab = useTerminalStore((state) => state.renameTab);
  const moveTab = useTerminalStore((state) => state.moveTab);

  const [shells, setShells] = useState<readonly TermShell[]>([]);
  const [creating, setCreating] = useState(false);
  const [renamingTermId, setRenamingTermId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState('');
  const [dragFrom, setDragFrom] = useState<number | null>(null);

  // 字体设置（settings store 是异步加载的：读不到就用默认值，加载完成后自然跟上）
  const fontSize = useSettingsStore((state) =>
    state.getJson<number>(TERMINAL_FONT_SIZE_KEY, DEFAULT_TERMINAL_FONT_SIZE),
  );
  const lineHeight = useSettingsStore((state) =>
    state.getJson<number>(TERMINAL_LINE_HEIGHT_KEY, DEFAULT_TERMINAL_LINE_HEIGHT),
  );

  const repoTabs = tabs.filter((tab) => tab.repoId === repoId);

  // 全局监听 + 主题跟随（页面级一次即可；store 与 manager 都是模块级单例，
  // 离开页面后事件仍被缓冲，回来时排干——见 manager.ts）
  useEffect(() => {
    ensureTerminalListeners();
    // 同时监听 data-theme 与 style：自定义主题（T6.6）不走 data-theme，
    // 而是把 --fd-* 写成 <html> 的内联样式——只盯属性会漏掉"激活同外观的
    // 自定义主题"（终端颜色不跟变的现场就在这）
    const observer = new MutationObserver(applyThemeToAll);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-theme', 'style'],
    });
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    let cancelled = false;
    void termShellList()
      .then((options) => {
        // 边界归一化：mock / 旧后端可能返回 null（settings 的同款教训）
        if (!cancelled) {
          setShells(options ?? [{ id: 'default', program: '' }]);
        }
      })
      .catch(() => {
        // 探测失败："+"菜单退化为仅默认项；term_create 内部同样有兜底
        if (!cancelled) {
          setShells([{ id: 'default', program: '' }]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const createSession = useCallback(
    async (shellId: string) => {
      if (!Number.isFinite(repoId) || creating) {
        return;
      }
      setCreating(true);
      try {
        ensureTerminalListeners();
        const created = await termCreate({
          repoId,
          ...(shellId === 'default' ? {} : { shell: shellId }),
          cols: 80,
          rows: 24,
        });
        const label = shells.find((shell) => shell.id === shellId)?.id ?? shellId;
        addTab({
          termId: created.termId,
          repoId,
          shellId,
          title: t(shellLabelKey(label), { defaultValue: label }),
          renamed: false,
          exited: false,
          exitCode: null,
        });
      } finally {
        setCreating(false);
      }
    },
    [addTab, creating, repoId, shells, t],
  );

  const closeTab = useCallback(
    (termId: string) => {
      removeTab(termId);
      void termClose(termId).catch(() => {
        // 会话可能已自己退出：标签已经移除，后端残留由退出线程自行清理
      });
    },
    [removeTab],
  );

  // 拖拽排序：draggable 标签 + 落点交换（与 rebase 面板同一套原生 DnD 约定）
  const handleDrop = useCallback(
    (to: number) => {
      if (dragFrom === null || dragFrom === to) {
        setDragFrom(null);
        return;
      }
      // moveTab 作用于全局 tabs 数组；本页列表是过滤后的子集，需要换算全局下标
      const fromTab = repoTabs[dragFrom];
      const toTab = repoTabs[to];
      if (fromTab && toTab) {
        const globalFrom = tabs.indexOf(fromTab);
        const globalTo = tabs.indexOf(toTab);
        if (globalFrom >= 0 && globalTo >= 0) {
          moveTab(globalFrom, globalTo);
        }
      }
      setDragFrom(null);
    },
    [dragFrom, moveTab, repoTabs, tabs],
  );

  return (
    <div className="flex h-full min-h-0 flex-col p-3">
      {/* 页面标题：与其他页面同一约定（routes.test / shell.spec 按可见 h1 断言）；
          标题在任何分支都渲染（含未打开仓库的提示态） */}
      <h1 className="text-20 font-semibold tracking-tight">{t('pages.repoTerminal.title')}</h1>
      {!Number.isFinite(repoId) ? (
        <p className="text-fg-muted p-4 text-13">{t('terminal.notOpen')}</p>
      ) : (
        <>
          <div
            className="border-line flex items-center gap-1 border-b pb-1.5"
            role="tablist"
            aria-label={t('terminal.tablistLabel')}
          >
            {repoTabs.map((tab, index) => {
              const isActive = tab.termId === activeTermId;
              return (
                <div
                  key={tab.termId}
                  role="presentation"
                  draggable
                  onDragStart={() => setDragFrom(index)}
                  onDragOver={(event) => event.preventDefault()}
                  onDrop={() => handleDrop(index)}
                  className={[
                    'fd-transition group flex items-center gap-1 rounded-md px-2 py-1 text-13',
                    isActive
                      ? 'bg-surface-raised text-fg'
                      : 'text-fg-muted hover:bg-surface-sunken hover:text-fg',
                    dragFrom === index ? 'opacity-50' : '',
                  ].join(' ')}
                >
                  {renamingTermId === tab.termId ? (
                    <Input
                      srLabel={t('terminal.rename.label')}
                      value={renameDraft}
                      autoFocus
                      onChange={(event) => setRenameDraft(event.target.value)}
                      onBlur={() => {
                        if (renameDraft.trim() !== '') {
                          renameTab(tab.termId, renameDraft.trim());
                        }
                        setRenamingTermId(null);
                      }}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter') {
                          (event.target as HTMLInputElement).blur();
                        }
                        if (event.key === 'Escape') {
                          setRenamingTermId(null);
                        }
                      }}
                      className="h-6 w-32 text-12"
                    />
                  ) : (
                    <button
                      type="button"
                      role="tab"
                      aria-selected={isActive}
                      onDoubleClick={() => {
                        setRenameDraft(tab.title);
                        setRenamingTermId(tab.termId);
                      }}
                      onClick={() => setActive(tab.termId)}
                      className="max-w-48 truncate"
                      title={tab.title}
                    >
                      {tab.title}
                      {tab.exited ? ' ⏹' : ''}
                    </button>
                  )}
                  <button
                    type="button"
                    aria-label={t('terminal.closeTab', { title: tab.title })}
                    className="text-fg-muted hover:text-fg hover:bg-surface-sunken rounded p-0.5 opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                    onClick={(event) => {
                      event.stopPropagation();
                      closeTab(tab.termId);
                    }}
                  >
                    <svg aria-hidden="true" viewBox="0 0 12 12" className="size-2.5 fill-current">
                      <path d="M6 4.94 10.3.64l1.06 1.06L7.06 6l4.3 4.3-1.06 1.06L6 7.06l-4.3 4.3L.64 10.3 4.94 6 .64 1.7 1.7.64 6 4.94Z" />
                    </svg>
                  </button>
                </div>
              );
            })}

            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="sm"
                  className="px-1.5"
                  disabled={creating}
                  aria-label={t('terminal.newTab')}
                >
                  <Plus aria-hidden="true" className="size-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start">
                <DropdownMenuLabel>{t('terminal.shellPickerTitle')}</DropdownMenuLabel>
                <DropdownMenuSeparator />
                {shells.map((shell) => (
                  <DropdownMenuItem key={shell.id} onSelect={() => void createSession(shell.id)}>
                    {shell.id === 'default'
                      ? t(shellLabelKey('default'))
                      : t(shellLabelKey(shell.id), { defaultValue: shell.id })}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>

          <div className="relative flex min-h-0 flex-1 pt-2">
            {repoTabs.length === 0 ? (
              <div className="text-fg-muted flex flex-1 items-center justify-center">
                <p className="text-13">{t('terminal.empty')}</p>
              </div>
            ) : (
              repoTabs.map((tab) => (
                <TerminalView
                  key={tab.termId}
                  tab={tab}
                  active={tab.termId === activeTermId}
                  fontSize={fontSize}
                  lineHeight={lineHeight}
                  onRestart={() => void createSession(tab.shellId)}
                  onClose={() => closeTab(tab.termId)}
                />
              ))
            )}
          </div>
        </>
      )}
    </div>
  );
}
