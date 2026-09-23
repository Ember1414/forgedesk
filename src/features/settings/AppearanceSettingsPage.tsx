import { useTranslation } from 'react-i18next';

import { THEME_MODES } from '@/app/theme';
import type { ThemeMode } from '@/app/theme';
import { SUPPORTED_LANGUAGES, changeLanguage, resolveActiveLanguage } from '@/lib/i18n';
import type { AppLanguage } from '@/lib/i18n';
import { ToggleGroup } from '@/ui/components/toggle-group';
import { useUiStore } from '@/stores/uiStore';

/**
 * 外观设置：主题与语言。
 *
 * 这是 M0 里少数**真的能用**的设置项（其余设置页仍是骨架），因此它承担两件事：
 *   1. 让主题/语言的切换有正式入口（此前只能改开发预览页）；
 *   2. 验证 uiStore 与 i18n 的接线在真实页面里可用（后续设置项照此模式累加）。
 *
 * 切换即时生效：主题写入 <html data-theme>（见 app/theme.ts），
 * 语言走 i18next 的 changeLanguage，react-i18next 会自动触发重渲染。
 */
export function AppearanceSettingsPage() {
  const { t, i18n } = useTranslation('shell');
  const themeMode = useUiStore((state) => state.themeMode);
  const setThemeMode = useUiStore((state) => state.setThemeMode);

  const activeLanguage = resolveActiveLanguage(i18n.resolvedLanguage ?? i18n.language);

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">
          {t('pages.settingsAppearance.title')}
        </h1>
        <p className="text-13 text-fg-muted">{t('pages.settingsAppearance.description')}</p>
      </header>

      <div className="flex flex-col gap-4 rounded-lg border border-line bg-surface p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex flex-col gap-0.5">
            <span className="text-14 font-medium">{t('settings.appearance.themeLabel')}</span>
            <span className="text-12 text-fg-subtle">{t('settings.appearance.themeHint')}</span>
          </div>
          <ToggleGroup
            label={t('settings.appearance.themeLabel')}
            value={themeMode}
            options={THEME_MODES.map((mode) => ({
              value: mode,
              label: t(`common:theme.${mode}`),
            }))}
            onValueChange={(next) => {
              setThemeMode(next as ThemeMode);
            }}
          />
        </div>

        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-line pt-4">
          <div className="flex flex-col gap-0.5">
            <span className="text-14 font-medium">{t('settings.appearance.languageLabel')}</span>
            <span className="text-12 text-fg-subtle">{t('settings.appearance.languageHint')}</span>
          </div>
          <ToggleGroup
            label={t('settings.appearance.languageLabel')}
            value={activeLanguage}
            options={SUPPORTED_LANGUAGES.map((language) => ({
              value: language,
              label: t(`common:language.${language}`),
            }))}
            onValueChange={(next) => {
              void changeLanguage(next as AppLanguage);
            }}
          />
        </div>
      </div>
    </section>
  );
}
