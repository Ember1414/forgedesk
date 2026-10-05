/**
 * 编辑器面板（T5.7）：Monaco + 多标签 + 外部变更三选一。
 *
 * # 资源策略
 *
 * - Monaco **本地打包**（不用 CDN：CSP 禁止远程脚本、离线必须可用），
 *   语言 worker 只带 editor 核心 + ts/css/html/json——任务书明确
 *   "不得打包全部语言"；其余扩展名仍可编辑（无高亮但功能完整）。
 * - `@monaco-editor/react` 的 loader 被指向本地 monaco，因此真正加载
 *   发生在首个文件打开时（懒 chunk，不进首屏）。
 *
 * # 外部变更三选一（任务书硬性要求：绝不静默覆盖）
 *
 * 文件打开/保存时记录磁盘基线；`repo:changed`（文件监听，T1.10）到达后
 * 对打开的文件重新 fs_read，与基线不同 → 提示三选一：
 * 重新加载（丢弃我的编辑）/ 保留我的编辑 / 并排对比。
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { Unlisten } from '@/lib/ipc/client';

import { Editor, loader, type Monaco } from '@monaco-editor/react';
import { useTranslation } from 'react-i18next';

import { watchExternalChanges } from '@/features/editor/editorSupport';
import { fsRead, fsWrite } from '@/lib/ipc/fs';
import type { EditorTab } from '@/stores/editorStore';
import { useEditorStore } from '@/stores/editorStore';
import { Button } from '@/ui/components/button';
import { cn } from '@/lib/utils';

// Monaco 本地装配（一次即可）。
//
// 语言策略（任务书：不得打包全部语言、不做 LSP/智能补全）：
// - worker 只有 editor 核心（查找/多光标/折叠等基础能力都在主线程 API 里，
//   worker 负责跨文件功能——正是"不做清单"里的东西）；
// - 语法高亮用 basic-languages 的 Monarch tokenizer（每个语言几十 KB），
//   按常用集合手动列出；集合之外的扩展名仍可编辑（无高亮，功能完整）。
// - 因此没有 ts/css/html/json worker（ts.worker 独占 10MB，是最大的一块）。
import * as monacoCore from 'monaco-editor/esm/vs/editor/editor.api';
import editorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker';

// 常用语言的 Monarch 高亮（import 即注册；几十 KB/语言）
import 'monaco-editor/esm/vs/basic-languages/typescript/typescript.contribution';
import 'monaco-editor/esm/vs/basic-languages/javascript/javascript.contribution';
import 'monaco-editor/esm/vs/basic-languages/css/css.contribution';
import 'monaco-editor/esm/vs/basic-languages/scss/scss.contribution';
import 'monaco-editor/esm/vs/basic-languages/html/html.contribution';
import 'monaco-editor/esm/vs/basic-languages/xml/xml.contribution';
import 'monaco-editor/esm/vs/basic-languages/markdown/markdown.contribution';
import 'monaco-editor/esm/vs/basic-languages/rust/rust.contribution';
import 'monaco-editor/esm/vs/basic-languages/python/python.contribution';
import 'monaco-editor/esm/vs/basic-languages/yaml/yaml.contribution';
import 'monaco-editor/esm/vs/basic-languages/shell/shell.contribution';
import 'monaco-editor/esm/vs/basic-languages/ini/ini.contribution';
import 'monaco-editor/esm/vs/basic-languages/sql/sql.contribution';
import 'monaco-editor/esm/vs/basic-languages/go/go.contribution';
import 'monaco-editor/esm/vs/basic-languages/java/java.contribution';
import 'monaco-editor/esm/vs/basic-languages/cpp/cpp.contribution';
import 'monaco-editor/esm/vs/basic-languages/dockerfile/dockerfile.contribution';

let monacoConfigured = false;
function configureMonaco(): void {
  if (monacoConfigured) {
    return;
  }
  monacoConfigured = true;
  self.MonacoEnvironment = {
    getWorker() {
      return new editorWorker();
    },
  };
  loader.config({ monaco: monacoCore as unknown as Monaco });
}

export interface EditorPanelProps {
  readonly repoId: number;
}

/** 编辑器主面板（由仓库内路由挂载）。 */
export function EditorPanel({ repoId }: EditorPanelProps) {
  const { t } = useTranslation('shell');
  const tabs = useEditorStore((state) => state.tabs);
  const activePath = useEditorStore((state) => state.activePath);
  const setActive = useEditorStore((state) => state.setActive);
  const closeTab = useEditorStore((state) => state.closeTab);

  return (
    <div className="flex h-full min-h-0 flex-col">
      {tabs.length > 0 ? (
        <div
          className="border-line flex flex-wrap items-center gap-1 border-b pb-1.5"
          role="tablist"
          aria-label={t('editor.tablistLabel')}
        >
          {tabs.map((tab) => (
            <button
              key={tab.path}
              type="button"
              role="tab"
              aria-selected={tab.path === activePath}
              onClick={() => setActive(tab.path)}
              className={cn(
                'fd-transition flex items-center gap-1 rounded-md px-2 py-1 text-13',
                tab.path === activePath
                  ? 'bg-surface-raised text-fg'
                  : 'text-fg-muted hover:bg-surface-sunken hover:text-fg',
              )}
              title={tab.path}
            >
              {tab.isBinary ? '⚙' : tab.dirtyDisk ? '⚠' : ''}
              {tab.name}
            </button>
          ))}
        </div>
      ) : null}

      {tabs.length === 0 ? (
        <div className="text-fg-muted flex flex-1 items-center justify-center">
          <p className="text-13">{t('editor.empty')}</p>
        </div>
      ) : (
        tabs.map((tab) => (
          <EditorTabView
            key={tab.path}
            tab={tab}
            repoId={repoId}
            active={tab.path === activePath}
            onClose={() => closeTab(tab.path)}
          />
        ))
      )}
    </div>
  );
}

function EditorTabView({
  tab,
  repoId,
  active,
  onClose,
}: {
  readonly tab: EditorTab;
  readonly repoId: number;
  readonly active: boolean;
  readonly onClose: () => void;
}) {
  const { t } = useTranslation('shell');
  const markSaved = useEditorStore((state) => state.markSaved);
  const markDirtyDisk = useEditorStore((state) => state.markDirtyDisk);
  const keepMine = useEditorStore((state) => state.keepMine);
  const [model, setModel] = useState<string | null>(tab.baseline);
  const [saving, setSaving] = useState(false);
  const [showCompare, setShowCompare] = useState(false);
  const [diskContent, setDiskContent] = useState<string | null>(null);
  const modelRef = useRef<string | null>(model);
  useEffect(() => {
    modelRef.current = model;
  }, [model]);

  // 外部变更检测：repo:changed 后重读磁盘，与基线不一致 → 三选一。
  useEffect(() => {
    if (!active) {
      return;
    }
    let cancelled = false;
    let unlisten: Unlisten | undefined;
    void watchExternalChanges(repoId, tab.path, () => {
      void fsRead(repoId, tab.path)
        .then((content) => {
          if (cancelled) {
            return;
          }
          const disk = content.content ?? '';
          setDiskContent(disk);
          if (modelRef.current !== null && disk !== modelRef.current) {
            markDirtyDisk(tab.path);
          } else if (modelRef.current !== null) {
            // 磁盘与编辑器一致（可能是自己保存的回声）：更新基线
            markSaved(tab.path, disk);
          }
        })
        .catch(() => {});
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [active, markDirtyDisk, markSaved, repoId, tab.path]);

  const save = useCallback(async () => {
    if (modelRef.current === null) {
      return;
    }
    setSaving(true);
    try {
      await fsWrite(repoId, tab.path, modelRef.current, tab.eol, tab.hasBom);
      markSaved(tab.path, modelRef.current);
      setDiskContent(null);
    } finally {
      setSaving(false);
    }
  }, [markSaved, repoId, tab.eol, tab.hasBom, tab.path]);

  const reloadFromDisk = useCallback(() => {
    if (diskContent !== null) {
      setModel(diskContent);
      markSaved(tab.path, diskContent);
      setDiskContent(null);
      setShowCompare(false);
    }
  }, [diskContent, markSaved, tab.path]);

  return (
    <div className={cn('relative min-h-0 min-w-0 flex-1', active ? 'flex' : 'hidden')}>
      {tab.isBinary ? (
        <div className="text-fg-muted flex flex-1 items-center justify-center text-13">
          {t('editor.binaryNotice', { size: tab.name })}
        </div>
      ) : (
        <>
          <div className="border-line flex items-center gap-2 border-b px-2 py-1">
            <Button size="sm" disabled={saving} onClick={() => void save()}>
              {t('editor.save')}
            </Button>
            {tab.dirtyDisk ? (
              <span className="text-warning text-12">{t('editor.externalChanged')}</span>
            ) : null}
            <span className="text-fg-subtle font-mono text-11">{tab.path}</span>
            <span className="flex-1" />
            <Button size="sm" variant="ghost" onClick={onClose}>
              {t('editor.close')}
            </Button>
          </div>

          {tab.dirtyDisk ? (
            <div className="border-warning bg-surface-raised text-fg flex flex-wrap items-center gap-2 border-b px-3 py-2">
              <span className="text-12">{t('editor.conflictPrompt')}</span>
              <Button size="sm" variant="danger" onClick={reloadFromDisk}>
                {t('editor.conflictReload')}
              </Button>
              <Button size="sm" onClick={() => keepMine(tab.path)}>
                {t('editor.conflictKeepMine')}
              </Button>
              {diskContent !== null ? (
                <Button size="sm" variant="secondary" onClick={() => setShowCompare((v) => !v)}>
                  {t('editor.conflictCompare')}
                </Button>
              ) : null}
            </div>
          ) : null}

          {showCompare && diskContent !== null ? (
            <div className="border-line flex min-h-0 flex-1 border-b">
              <pre className="border-line w-1/2 overflow-auto border-e p-2 font-mono text-12">
                {diskContent}
              </pre>
              <pre className="w-1/2 overflow-auto p-2 font-mono text-12">{model}</pre>
            </div>
          ) : null}

          <div className="min-h-0 flex-1">
            <Editor
              theme="vs-dark"
              path={tab.path}
              value={model ?? ''}
              onChange={(value) => setModel(value ?? '')}
              onMount={(editor, monaco) => {
                configureMonaco();
                // 诊断/E2E 钩子（与 __forgedeskTermText 同款做法）：只读暴露实例
                (window as unknown as { __forgedeskEditor?: unknown }).__forgedeskEditor = editor;
                editor.focus();
                // 默认基础补全（编辑器内置；不做 LSP——"不做清单"）
                monaco.languages.typescript?.typescriptDefaults?.setCompilerOptions({
                  allowNonTsExtensions: true,
                  noSemanticValidation: true,
                  noSyntaxValidation: false,
                });
              }}
              options={{
                fontSize: 13,
                minimap: { enabled: false },
                wordWrap: 'on',
                automaticLayout: true,
              }}
            />
          </div>
        </>
      )}
    </div>
  );
}
