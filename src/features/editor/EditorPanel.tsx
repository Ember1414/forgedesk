/**
 * 编辑器面板（T5.7/T5.8）：Monaco + 多标签 + 外部变更三选一 + Blame + 文件历史。
 *
 * # 资源策略
 *
 * Monaco 本地打包（不用 CDN：CSP 禁止远程脚本、离线必须可用），语言 worker
 * 只带 editor 核心（0.5MB）；语法高亮用 basic-languages 的 Monarch tokenizer
 * （每个语言几十 KB）——任务书明确"不得打包全部语言"。
 *
 * # 外部变更三选一（任务书硬性要求：绝不静默覆盖）
 *
 * 打开/保存时记录磁盘基线；`repo:changed`（文件监听，T1.10）到达后重读
 * 磁盘比对，不一致 → 提示三选一：重新加载 / 保留我的编辑 / 并排对比。
 *
 * # Blame 与文件历史（T5.8）
 *
 * Blame 用 Monaco 装饰渲染（左侧色条 + hover 摘要，未提交行单独标记）；
 * 点击行打开提交详情对话框。文件历史面板列出 A/M/D/R 时间线，条目可
 * "与当前内容并排对比"（git_file_at 取历史版本）。
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { Unlisten } from '@/lib/ipc/client';

import { Editor, loader, type Monaco } from '@monaco-editor/react';
import { useTranslation } from 'react-i18next';

import { applyBlameDecorations, blameAtLine } from '@/features/editor/blameDecorations';
import { CommitDetailDialog } from '@/features/editor/FileHistoryPanel';
import { FileHistoryPanel } from '@/features/editor/FileHistoryPanel';
import { watchExternalChanges } from '@/features/editor/editorSupport';
import { gitBlame } from '@/lib/ipc/blame';
import type { BlameLine } from '@/lib/ipc/blame';
import { fsRead, fsWrite } from '@/lib/ipc/fs';
import type { EditorTab } from '@/stores/editorStore';
import { useEditorStore } from '@/stores/editorStore';
import { Button } from '@/ui/components/button';
import { cn } from '@/lib/utils';
import '@/features/editor/blame.css';

// Monaco 本地装配（一次即可）。
// 语言策略（任务书：不得打包全部语言、不做 LSP）：只有 0.5MB editor worker；
// 高亮用 basic-languages 的 Monarch tokenizer（import 即注册，~16KB/语言）。
import * as monacoCore from 'monaco-editor/esm/vs/editor/editor.api';
import editorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker';
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
  const [compareContent, setCompareContent] = useState<string | null>(null);
  const [blameOn, setBlameOn] = useState(false);
  const [blame, setBlame] = useState<readonly BlameLine[]>([]);
  const [historyOn, setHistoryOn] = useState(false);
  const [detailOid, setDetailOid] = useState<string | null>(null);
  const [editorReady, setEditorReady] = useState(false);
  const modelRef = useRef<string | null>(model);
  const editorRef = useRef<
    Parameters<NonNullable<Parameters<typeof Editor>[0]['onMount']>>[0] | null
  >(null);
  const blameCollectionRef = useRef<{ clear: () => void } | null>(null);

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

  // blame 开关：拉取逐行归属并渲染为 Monaco 装饰（未提交行单独标记）
  useEffect(() => {
    const editor = editorRef.current;
    if (!editorReady || !editor || !blameOn || !active) {
      return;
    }
    let cancelled = false;
    void gitBlame(repoId, tab.path, { detectMoves: true })
      .then((lines) => {
        if (cancelled) {
          return;
        }
        setBlame(lines);
        blameCollectionRef.current = applyBlameDecorations(editor, lines);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      blameCollectionRef.current?.clear();
      blameCollectionRef.current = null;
      setBlame([]);
    };
  }, [blameOn, active, editorReady, repoId, tab.path]);
  void editorReady;

  // blame 点击 → 该行提交详情
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || !blameOn || blame.length === 0) {
      return;
    }
    const listener = editor.onMouseDown((event) => {
      const line = event.target.position?.lineNumber;
      if (line === undefined) {
        return;
      }
      const hit = blameAtLine(blame, line);
      if (hit !== null) {
        setDetailOid(hit.oid);
      }
    });
    return () => {
      listener.dispose();
    };
  }, [blameOn, blame]);

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

  const insertExample = useCallback((example: string) => {
    const managed = editorRef.current;
    if (managed) {
      managed.trigger('insertExample', 'type', { text: example });
    }
  }, []);
  void insertExample;

  return (
    <div className={cn('relative flex min-h-0 min-w-0 flex-1', active ? '' : 'hidden')}>
      {detailOid !== null ? (
        <CommitDetailDialog repoId={repoId} oid={detailOid} onClose={() => setDetailOid(null)} />
      ) : null}
      {tab.isBinary ? (
        <div className="text-fg-muted flex flex-1 items-center justify-center text-13">
          {t('editor.binaryNotice', { name: tab.name })}
        </div>
      ) : (
        <div className="flex min-h-0 min-w-0 flex-1">
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            <div className="border-line flex items-center gap-2 border-b px-2 py-1">
              <Button size="sm" disabled={saving} onClick={() => void save()}>
                {t('editor.save')}
              </Button>
              {tab.dirtyDisk ? (
                <span className="text-warning text-12">{t('editor.externalChanged')}</span>
              ) : null}
              <span className="text-fg-subtle font-mono text-11">{tab.path}</span>
              <span className="flex-1" />
              <Button
                size="sm"
                variant={blameOn ? 'primary' : 'ghost'}
                onClick={() => setBlameOn((on) => !on)}
              >
                {t('editor.blame.toggle')}
              </Button>
              <Button
                size="sm"
                variant={historyOn ? 'primary' : 'ghost'}
                onClick={() => setHistoryOn((on) => !on)}
              >
                {t('editor.history.toggle')}
              </Button>
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

            {showCompare && !tab.isBinary ? (
              <div className="border-line flex min-h-0 flex-1 border-b">
                <pre className="border-line w-1/2 overflow-auto border-e p-2 font-mono text-12">
                  {compareContent ?? diskContent}
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
                  editorRef.current = editor;
                  setEditorReady(true);
                  editor.focus();
                  // 不做 LSP（"不做清单"）：关掉 TS 语义校验，只留语法高亮
                  monaco.languages.typescript?.typescriptDefaults?.setCompilerOptions({
                    allowNonTsExtensions: true,
                    noSemanticValidation: true,
                    noSyntaxValidation: false,
                  });
                }}
                options={{
                  fontSize: 13,
                  minimap: { enabled: false },
                  // blame 色条按行对齐的前提是行高恒定（word wrap 会让一行占多行）
                  wordWrap: blameOn ? ('off' as const) : ('on' as const),
                  automaticLayout: true,
                  lineDecorationsWidth: blameOn ? 16 : 10,
                }}
              />
            </div>
          </div>

          {historyOn ? (
            <FileHistoryPanel
              repoId={repoId}
              path={tab.path}
              onClose={() => setHistoryOn(false)}
              onCompare={(content) => {
                setCompareContent(content);
                setShowCompare(true);
              }}
            />
          ) : null}
        </div>
      )}
    </div>
  );
}
