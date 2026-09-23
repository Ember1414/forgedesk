import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';

/**
 * 404 页面。
 *
 * 为什么桌面应用也需要它：路由可由外部触发（未来的深链、插件跳转、
 * 甚至是开发者手改 hash），落到未知路径时给出明确出口比白屏好。
 */
export function NotFoundPage() {
  const { t } = useTranslation('shell');

  return (
    <section className="flex h-full flex-col items-center justify-center gap-3 text-center">
      <p className="font-mono text-32 font-semibold text-fg-subtle">404</p>
      <h1 className="text-20 font-semibold">{t('notFound.title')}</h1>
      <p className="max-w-md text-13 text-fg-muted">{t('notFound.description')}</p>
      <Link
        to="/"
        className="fd-transition mt-1 rounded-md bg-brand px-3 py-1.5 text-13 font-medium text-brand-fg hover:bg-brand-hover"
      >
        {t('notFound.back')}
      </Link>
    </section>
  );
}
