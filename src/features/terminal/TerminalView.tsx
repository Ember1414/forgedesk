/**
 * 单个终端的视图（T5.2）：xterm 生命周期 + 复制粘贴 + 搜索 + 链接。
 *
 * # 实例归属
 *
 * `Terminal` 实例住在 `manager.ts` 的模块级注册表里（理由见其文件头）；
 * 本组件只负责"把实例打开到这个 div + 随尺寸重排 + 键盘/剪贴板接线"，
 * 卸载时注销并销毁实例（再次进入页面时会重建，离屏期间的输出由
 * manager 的 pending 缓冲保存）。
 *
 * # 复制粘贴的键位约定（终端惯例）
 *
 * - Ctrl/Cmd+C：**有选区 = 复制**，无选区 = 原样放行（xterm 发送 ETX → SIGINT）；
 * - Ctrl/Cmd+V：粘贴；
 * - Ctrl/Cmd+F：搜索（接管浏览器查找——桌面应用里浏览器查找无意义）。
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useQueryClient } from '@tanstack/react-query';
import { CaseSensitive, ChevronDown, ChevronUp, Regex, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import { Terminal } from '@xterm/xterm';

import {
  applyFontToAll,
  findTerminalInstance,
  getLastCommand,
  openExternalUrl,
  recordCommand,
  registerTerminal,
  unregisterTerminal,
} from '@/features/terminal/manager';
import { LineTracker, isGitCommand } from '@/features/terminal/gitInputRefresh';
import { createRepoChangeInvalidator } from '@/lib/repoChanged';
import { termResize, termWrite } from '@/lib/ipc';
import type { TerminalTab } from '@/stores/terminalStore';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from '@/ui/components/context-menu';
import { IconButton } from '@/ui/components/icon-button';
import { Input } from '@/ui/components/input';

import { deriveXtermTheme } from './xtermTheme';

export interface TerminalViewProps {
  readonly tab: TerminalTab;
  /** 是否为激活标签（隐藏的标签保持挂载以保住滚动缓冲，但不抢焦点）。 */
  readonly active: boolean;
  readonly fontSize: number;
  readonly lineHeight: number;
  /** 重新开启一个同 shell 的新会话（退出横幅的"重新开始"）。 */
  readonly onRestart: () => void;
  /** 关闭本标签（后端 term_close 由父层统一处理）。 */
  readonly onClose: () => void;
}

export function TerminalView({
  tab,
  active,
  fontSize,
  lineHeight,
  onRestart,
  onClose,
}: TerminalViewProps) {
  const { t } = useTranslation('shell');
  const containerRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [showSearch, setShowSearch] = useState(false);
  const [searchState, setSearchState] = useState({ query: '', regex: false, caseSensitive: false });
  const queryClient = useQueryClient();

  // 搜索框打开时聚焦输入框
  useEffect(() => {
    if (showSearch) {
      searchInputRef.current?.focus();
    }
  }, [showSearch]);

  // xterm 生命周期：挂载建立，卸载销毁（StrictMode 双挂载由 manager 的
  // pending 缓冲保证输出不丢，见 manager.ts）
  useEffect(() => {
    const container = containerRef.current;
    if (!container) {
      return;
    }

    const term = new Terminal({
      fontFamily: 'var(--fd-font-mono, Consolas, monospace)',
      fontSize,
      lineHeight,
      cursorBlink: true,
      scrollback: 5000,
      theme: deriveXtermTheme(),
    });
    const fit = new FitAddon();
    const search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    // 链接：http(s) 交系统默认浏览器（后端二次校验协议白名单）
    term.loadAddon(new WebLinksAddon((_event, uri) => openExternalUrl(uri)));

    term.open(container);

    // WebGL 渲染失败自动回落 canvas（软件渲染 / GPU 被禁）：
    // xterm 的默认渲染器就是 canvas，"回落"即不保留 WebGL 插件。
    // 渲染器选择在 stderr 级别可观测（warn 是 lint 允许的最低级别）。
    try {
      const webgl = new WebglAddon();
      term.loadAddon(webgl);
      webgl.onContextLoss(() => {
        webgl.dispose();
        console.warn('ForgeDesk terminal: WebGL context lost, fell back to canvas');
      });
    } catch (error) {
      console.warn('ForgeDesk terminal: WebGL renderer unavailable, using canvas', error);
    }

    const doFit = () => {
      if (container.clientWidth <= 0 || container.clientHeight <= 0) {
        return; // 隐藏容器：0 尺寸下 fit 会抛，等可见后由 ResizeObserver 再触发
      }
      try {
        fit.fit();
        void termResize(tab.termId, term.cols, term.rows).catch(() => {
          // 会话刚好退出：resize 失败无害，退出横幅马上就会出现
        });
      } catch {
        // fit 在极端尺寸下可能抛：忽略这一次，下一次尺寸变化会重试
      }
    };

    const observer = new ResizeObserver(doFit);
    observer.observe(container);
    doFit();

    // git 命令后的状态刷新：命令结果需要几百毫秒到几十秒，
    // 两级延迟（800ms 覆盖 status 类快速命令；2.5s 兜底 clone/fetch）
    const invalidator = createRepoChangeInvalidator(queryClient);
    const scheduledRefreshes = new Set<ReturnType<typeof setTimeout>>();
    const scheduleRefresh = () => {
      for (const delay of [800, 2500]) {
        const timer = setTimeout(() => {
          scheduledRefreshes.delete(timer);
          invalidator.invalidate(tab.repoId, 'refs');
        }, delay);
        scheduledRefreshes.add(timer);
      }
    };

    // 键入：写后端 + 行跟踪（命令历史 / git 命令刷新）
    const tracker = new LineTracker();
    const dataSubscription = term.onData((data) => {
      void termWrite(tab.termId, new TextEncoder().encode(data));
      for (const line of tracker.feed(data)) {
        if (line.trim() !== '') {
          recordCommand(tab.repoId, line);
        }
        if (isGitCommand(line)) {
          scheduleRefresh();
        }
      }
    });

    // 键盘约定：见文件头
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown') {
        return true;
      }
      const mod = event.ctrlKey || event.metaKey;
      if (mod && event.key === 'c' && term.hasSelection()) {
        void navigator.clipboard.writeText(term.getSelection());
        return false;
      }
      if (mod && event.key === 'v') {
        void navigator.clipboard.readText().then((text) => {
          if (text !== '') {
            term.paste(text);
          }
        });
        return false;
      }
      if (mod && event.key === 'f') {
        setShowSearch(true);
        return false;
      }
      return true;
    });

    registerTerminal(tab.termId, { term, fit, search });

    if (active) {
      term.focus();
    }

    return () => {
      for (const timer of scheduledRefreshes) {
        clearTimeout(timer);
      }
      invalidator.dispose();
      observer.disconnect();
      dataSubscription.dispose();
      unregisterTerminal(tab.termId);
      term.dispose();
    };
    // 建立一次；fontSize/active 的变化走下面的 options 更新路径，
    // 不重建实例（重建会丢滚动缓冲）
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.termId, tab.repoId, queryClient]);

  // 字号 / 行高变化：原地应用 + 触发重排
  useEffect(() => {
    applyFontToAll(fontSize, lineHeight);
    containerRef.current?.dispatchEvent(new Event('resize'));
  }, [fontSize, lineHeight]);

  // 激活状态变化：可见时重排并聚焦
  useEffect(() => {
    if (!active) {
      return;
    }
    const frame = requestAnimationFrame(() => {
      containerRef.current?.dispatchEvent(new Event('resize'));
      findTerminalInstance(tab.termId)?.term.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [active, tab.termId]);

  const runSearch = useCallback(
    (direction: 'next' | 'prev') => {
      const managed = findTerminalInstance(tab.termId);
      if (!managed || searchState.query === '') {
        return;
      }
      const options = {
        regex: searchState.regex,
        caseSensitive: searchState.caseSensitive,
      };
      if (direction === 'next') {
        managed.search.findNext(searchState.query, options);
      } else {
        managed.search.findPrevious(searchState.query, options);
      }
    },
    [searchState, tab.termId],
  );

  const copySelection = useCallback(() => {
    const managed = findTerminalInstance(tab.termId);
    if (managed?.term.hasSelection()) {
      void navigator.clipboard.writeText(managed.term.getSelection());
    }
  }, [tab.termId]);

  const pasteClipboard = useCallback(() => {
    const managed = findTerminalInstance(tab.termId);
    if (!managed) {
      return;
    }
    void navigator.clipboard.readText().then((text) => {
      if (text !== '') {
        managed.term.paste(text);
      }
    });
  }, [tab.termId]);

  const insertLastCommand = useCallback(() => {
    const last = getLastCommand(tab.repoId);
    const managed = findTerminalInstance(tab.termId);
    if (last !== null && managed) {
      // paste 走 onData 路径：命令会像手敲一样进入行缓冲（进而进历史）
      managed.term.paste(last);
    }
  }, [tab.repoId, tab.termId]);

  return (
    <div
      className={active ? 'relative min-h-0 min-w-0 flex-1' : 'hidden'}
      data-term-view={tab.termId}
    >
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div ref={containerRef} className="h-full w-full" data-testid="terminal-surface" />
        </ContextMenuTrigger>
        <ContextMenuContent>
          <ContextMenuItem onSelect={copySelection}>{t('terminal.menu.copy')}</ContextMenuItem>
          <ContextMenuItem onSelect={pasteClipboard}>{t('terminal.menu.paste')}</ContextMenuItem>
          <ContextMenuItem onSelect={() => findTerminalInstance(tab.termId)?.term.selectAll()}>
            {t('terminal.menu.selectAll')}
          </ContextMenuItem>
          <ContextMenuItem onSelect={() => findTerminalInstance(tab.termId)?.term.clear()}>
            {t('terminal.menu.clear')}
          </ContextMenuItem>
          <ContextMenuItem onSelect={() => setShowSearch(true)}>
            {t('terminal.menu.search')}
          </ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem
            disabled={getLastCommand(tab.repoId) === null}
            onSelect={insertLastCommand}
          >
            {t('terminal.menu.insertLastCommand')}
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>

      {tab.exited ? (
        <div className="bg-scrim absolute inset-0 z-10 flex items-center justify-center">
          <div className="border-line bg-surface-raised rounded-lg border p-4 shadow-lg">
            <p className="text-14">
              {t('terminal.exitBanner.title', { code: tab.exitCode ?? '—' })}
            </p>
            <div className="mt-3 flex gap-2">
              <button
                type="button"
                className="bg-brand text-brand-fg hover:bg-brand-hover fd-transition rounded-md px-3 py-1.5 text-13"
                onClick={onRestart}
              >
                {t('terminal.exitBanner.restart')}
              </button>
              <button
                type="button"
                className="border-line bg-surface text-fg hover:bg-surface-sunken fd-transition rounded-md border px-3 py-1.5 text-13"
                onClick={onClose}
              >
                {t('terminal.exitBanner.close')}
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {showSearch ? (
        <div className="border-line bg-surface-raised absolute end-3 top-3 z-20 flex items-center gap-1 rounded-md border p-1.5 shadow-lg">
          <Input
            ref={searchInputRef}
            srLabel={t('terminal.search.placeholder')}
            value={searchState.query}
            onChange={(event) =>
              setSearchState((state) => ({ ...state, query: event.target.value }))
            }
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                runSearch(event.shiftKey ? 'prev' : 'next');
              }
              if (event.key === 'Escape') {
                setShowSearch(false);
              }
            }}
            className="h-7 w-44 text-12"
          />
          <IconButton
            label={t('terminal.search.caseSensitive')}
            tooltip={t('terminal.search.caseSensitive')}
            size="sm"
            aria-pressed={searchState.caseSensitive}
            onClick={() =>
              setSearchState((state) => ({ ...state, caseSensitive: !state.caseSensitive }))
            }
          >
            <CaseSensitive aria-hidden="true" className="size-3.5" />
          </IconButton>
          <IconButton
            label={t('terminal.search.regex')}
            tooltip={t('terminal.search.regex')}
            size="sm"
            aria-pressed={searchState.regex}
            onClick={() => setSearchState((state) => ({ ...state, regex: !state.regex }))}
          >
            <Regex aria-hidden="true" className="size-3.5" />
          </IconButton>
          <IconButton
            label={t('terminal.search.previous')}
            tooltip={t('terminal.search.previous')}
            size="sm"
            onClick={() => runSearch('prev')}
          >
            <ChevronUp aria-hidden="true" className="size-3.5" />
          </IconButton>
          <IconButton
            label={t('terminal.search.next')}
            tooltip={t('terminal.search.next')}
            size="sm"
            onClick={() => runSearch('next')}
          >
            <ChevronDown aria-hidden="true" className="size-3.5" />
          </IconButton>
          <IconButton
            label={t('terminal.search.close')}
            tooltip={t('terminal.search.close')}
            size="sm"
            onClick={() => setShowSearch(false)}
          >
            <X aria-hidden="true" className="size-3.5" />
          </IconButton>
        </div>
      ) : null}
    </div>
  );
}
