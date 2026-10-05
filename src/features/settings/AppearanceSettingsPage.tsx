import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { THEME_MODES, currentResolvedTheme } from '@/app/theme';
import type { ThemeMode } from '@/app/theme';
import { BUILTIN_THEMES, builtinThemeFor } from '@/features/themes/builtinThemes';
import { useCustomThemesStore } from '@/features/themes/customThemesStore';
import {
  currentActiveCustomTheme,
  setActiveCustomTheme,
} from '@/features/themes/activeCustomTheme';
import { contrastReport } from '@/features/themes/themeModel';
import type { ThemeDefinition, ThemeFieldError } from '@/features/themes/themeModel';
import { SUPPORTED_LANGUAGES, changeLanguage, resolveActiveLanguage } from '@/lib/i18n';
import type { AppLanguage } from '@/lib/i18n';
import { useSettingsStore } from '@/stores/settingsStore';
import { useUiStore } from '@/stores/uiStore';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/ui/components/alert-dialog';
import { Badge } from '@/ui/components/badge';
import { Button } from '@/ui/components/button';
import { ToggleGroup } from '@/ui/components/toggle-group';

/**
 * 外观设置：主题模式、主题画廊（T6.6）与语言。
 *
 * 主题写入路径统一走 activeCustomTheme（缓存 + DOM 变量），本页不直接碰 inline style；
 * 激活时若当前解析外观与主题外观不一致，会顺手把模式切过去——否则用户点了暗色主题
 * 却"什么都没发生"（外观归属原则在视觉上不可感知）。
 */

const ERROR_CODE_SUFFIX: Record<string, string> = {
  notAnObject: 'NotAnObject',
  invalidId: 'InvalidId',
  builtinId: 'BuiltinId',
  invalidName: 'InvalidName',
  invalidAppearance: 'InvalidAppearance',
  invalidVersion: 'InvalidVersion',
  unknownToken: 'UnknownToken',
  invalidColor: 'InvalidColor',
  invalidFont: 'InvalidFont',
  invalidSizeScale: 'InvalidSizeScale',
  unknownXtermKey: 'UnknownXtermKey',
};

function themeSwatchStyle(token: string, value: string | undefined): { backgroundColor: string } {
  // 缺失 token 回退到 CSS 变量引用：内置值由 tokens.css 提供，不需要 JS 复制
  return {
    backgroundColor: value ?? `var(--fd-${token.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)})`,
  };
}

function ContrastRows({ theme }: { readonly theme: ThemeDefinition }): React.JSX.Element {
  const { t } = useTranslation('shell');
  const rows = contrastReport(theme);
  return (
    <div className="flex flex-col gap-1 text-12">
      <span className="font-medium text-fg-muted">{t('settings.appearance.contrastTitle')}</span>
      {rows.map((row) => (
        <div key={row.label} className="flex items-center justify-between gap-2">
          <span className="text-fg-subtle">
            {t(
              `settings.appearance.contrast${row.label.charAt(0).toUpperCase()}${row.label.slice(1)}`,
            )}
          </span>
          <span className={row.state === 'fail' ? 'font-medium text-danger' : 'text-fg-subtle'}>
            {row.state === 'inherited'
              ? t('settings.appearance.contrastInherited')
              : `${row.ratio?.toFixed(2)}:1 ${row.state === 'pass' ? '✓' : '✗'}`}
          </span>
        </div>
      ))}
    </div>
  );
}

function ThemeCard({
  theme,
  active,
  onActivate,
  onExport,
  onDelete,
}: {
  readonly theme: ThemeDefinition;
  readonly active: boolean;
  readonly onActivate: (theme: ThemeDefinition) => void;
  readonly onExport: (theme: ThemeDefinition) => void;
  readonly onDelete?: (theme: ThemeDefinition) => void;
}): React.JSX.Element {
  const { t } = useTranslation('shell');
  const colors = theme.colors as Readonly<Record<string, string | undefined>>;
  const swatchTokens = ['canvas', 'surface', 'brand', 'success', 'danger'] as const;

  return (
    <div
      className={`flex flex-col gap-2 rounded-lg border p-3 ${
        active ? 'border-brand bg-brand-subtle' : 'border-line bg-surface'
      }`}
      data-testid={`theme-card-${theme.id}`}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-14 font-medium">{theme.name}</span>
        <Badge tone="neutral">
          {theme.appearance === 'dark'
            ? t('settings.appearance.themesAppearanceDark')
            : t('settings.appearance.themesAppearanceLight')}
        </Badge>
      </div>
      <div className="flex overflow-hidden rounded border border-line" aria-hidden>
        {swatchTokens.map((token) => (
          <div key={token} className="h-6 flex-1" style={themeSwatchStyle(token, colors[token])} />
        ))}
      </div>
      <ContrastRows theme={theme} />
      <div className="flex items-center gap-2">
        {active ? (
          <Badge>{t('settings.appearance.themesActive')}</Badge>
        ) : (
          <Button size="sm" variant="secondary" onClick={() => onActivate(theme)}>
            {t('settings.appearance.themesActivate')}
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={() => onExport(theme)}>
          {t('settings.appearance.themesExport')}
        </Button>
        {onDelete !== undefined && (
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button size="sm" variant="ghost">
                {t('settings.appearance.themesDelete')}
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent
              impact={t('settings.appearance.themesDeleteImpact', { name: theme.name })}
            >
              <AlertDialogHeader>
                <AlertDialogTitle>
                  {t('settings.appearance.themesDeleteConfirm', { name: theme.name })}
                </AlertDialogTitle>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>{t('common:actions.cancel')}</AlertDialogCancel>
                <AlertDialogAction onClick={() => onDelete(theme)}>
                  {t('settings.appearance.themesDelete')}
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        )}
      </div>
    </div>
  );
}

export function AppearanceSettingsPage() {
  const { t, i18n } = useTranslation('shell');
  const themeMode = useUiStore((state) => state.themeMode);
  const setThemeMode = useUiStore((state) => state.setThemeMode);
  const themes = useCustomThemesStore((state) => state.themes);
  const customLoaded = useCustomThemesStore((state) => state.loaded);
  const importFromText = useCustomThemesStore((state) => state.importFromText);
  const removeTheme = useCustomThemesStore((state) => state.remove);
  const [importErrors, setImportErrors] = useState<readonly ThemeFieldError[] | null>(null);
  const [importParseDetail, setImportParseDetail] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // 自定义主题列表存放在全局设置里：进入本页时确保设置已加载再读列表
  useEffect(() => {
    void useSettingsStore
      .getState()
      .load()
      .then(() => {
        useCustomThemesStore.getState().load();
      })
      .catch(() => {
        /* 设置加载失败时画廊只显示内置主题，不打断页面 */
      });
  }, []);

  const activeCustom = currentActiveCustomTheme();
  const activeBuiltin = builtinThemeFor(currentResolvedTheme());

  const activateTheme = (theme: ThemeDefinition): void => {
    if (currentResolvedTheme() !== theme.appearance) {
      setThemeMode(theme.appearance as ThemeMode);
    }
    setActiveCustomTheme(theme);
  };
  const activateBuiltin = (theme: ThemeDefinition): void => {
    if (currentResolvedTheme() !== theme.appearance) {
      setThemeMode(theme.appearance as ThemeMode);
    }
    setActiveCustomTheme(null);
  };

  const handleImportFile = (file: File): void => {
    const reader = new FileReader();
    reader.onload = () => {
      const outcome = importFromText(String(reader.result ?? ''));
      if (outcome.ok) {
        setImportErrors(null);
        setImportParseDetail(null);
        activateTheme(outcome.theme);
      } else if (outcome.reason === 'parse') {
        setImportErrors(null);
        setImportParseDetail(outcome.detail);
      } else {
        setImportErrors(outcome.errors);
        setImportParseDetail(null);
      }
    };
    reader.readAsText(file);
  };

  const exportTheme = (theme: ThemeDefinition): void => {
    const blob = new Blob([JSON.stringify(theme, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = `${theme.id}.json`;
    anchor.click();
    URL.revokeObjectURL(url);
  };

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

        <div className="flex flex-col gap-3 border-t border-line pt-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex flex-col gap-0.5">
              <span className="text-14 font-medium">{t('settings.appearance.themesTitle')}</span>
              <span className="text-12 text-fg-subtle">{t('settings.appearance.themesHint')}</span>
            </div>
            <div>
              <input
                ref={fileInputRef}
                type="file"
                accept="application/json,.json"
                className="sr-only"
                aria-label={t('settings.appearance.themesImport')}
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  if (file !== undefined) {
                    handleImportFile(file);
                  }
                  event.target.value = '';
                }}
              />
              <Button
                size="sm"
                variant="secondary"
                onClick={() => {
                  fileInputRef.current?.click();
                }}
              >
                {t('settings.appearance.themesImport')}
              </Button>
            </div>
          </div>
          <p className="text-12 text-fg-subtle">{t('settings.appearance.themesImportHint')}</p>

          {importParseDetail !== null && (
            <p className="text-13 text-danger" role="alert">
              {t('settings.appearance.themesErrorTitle')}：
              {t('settings.appearance.themesErrorParse', { detail: importParseDetail })}
            </p>
          )}
          {importErrors !== null && importErrors.length > 0 && (
            <div className="text-13 text-danger" role="alert">
              <p>{t('settings.appearance.themesErrorTitle')}</p>
              <ul className="list-inside list-disc">
                {importErrors.map((error) => (
                  <li key={`${error.field}:${error.code}`}>
                    {t(
                      `settings.appearance.themesError${ERROR_CODE_SUFFIX[error.code] ?? 'NotAnObject'}`,
                      { value: error.value ?? '' },
                    )}
                  </li>
                ))}
              </ul>
            </div>
          )}

          <div className="grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-3">
            <div className="col-span-full text-12 font-medium text-fg-subtle">
              {t('settings.appearance.themesBuiltinGroup')}
            </div>
            {BUILTIN_THEMES.map((theme) => (
              <ThemeCard
                key={theme.id}
                theme={theme}
                active={activeCustom === null && theme.id === activeBuiltin.id}
                onActivate={activateBuiltin}
                onExport={exportTheme}
              />
            ))}
            {customLoaded && themes.length > 0 && (
              <>
                <div className="col-span-full text-12 font-medium text-fg-subtle">
                  {t('settings.appearance.themesCustomGroup')}
                </div>
                {themes.map((theme) => (
                  <ThemeCard
                    key={theme.id}
                    theme={theme}
                    active={activeCustom?.id === theme.id}
                    onActivate={activateTheme}
                    onExport={exportTheme}
                    onDelete={(target) => {
                      removeTheme(target.id);
                    }}
                  />
                ))}
              </>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
