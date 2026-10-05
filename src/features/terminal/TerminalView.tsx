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
import { CaseSensitive, ChevronDown, ChevronUp, Regex, TriangleAlert, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import { Terminal } from '@xterm/xterm';

import {
  applyFontToAll,
  dropLineTracker,
  findTerminalInstance,
  getLastCommand,
  getLineTracker,
  openExternalUrl,
  recordCommand,
  registerTerminal,
  scheduleGitRefresh,
  setQueryClientForRefresh,
  unregisterTerminal,
} from '@/features/terminal/manager';
import { isGitCommand } from '@/features/terminal/gitInputRefresh';
import { termReportCommand, termResize, termScanCommand, termWrite } from '@/lib/ipc';
import type { TermDanger } from '@/lib/ipc';
import {
  TERMINAL_SAFETY_AUTO_SNAPSHOT_KEY,
  TERMINAL_SAFETY_ENABLED_KEY,
  TERMINAL_SAFETY_LEVEL_KEY,
  useSettingsStore,
} from '@/stores/settingsStore';
import type { TerminalSafetyLevel } from '@/stores/settingsStore';
import { useTerminalStore } from '@/stores/terminalStore';
import type { TerminalTab } from '@/stores/terminalStore';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogTitle,
} from '@/ui/components/alert-dialog';
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
  const [hint, setHint] = useState<TermDanger | null>(null);
  const pendingConfirm = useTerminalStore((state) =>
    state.pendingConfirm?.termId === tab.termId ? state.pendingConfirm : null,
  );
  const setStorePendingConfirm = useTerminalStore((state) => state.setPendingConfirm);
  const queryClient = useQueryClient();
  const hintTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // 终端安全设置（T5.3）：mount effect 的闭包通过 ref 读取最新值
  const safetyEnabled = useSettingsStore((state) =>
    state.getJson<boolean>(TERMINAL_SAFETY_ENABLED_KEY, true),
  );
  const safetyLevel = useSettingsStore((state) =>
    state.getJson<TerminalSafetyLevel>(TERMINAL_SAFETY_LEVEL_KEY, 'hint'),
  );
  const autoSnapshot = useSettingsStore((state) =>
    state.getJson<boolean>(TERMINAL_SAFETY_AUTO_SNAPSHOT_KEY, true),
  );
  const safetyRef = useRef({ safetyEnabled, safetyLevel, autoSnapshot });
  useEffect(() => {
    safetyRef.current = { safetyEnabled, safetyLevel, autoSnapshot };
  }, [autoSnapshot, safetyEnabled, safetyLevel]);
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

    setQueryClientForRefresh(queryClient);

    // 键入：写后端 + 行跟踪（命令历史 / git 命令刷新 / T5.3 安全流）。
    //
    // Enter 是唯一被"扣下"的键：行内容先交给识别器（异步），按结果决定
    // 直接放行（安全/识别失败/提示级）还是挂起等确认（确认级 + 高危）。
    // 其余字节永远原样、立即转发——拦截不修改用户输入。
    // 行跟踪器在 manager（模块级）：视图重挂载不丢行缓冲。
    const tracker = getLineTracker(tab.termId);
    const ENTER = new Uint8Array([13]);
    const sendEnter = () => {
      void termWrite(tab.termId, ENTER).catch(() => {});
    };
    const dataSubscription = term.onData((data) => {
      const completed = tracker.feed(data);
      const withoutEnter = data.replace(/\r/g, '');
      if (withoutEnter !== '') {
        void termWrite(tab.termId, new TextEncoder().encode(withoutEnter));
      }
      for (const line of completed) {
        if (line.trim() !== '') {
          recordCommand(tab.repoId, line);
        }
        if (isGitCommand(line)) {
          scheduleGitRefresh(tab.repoId);
        }
      }
      if (!data.includes('\r')) {
        return;
      }
      const safety = safetyRef.current;
      const last = [...completed].reverse().find((line) => line.trim() !== '');
      if (!safety.safetyEnabled || !last) {
        sendEnter();
        return;
      }
      void termScanCommand(last)
        .then((danger) => {
          if (!danger) {
            sendEnter();
            return;
          }
          // 始终记录级（强制）：无论级别，登记 + 可选补偿快照（后端）
          void termReportCommand({
            repoId: tab.repoId,
            line: last,
            kind: danger.kind,
            autoSnapshot: safety.autoSnapshot,
          }).catch(() => {
            // 留痕失败不阻断终端
          });
          if (danger.level === 'caution' || safety.safetyLevel === 'hint') {
            // 提示级：非阻塞条，2.5s 自动消失；命令照常执行
            if (hintTimerRef.current !== null) {
              clearTimeout(hintTimerRef.current);
            }
            setHint(danger);
            hintTimerRef.current = setTimeout(() => setHint(null), 2500);
            sendEnter();
          } else {
            setStorePendingConfirm({
              termId: tab.termId,
              kind: danger.kind,
              level: danger.level,
              canonical: danger.canonical,
            });
          }
        })
        .catch(() => {
          // 识别服务不可用：放行（宁可漏报不可误伤）
          sendEnter();
        });
    });

    // 键盘约定：见文件头
    term.attachCustomKeyEventHandler((event) => {
      // 确认对话框打开期间冻结全部输入（防止挂起的行被继续改写）
      if (useTerminalStore.getState().pendingConfirm !== null) {
        return false;
      }
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
      if (hintTimerRef.current !== null) {
        clearTimeout(hintTimerRef.current);
      }
      observer.disconnect();
      dataSubscription.dispose();
      unregisterTerminal(tab.termId);
      dropLineTracker(tab.termId);
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

  // 确认对话框关闭后焦点必须回到终端：Radix 的焦点还原落在 body 上，
  // 不补焦的下一轮键入会全部丢失（T5.3 E2E 实测）。
  useEffect(() => {
    if (pendingConfirm === null && active) {
      findTerminalInstance(tab.termId)?.term.focus();
    }
  }, [pendingConfirm, active, tab.termId]);

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

  /** 确认执行：把挂起的 Enter 发出去，shell 侧的行原样落地（不改写输入）。 */
  const confirmDanger = useCallback(() => {
    setStorePendingConfirm(null);
    void termWrite(tab.termId, new Uint8Array([13])).catch(() => {});
    scheduleGitRefresh(tab.repoId);
  }, [setStorePendingConfirm, tab.repoId, tab.termId]);

  /** 取消执行：发 Ctrl+C 取消 shell 侧的行（PSReadLine 行取消 / bash SIGINT）。 */
  const cancelDanger = useCallback(() => {
    setStorePendingConfirm(null);
    void termWrite(tab.termId, new Uint8Array([3])).catch(() => {});
    scheduleGitRefresh(tab.repoId);
  }, [setStorePendingConfirm, tab.repoId, tab.termId]);

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

      {hint ? (
        <div
          role="status"
          className="border-warning bg-surface-raised text-fg absolute inset-x-3 top-3 z-30 flex items-center gap-2 rounded-md border px-3 py-2 shadow-lg"
        >
          <TriangleAlert aria-hidden="true" className="text-warning size-4 shrink-0" />
          <p className="text-12 leading-snug">
            {t('terminal.safety.hint', { command: hint.canonical })}
            {hint.level === 'dangerous' ? (
              <>
                {' '}
                <button
                  type="button"
                  className="text-brand hover:underline"
                  onClick={() => {
                    // 图形安全入口：快照页（可回滚点的唯一清单）
                    window.location.hash = `/repo/${tab.repoId}/snapshots`;
                  }}
                >
                  {t('terminal.safety.openSnapshots')}
                </button>
              </>
            ) : null}
          </p>
        </div>
      ) : null}

      {pendingConfirm ? (
        <AlertDialog open>
          <AlertDialogContent
            tone="danger"
            impact={t('terminal.safety.confirmImpact', { command: pendingConfirm.canonical })}
          >
            <AlertDialogTitle>{t('terminal.safety.confirmTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('terminal.safety.confirmBody', { command: pendingConfirm.canonical })}
            </AlertDialogDescription>
            <AlertDialogAction onClick={confirmDanger}>
              {t('terminal.safety.confirmExecute')}
            </AlertDialogAction>
            <AlertDialogCancel onClick={cancelDanger}>
              {t('terminal.safety.confirmCancel')}
            </AlertDialogCancel>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}

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
