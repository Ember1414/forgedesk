/**
 * README 查看对话框（T4.6）。
 *
 * # 内容是"已消毒"的，但仍不信任渲染层
 *
 * 后端（services::readme）已经把 Markdown 白名单化成 HTML：脚本、事件
 * 属性、javascript:/data: 协议在 Rust 侧就被剥掉（XSS 用例在那边穷举）。
 * 这里用 `dangerouslySetInnerHTML` 是**唯一**的注入点，且只接收这一条
 * 后端产物；前端不再对它做任何二次解析。
 *
 * # 链接为什么点击后复制
 *
 * 消毒过的 <a> 带 rel=noopener，但没有 opener 插件时 webview 内导航
 * 会把整个应用带走。委托拦截：点链接 = 复制地址（与登录向导、仓库
 * 列表的复制语义一致），opener 接入后统一换成真跳转。
 */
import { useEffect, useState } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import { repoRemoteReadme } from '@/lib/ipc';
import type { RemoteRepo } from '@/lib/ipc';
import { pushToast } from '@/stores/toastStore';

import { Button } from '@/ui/components/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';
import { ErrorState } from '@/ui/components/error-state';

/** 与远程仓库页一致的站点常量（当前唯一已实现 provider）。 */
const HOST = 'github.com';

interface ReadmeDialogProps {
  /** 目标仓库；`null` 表示关闭。 */
  readonly repo: RemoteRepo | null;
  readonly onOpenChange: (open: boolean) => void;
}

export function ReadmeDialog({ repo, onOpenChange }: ReadmeDialogProps) {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const [html, setHtml] = useState<string | null>(null);
  const [notFound, setNotFound] = useState(false);
  const [error, setError] = useState(false);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    // 状态更新一律经微任务：不在 effect 体内同步 setState（react-hooks 规则）
    if (repo === null) {
      // 关闭时清状态：下一个仓库不能看到上一个的内容残留
      void Promise.resolve().then(() => {
        setHtml(null);
        setNotFound(false);
        setError(false);
      });
      return;
    }
    const cancelled = { value: false };
    void Promise.resolve()
      .then(() => {
        setLoading(true);
        return repoRemoteReadme(HOST, repo.owner, repo.name);
      })
      .then((rendered) => {
        if (!cancelled.value) {
          setHtml(rendered);
        }
      })
      .catch((raw: unknown) => {
        if (cancelled.value) {
          return;
        }
        const code = (raw as { code?: string }).code;
        if (code === 'NOT_FOUND') {
          setNotFound(true);
        } else {
          setError(true);
          show(raw);
        }
      })
      .finally(() => {
        if (!cancelled.value) {
          setLoading(false);
        }
      });
    return () => {
      cancelled.value = true;
    };
  }, [repo, show]);

  // 外链委托：点击 = 复制地址（webview 内导航会把应用带走）
  const onContentClick = (event: React.MouseEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement;
    const anchor = target.closest('a');
    if (anchor !== null) {
      event.preventDefault();
      const href = anchor.getAttribute('href') ?? '';
      if (href !== '') {
        void navigator.clipboard?.writeText(href).catch(() => undefined);
        pushToast({ tone: 'info', title: t('github.repos.linkCopiedToast') });
      }
    }
  };

  return (
    <Dialog open={repo !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl" closeLabel={t('common:actions.close')}>
        <DialogHeader>
          <DialogTitle>{t('github.repos.readmeTitle', { fullName: repo?.fullName ?? '' })}</DialogTitle>
          <DialogDescription>{t('github.repos.readmeDescription')}</DialogDescription>
        </DialogHeader>

        {loading ? (
          <p className="text-13 text-fg-subtle" data-testid="readme-loading">
            {t('github.repos.loading')}
          </p>
        ) : null}

        {notFound ? (
          <ErrorState title={t('github.repos.readmeMissing')} />
        ) : null}

        {error && !notFound ? (
          <ErrorState title={t('github.repos.listErrorHint')} />
        ) : null}

        {html !== null && !loading ? (
          <div
            className="max-h-[60vh] overflow-auto rounded-md border border-line bg-surface p-4 text-13 leading-relaxed"
            onClick={onContentClick}
            data-testid="readme-content"
            // 内容已在 Rust 侧白名单消毒（services::readme，XSS 用例在彼处）；
            // 这是全文唯一的安全 HTML 注入点
            dangerouslySetInnerHTML={{ __html: html }}
          />
        ) : null}

        <div className="flex justify-end">
          <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
            {t('common:actions.close')}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
