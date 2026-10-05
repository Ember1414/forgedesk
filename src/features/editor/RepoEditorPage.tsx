/**
 * 编辑器页面（T5.7）：文件树 + Monaco 多标签。
 *
 * 布局：左侧文件树（可折叠），右侧编辑器标签。文件从树上打开
 * （fs_read → 标签装配）；保存走 fs_write（EOL/BOM 保留）；保存后
 * 刷新仓库状态（工作区"修改"计数随之变化）。
 */
import { useCallback, useState } from 'react';

import { useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';

import { EditorPanel } from '@/features/editor/EditorPanel';
import { FileTreePanel } from '@/features/editor/FileTreePanel';
import { openEditorFile } from '@/features/editor/editorSupport';
import { useEditorStore } from '@/stores/editorStore';
import { useAppError } from '@/lib/errors';
import { Button } from '@/ui/components/button';

export function RepoEditorPage() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const openTab = useEditorStore((state) => state.openTab);
  const { show } = useAppError();
  const [treeOpen, setTreeOpen] = useState(true);

  const openFile = useCallback(
    (path: string) => {
      void openEditorFile(repoId, path)
        .then((tab) => openTab(tab))
        .catch((error) => show(error));
    },
    [openTab, repoId, show],
  );

  if (!Number.isFinite(repoId)) {
    return <p className="text-fg-muted p-4 text-13">{t('editor.notOpen')}</p>;
  }

  return (
    <div className="flex h-full min-h-0 flex-col p-3">
      <div className="flex items-center gap-2 pb-2">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.editor.title')}</h1>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => setTreeOpen((open) => !open)}
          aria-expanded={treeOpen}
        >
          {t('editor.toggleTree')}
        </Button>
      </div>
      <div className="flex min-h-0 flex-1 gap-2">
        {treeOpen ? (
          <div className="w-64 shrink-0">
            <FileTreePanel repoId={repoId} onOpenFile={openFile} />
          </div>
        ) : null}
        <EditorPanel repoId={repoId} />
      </div>
    </div>
  );
}
