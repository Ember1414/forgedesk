import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

/**
 * 骨架页面（M0 专用）。
 *
 * 为什么要有它：T0.4 只做外壳与导航，十多个页面此刻都是空壳。
 * 若每个页面各写一份"这里以后会有东西"的说明，既重复又容易在后续任务里
 * 漏删。统一组件保证：① 文案一致；② 每个空壳都显式声明自己的**计划任务号**，
 * 后续实现时能一眼看出"这个页面归属哪个里程碑"，不会误以为已经做完。
 *
 * 约束：本组件不含任何业务逻辑，所有文案走 i18n key。
 */
export interface PlaceholderPageProps {
  /** 页面标题的 i18n key（shell 命名空间）。 */
  readonly titleKey: string;
  /** 页面说明的 i18n key（shell 命名空间）。 */
  readonly descriptionKey: string;
  /** 计划实现该页面的任务号，如 'T1.3'。 */
  readonly plannedTask: string;
  /** 页面自己的示意内容（可选）。 */
  readonly children?: ReactNode;
}

export function PlaceholderPage({
  titleKey,
  descriptionKey,
  plannedTask,
  children,
}: PlaceholderPageProps) {
  const { t } = useTranslation('shell');

  return (
    <section className="flex h-full flex-col gap-4">
      <header className="flex flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <h1 className="text-20 font-semibold tracking-tight">{t(titleKey)}</h1>
          <span className="rounded-sm border border-line bg-surface-sunken px-2 py-0.5 font-mono text-12 text-fg-subtle">
            {t('placeholder.planned')} {plannedTask}
          </span>
        </div>
        <p className="text-13 text-fg-muted">{t(descriptionKey)}</p>
      </header>

      {children}

      <p className="rounded-md border border-dashed border-line bg-surface p-4 text-12 text-fg-subtle">
        {t('placeholder.note')}
      </p>
    </section>
  );
}
