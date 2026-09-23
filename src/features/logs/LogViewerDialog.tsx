import { useTranslation } from 'react-i18next';

import { Button } from '@/ui/components/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';
import { LogViewer } from '@/features/logs/LogViewer';
import { logsOpen } from '@/lib/ipc';
import { useAppError } from '@/lib/errors';
import { useLogViewerStore } from '@/stores/logViewerStore';

/**
 * 全局日志对话框。
 *
 * 挂在应用根部（`src/app/App.tsx`）：错误提示里的"查看相关日志"可能在任意页面触发，
 * 因此需要一个不依赖当前路由的出口。
 *
 * 打开时机与高亮锚点都由 [`useLogViewerStore`] 决定，这里只负责渲染与"打开日志目录"。
 */
export function LogViewerDialog() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();

  const open = useLogViewerStore((state) => state.open);
  const nearTimestamp = useLogViewerStore((state) => state.nearTimestamp);
  const closeViewer = useLogViewerStore((state) => state.closeViewer);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          closeViewer();
        }
      }}
    >
      <DialogContent
        closeLabel={t('common:actions.close')}
        className="flex h-[70vh] max-w-3xl flex-col"
      >
        <DialogHeader>
          <DialogTitle>{t('logs.title')}</DialogTitle>
          <DialogDescription>{t('logs.description')}</DialogDescription>
        </DialogHeader>

        <LogViewer nearTimestamp={nearTimestamp} className="mt-4 min-h-0 flex-1" />

        <DialogFooter>
          <Button
            variant="secondary"
            onClick={() => {
              // 打开文件管理器失败时给出可读提示，而不是静默无反应
              void logsOpen().catch(show);
            }}
          >
            {t('logs.openDirectory')}
          </Button>
          <Button onClick={closeViewer}>{t('logs.close')}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
