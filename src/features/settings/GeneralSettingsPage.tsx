import { useEffect } from 'react';

import { useTranslation } from 'react-i18next';

import {
  DEBOUNCE_CHOICES_MS,
  DEFAULT_DEBOUNCE_MS,
  DENSITIES,
  DENSITY_KEY,
  useSettingsStore,
  WATCH_AUTO_REFRESH_KEY,
  WATCH_DEBOUNCE_KEY,
} from '@/stores/settingsStore';
import type { Density } from '@/stores/settingsStore';
import {
  PERFORMANCE_MODES,
  PERFORMANCE_MODE_KEY,
  parsePerformanceMode,
} from '@/lib/performanceMode';
import { Checkbox } from '@/ui/components/checkbox';
import { ErrorState } from '@/ui/components/error-state';
import { Skeleton } from '@/ui/components/skeleton';
import { ToggleGroup } from '@/ui/components/toggle-group';
import { useAppError } from '@/lib/errors';

/**
 * 通用设置页。
 *
 * T0.7 起它不是骨架页：界面密度这一项**真的会持久化到本地 SQLite**，
 * 重启应用后保持——这也是"存储层 + 迁移 + 设置服务"整条链路的验收点。
 *
 * 为什么把密度作为第一个落库的设置项：它满足两个条件——
 * ① 用户可感知（改了立刻看得出差别）；② 只在内容区生效（不涉及首帧同步问题）。
 * 主题则**刻意**留在 localStorage（理由见 `src/stores/settingsStore.ts` 顶部说明）。
 */
export function GeneralSettingsPage() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();

  const loaded = useSettingsStore((state) => state.loaded);
  const loading = useSettingsStore((state) => state.loading);
  const loadError = useSettingsStore((state) => state.loadError);
  const load = useSettingsStore((state) => state.load);
  const setJson = useSettingsStore((state) => state.setJson);

  useEffect(() => {
    // 进入设置页时按需加载（幂等：已加载过则不会重复请求）
    void load();
  }, [load]);

  // 逐项经 selector 订阅（selector 内调用 getJson）：值变才重渲染。
  // 曾踩的坑：订阅 getJson 函数引用（恒定）再在渲染里调用，控件点了不更新。
  const density = useSettingsStore((state) => state.getJson<Density>(DENSITY_KEY, 'comfortable'));
  const autoRefresh = useSettingsStore((state) =>
    state.getJson<boolean>(WATCH_AUTO_REFRESH_KEY, true),
  );
  const debounceMs = useSettingsStore((state) =>
    state.getJson<number>(WATCH_DEBOUNCE_KEY, DEFAULT_DEBOUNCE_MS),
  );
  // 性能模式（T2.9）：订阅原始值（字符串比较稳定，切换时触发重渲染），
  // 再经 parsePerformanceMode 解析（坏值回落 auto）——与 parseDensity 同一防御姿势
  const performanceModeRaw = useSettingsStore((state) => state.values[PERFORMANCE_MODE_KEY]);
  const performanceMode = parsePerformanceMode(performanceModeRaw);

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.settingsGeneral.title')}</h1>
        <p className="text-13 text-fg-muted">{t('pages.settingsGeneral.description')}</p>
      </header>

      {loadError !== null ? (
        <ErrorState
          title={t('settings.general.loadFailed')}
          hint={t('settings.general.loadFailedHint')}
          details={loadError}
          retryLabel={t('settings.general.retry')}
          onRetry={() => {
            void load({ force: true });
          }}
        />
      ) : null}

      <div className="flex flex-col gap-4 rounded-lg border border-line bg-surface p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex max-w-md flex-col gap-0.5">
            <span className="text-14 font-medium">{t('settings.general.densityLabel')}</span>
            <span className="text-12 text-fg-subtle">{t('settings.general.densityHint')}</span>
          </div>

          {loading && !loaded ? (
            <Skeleton className="h-7 w-40" />
          ) : (
            <ToggleGroup
              label={t('settings.general.densityLabel')}
              value={density}
              options={DENSITIES.map((option) => ({
                value: option,
                label: t(`settings.general.density.${option}`),
              }))}
              onValueChange={(next) => {
                // 失败时 store 已回滚，这里只负责把原因告诉用户
                void setJson(DENSITY_KEY, next).catch(show);
              }}
            />
          )}
        </div>

        {/* 自动刷新（T1.10）：文件监听的开与关，以及它的合并窗口 */}
        <div className="flex flex-col gap-3 border-t border-line pt-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex max-w-md flex-col gap-0.5">
              <span className="text-14 font-medium">{t('settings.general.watchLabel')}</span>
              <span className="text-12 text-fg-subtle">{t('settings.general.watchHint')}</span>
            </div>
            {loading && !loaded ? (
              <Skeleton className="h-5 w-24" />
            ) : (
              <Checkbox
                label={t('settings.general.watchAutoRefresh')}
                checked={autoRefresh}
                onCheckedChange={(next) => {
                  // 后端的监听类设置是"写完即生效"的：命令层会把正在跑的监听
                  // 按新值重启或停掉，因此这里不需要额外通知
                  void setJson(WATCH_AUTO_REFRESH_KEY, next === true).catch(show);
                }}
              />
            )}
          </div>

          <div className="flex flex-wrap items-center justify-between gap-3">
            <span className="max-w-md text-12 text-fg-subtle">
              {t('settings.general.watchDebounceHint')}
            </span>
            {loading && !loaded ? (
              <Skeleton className="h-7 w-40" />
            ) : (
              <ToggleGroup
                label={t('settings.general.watchDebounceLabel')}
                value={String(debounceMs)}
                options={DEBOUNCE_CHOICES_MS.map((value) => ({
                  value: String(value),
                  label: t('settings.general.watchDebounceValue', { ms: value }),
                }))}
                onValueChange={(next) => {
                  void setJson(WATCH_DEBOUNCE_KEY, Number(next)).catch(show);
                }}
              />
            )}
          </div>
        </div>

        {/* 性能模式（T2.9）：大仓库自动收紧渲染细节 */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-line pt-4">
          <div className="flex max-w-md flex-col gap-0.5">
            <span className="text-14 font-medium">{t('settings.general.perfLabel')}</span>
            <span className="text-12 text-fg-subtle">{t('settings.general.perfHint')}</span>
          </div>
          {loading && !loaded ? (
            <Skeleton className="h-7 w-40" />
          ) : (
            <ToggleGroup
              label={t('settings.general.perfLabel')}
              value={performanceMode}
              options={PERFORMANCE_MODES.map((option) => ({
                value: option,
                label: t(`settings.general.perf.${option}`),
              }))}
              onValueChange={(next) => {
                void setJson(PERFORMANCE_MODE_KEY, next).catch(show);
              }}
            />
          )}
        </div>

        <div className="flex flex-col gap-1 border-t border-line pt-4">
          <span className="text-14 font-medium">{t('settings.general.storageTitle')}</span>
          <p className="max-w-2xl text-12 text-fg-subtle">{t('settings.general.storageHint')}</p>
          <p className="max-w-2xl text-12 text-fg-subtle">{t('settings.general.themeNote')}</p>
        </div>
      </div>
    </section>
  );
}
