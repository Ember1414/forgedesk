import { CircleUser, RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { GlobalSearch } from '@/app/shell/GlobalSearch';
import { RepoSwitcher } from '@/app/shell/RepoSwitcher';
import { cn } from '@/lib/utils';

/**
 * 顶部标题栏：仓库切换器 + 全局搜索入口 + 更新提示位 + 账号位。
 *
 * 布局决策（原创，未参考任何竞品）：
 *   采用「左：身份与上下文（应用标 + 当前仓库）｜中：检索入口｜右：账号与更新」的单行结构。
 *   理由是把"我在哪个仓库"（上下文）与"我要找什么"（动作）放在注意力的左侧到中部，
 *   而把低频的账号/更新放到最右，符合从左到右递减的使用频率。
 *
 * 应用标记目前是一个纯 CSS 的抽象几何块（品牌标导出为前端资源后替换），
 * 不使用任何第三方标识（红线 R2）。
 */
export function TitleBar() {
  const { t } = useTranslation('shell');

  return (
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-line bg-surface px-3">
      <div className="flex shrink-0 items-center gap-2">
        {/* 抽象锻炉记号：方形炉体 + 右上角的火花缺口。纯几何，无第三方标识元素。 */}
        <span aria-hidden="true" className="relative size-5 rounded-sm bg-brand">
          <span className="absolute right-0 top-0 size-2 rounded-bl-sm bg-spark" />
        </span>
        <span className="hidden text-14 font-semibold tracking-tight sm:inline">
          {t('common:app.name')}
        </span>
      </div>

      <RepoSwitcher />

      <div className="flex min-w-0 flex-1 justify-center px-2">
        <GlobalSearch />
      </div>

      <div className="flex shrink-0 items-center gap-1">
        <span
          className="hidden items-center gap-1.5 rounded-sm px-2 py-1 text-12 text-fg-subtle lg:flex"
          title={t('titleBar.update.label')}
        >
          <RefreshCw aria-hidden="true" className="size-3.5" />
          {t('titleBar.update.latest')}
        </span>

        <button
          type="button"
          aria-label={t('titleBar.account.label')}
          title={t('titleBar.account.signedOut')}
          className={cn(
            'fd-transition flex size-8 items-center justify-center rounded-md border border-line',
            'text-fg-subtle hover:border-line-strong hover:bg-surface-sunken hover:text-fg',
          )}
        >
          <CircleUser aria-hidden="true" className="size-4" />
        </button>
      </div>
    </header>
  );
}
