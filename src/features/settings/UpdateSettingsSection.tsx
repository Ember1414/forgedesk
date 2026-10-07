import { useEffect } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import {
  DEFAULT_UPDATE_AUTO_CHECK,
  UPDATE_AUTO_CHECK_KEY,
  UPDATE_SKIPPED_VERSION_KEY,
  useSettingsStore,
} from '@/stores/settingsStore';
import { Button } from '@/ui/components/button';
import { Checkbox } from '@/ui/components/checkbox';

/**
 * 「更新」设置区块（T7.1）——挂在高级设置页。
 *
 * 两件事：自动检查的开关，以及"看一眼 / 取消"被跳过的版本。
 *
 * # 为什么把"已跳过的版本"显示出来
 *
 * 跳过后横幅不再出现，用户很容易**忘记自己跳过过**，之后又把"怎么不提示更新了"当成故障。
 * 把当前跳过的版本号摆在设置里并给一个「不再跳过」，是把这条状态变成可发现、可撤销的。
 *
 * # 为什么选择器里调 `getJson`
 *
 * zustand 只在该选择器结果变化时重渲染；直接取函数引用不会随值变化重渲染，
 * 开关就会出现"点了没反应"的假象（值其实已写入）。`getJson` 返回原始值，正好适合做选择器。
 */
export function UpdateSettingsSection() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();

  // 进入设置页时按需加载（幂等：App 启动时已加载过则不会再请求）
  useEffect(() => {
    void useSettingsStore.getState().load({ scope: 'global', force: false });
  }, []);

  const setJson = useSettingsStore((state) => state.setJson);
  const autoCheck = useSettingsStore((state) =>
    state.getJson<boolean>(UPDATE_AUTO_CHECK_KEY, DEFAULT_UPDATE_AUTO_CHECK),
  );
  const skippedVersion = useSettingsStore((state) =>
    state.getJson<string | null>(UPDATE_SKIPPED_VERSION_KEY, null),
  );

  return (
    <div className="border-line bg-surface flex flex-col gap-3 rounded-lg border p-4">
      <h2 className="text-14 font-medium">{t('settings.update.title')}</h2>

      <label className="flex flex-col gap-0.5 text-13">
        <span className="flex items-center gap-2">
          <Checkbox
            checked={autoCheck}
            // 显式无障碍名称：外层 label 还包含提示文字，靠 label 推断会读到一整句
            aria-label={t('settings.update.autoCheck')}
            onCheckedChange={(checked) => {
              void setJson(UPDATE_AUTO_CHECK_KEY, checked === true).catch(show);
            }}
          />
          {t('settings.update.autoCheck')}
        </span>
        <span className="text-fg-subtle pl-6 text-12">{t('settings.update.autoCheckHint')}</span>
      </label>

      <div className="flex flex-wrap items-center justify-between gap-2 text-13">
        <span>
          {t('settings.update.skippedLabel')}
          {': '}
          <span className="font-mono text-12">
            {skippedVersion ?? t('settings.update.skippedNone')}
          </span>
        </span>
        {skippedVersion !== null ? (
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              void setJson(UPDATE_SKIPPED_VERSION_KEY, null).catch(show);
            }}
          >
            {t('settings.update.skippedClear')}
          </Button>
        ) : null}
      </div>

      <p className="text-fg-subtle text-12">{t('settings.update.channelHint')}</p>
    </div>
  );
}
