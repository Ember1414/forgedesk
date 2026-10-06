/**
 * 布局设置页（T5.10）：预设、树宽、导入导出、恢复默认。
 *
 * 持久化在 settings 表（`ui.layout`，JSON）；导入 = 从剪贴板读 JSON，
 * 形状校验失败给出明确错误（不静默回退——导出的坏数据用户需要知道）。
 * 恢复默认 = 重置为 DEFAULT_LAYOUT 并持久化。
 */
import { useEffect, useState } from 'react';

import { useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { pushToast } from '@/stores/toastStore';
import {
  DEFAULT_LAYOUT,
  LAYOUT_KEY,
  LAYOUT_PRESETS,
  parseLayout,
  useLayoutStore,
} from '@/stores/layoutStore';
import type { LayoutState } from '@/stores/layoutStore';
import { settingsGet, settingsSet } from '@/lib/ipc';
import { Button } from '@/ui/components/button';
import { Slider } from '@/ui/components/slider';
import { ToggleGroup } from '@/ui/components/toggle-group';

export function LayoutSettingsPage() {
  const { t } = useTranslation('shell');
  const layout = useLayoutStore((state) => state.layout);
  const corrupted = useLayoutStore((state) => state.corrupted);
  const markCorruptionHandled = useLayoutStore((state) => state.markCorruptionHandled);
  const setPreset = useLayoutStore((state) => state.setPreset);
  const setTreeWidth = useLayoutStore((state) => state.setTreeWidth);
  const resetToDefault = useLayoutStore((state) => state.resetToDefault);
  const hydrate = useLayoutStore((state) => state.hydrate);
  const queryClient = useQueryClient();
  const [exported, setExported] = useState(false);

  // 载入持久化值（损坏 → toast 一次性提示）
  useEffect(() => {
    void settingsGet('global', LAYOUT_KEY).then((raw) => {
      hydrate(raw ?? undefined);
      if (corrupted) {
        pushToast({ tone: 'warning', title: t('settings.layout.corruptedToast') });
        markCorruptionHandled();
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const persist = async (next: LayoutState) => {
    await settingsSet('global', LAYOUT_KEY, JSON.stringify(next));
    hydrate(JSON.stringify(next));
    void queryClient.invalidateQueries();
  };

  return (
    <div className="flex max-w-2xl flex-col gap-5">
      <h1 className="text-20 font-semibold tracking-tight">{t('pages.settingsLayout.title')}</h1>
      {corrupted ? (
        <p className="text-warning text-12">{t('settings.layout.corruptedInline')}</p>
      ) : null}

      <section className="flex flex-col gap-2">
        <h2 className="text-16 font-semibold">{t('settings.layout.presetTitle')}</h2>
        <ToggleGroup
          label={t('settings.layout.presetTitle')}
          value={layout.preset}
          onValueChange={(value) => {
            const preset = LAYOUT_PRESETS.find((name) => name === value);
            if (preset !== undefined) {
              setPreset(preset);
              void persist({ ...layout, preset });
            }
          }}
          options={LAYOUT_PRESETS.map((preset) => ({
            value: preset,
            label: t(`settings.layout.preset.${preset}`),
          }))}
        />
        <p className="text-fg-subtle text-12">{t(`settings.layout.presetHint.${layout.preset}`)}</p>
      </section>

      <section className="flex flex-col gap-2">
        <h2 className="text-16 font-semibold">{t('settings.layout.treeWidthTitle')}</h2>
        <Slider
          label={t('settings.layout.treeWidthTitle')}
          min={200}
          max={480}
          step={8}
          value={[layout.treeWidth]}
          onValueChange={(value) => setTreeWidth(value[0] ?? layout.treeWidth)}
          onValueCommit={(value) => {
            if (value[0] !== undefined) {
              void persist({ ...layout, treeWidth: value[0] });
            }
          }}
          formatValue={(value) => `${value}px`}
        />
        <span className="text-fg-subtle font-mono text-12">{layout.treeWidth}px</span>
      </section>

      <section className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="secondary"
          onClick={() => {
            void navigator.clipboard.writeText(JSON.stringify(layout)).then(() => {
              setExported(true);
              setTimeout(() => setExported(false), 2000);
            });
          }}
        >
          {exported ? t('settings.layout.exported') : t('settings.layout.export')}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => {
            void navigator.clipboard
              .readText()
              .then((raw) => {
                const parsed = parseLayout(raw);
                if (parsed.error !== null) {
                  pushToast({ tone: 'danger', title: t('settings.layout.importInvalid') });
                  return;
                }
                void persist(parsed.layout);
                pushToast({ tone: 'success', title: t('settings.layout.imported') });
              })
              .catch(() => {
                pushToast({ tone: 'danger', title: t('settings.layout.importInvalid') });
              });
          }}
        >
          {t('settings.layout.import')}
        </Button>
        <Button
          size="sm"
          variant="danger"
          onClick={() => {
            resetToDefault();
            void persist({ ...DEFAULT_LAYOUT });
          }}
        >
          {t('settings.layout.reset')}
        </Button>
      </section>
    </div>
  );
}
