import { useCallback, useEffect, useState } from 'react';

import { useTranslation } from 'react-i18next';

import { clearDiagnosisHistory, readDiagnosisHistory } from '@/features/diagnostics/history';
import {
  DEFAULT_TERMINAL_FONT_SIZE,
  DEFAULT_TERMINAL_LINE_HEIGHT,
  TERMINAL_FONT_SIZE_KEY,
  TERMINAL_LINE_HEIGHT_KEY,
  TERMINAL_SAFETY_AUTO_SNAPSHOT_KEY,
  TERMINAL_SAFETY_ENABLED_KEY,
  TERMINAL_SAFETY_LEVEL_KEY,
  TERMINAL_SAFETY_LEVELS,
  useSettingsStore,
} from '@/stores/settingsStore';
import type { TerminalSafetyLevel } from '@/stores/settingsStore';
import { Button } from '@/ui/components/button';
import { Checkbox } from '@/ui/components/checkbox';
import { Input } from '@/ui/components/input';
import { RadioGroup } from '@/ui/components/radio-group';
import { ToggleGroup } from '@/ui/components/toggle-group';
import { formatDateTime } from '@/lib/i18n/intl';

/** 数值输入的公共约束：非法输入不改存储，恢复当前值。 */
function NumberSetting({
  label,
  value,
  min,
  max,
  step,
  onChange,
}: {
  readonly label: string;
  readonly value: number;
  readonly min: number;
  readonly max: number;
  readonly step: number;
  readonly onChange: (value: number) => void;
}) {
  return (
    <label className="flex items-center gap-2 text-13">
      <span className="text-fg-muted">{label}</span>
      <Input
        srLabel={label}
        type="number"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => {
          const parsed = Number(event.target.value);
          if (Number.isFinite(parsed) && parsed >= min && parsed <= max) {
            onChange(parsed);
          }
        }}
        className="w-20"
      />
    </label>
  );
}

/**
 * 终端设置（T5.3）：安全衔接（提示开关 / 级别 / 补偿快照）与外观（字号 / 行高）。
 *
 * 级别语义（与 danger 识别器、PTY-SPIKE 的能力边界一致）：
 * - **hint（默认）**：非阻塞提示条 + 强制登记 + 可选补偿快照，命令照常执行；
 * - **confirm**：高危命令在执行前需要确认（取消时发 Ctrl+C 取消 shell 侧的行）；
 * - "始终记录级"不受开关与级别影响：它由后端 `term_report_command` 强制执行。
 *
 * 能力边界（界面不得宣称相反的话）：终端中的任何操作都无法保证可回滚，
 * 补偿快照只是事后的对照点。
 */
export function TerminalSettingsPage() {
  const { t } = useTranslation('shell');
  const setJson = useSettingsStore((state) => state.setJson);
  const getJson = useSettingsStore((state) => state.getJson);

  const safetyEnabled = getJson<boolean>(TERMINAL_SAFETY_ENABLED_KEY, true);
  const safetyLevel = getJson<TerminalSafetyLevel>(TERMINAL_SAFETY_LEVEL_KEY, 'hint');
  const autoSnapshot = getJson<boolean>(TERMINAL_SAFETY_AUTO_SNAPSHOT_KEY, true);
  const fontSize = getJson<number>(TERMINAL_FONT_SIZE_KEY, DEFAULT_TERMINAL_FONT_SIZE);
  const lineHeight = getJson<number>(TERMINAL_LINE_HEIGHT_KEY, DEFAULT_TERMINAL_LINE_HEIGHT);

  // 设置项在页面加载时可能尚未就绪：触发一次全局加载（幂等）
  useEffect(() => {
    void useSettingsStore.getState().load({ scope: 'global', force: false });
  }, []);

  const [history, setHistory] = useState(() => readDiagnosisHistory());
  const refreshHistory = useCallback(() => setHistory(readDiagnosisHistory()), []);

  return (
    <div className="flex max-w-2xl flex-col gap-6">
      <section className="flex flex-col gap-3">
        <h2 className="text-16 font-semibold">{t('settings.terminal.historyTitle')}</h2>
        {history.length === 0 ? (
          <p className="text-fg-subtle text-12">{t('settings.terminal.historyEmpty')}</p>
        ) : (
          <>
            <ul className="flex flex-col gap-1.5">
              {history
                .slice()
                .reverse()
                .slice(0, 20)
                .map((record) => (
                  <li
                    key={`${record.at}-${record.ruleId ?? 'none'}`}
                    className="border-line bg-surface flex flex-col rounded-md border px-3 py-2"
                  >
                    <span className="text-12 font-medium">
                      {record.ruleId === null
                        ? t('settings.terminal.historyNoRule')
                        : t(`diag.${record.ruleId}.title`)}
                    </span>
                    <span className="text-fg-subtle font-mono text-11">
                      {formatDateTime(record.at)}
                    </span>
                  </li>
                ))}
            </ul>
            <div>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => {
                  clearDiagnosisHistory();
                  refreshHistory();
                }}
              >
                {t('settings.terminal.historyClear')}
              </Button>
            </div>
          </>
        )}
      </section>

      <section className="flex flex-col gap-3">
        <h2 className="text-16 font-semibold">{t('settings.terminal.safetyTitle')}</h2>

        <label className="flex items-center gap-2 text-13">
          <Checkbox
            checked={safetyEnabled}
            onCheckedChange={(checked) => {
              void setJson(TERMINAL_SAFETY_ENABLED_KEY, checked === true);
            }}
          />
          {t('settings.terminal.safetyEnabled')}
        </label>

        <div className="flex flex-col gap-2">
          <span className="text-fg-muted text-13">{t('settings.terminal.safetyLevel')}</span>
          <RadioGroup
            label={t('settings.terminal.safetyLevel')}
            value={safetyLevel}
            onValueChange={(value) => {
              if ((TERMINAL_SAFETY_LEVELS as readonly string[]).includes(value)) {
                void setJson(TERMINAL_SAFETY_LEVEL_KEY, value);
              }
            }}
            options={TERMINAL_SAFETY_LEVELS.map((level) => ({
              value: level,
              label: t(`settings.terminal.level.${level}`),
            }))}
          />
          <p className="text-fg-subtle text-12">
            {t(`settings.terminal.levelHint.${safetyLevel}`)}
          </p>
        </div>

        <label className="flex items-center gap-2 text-13">
          <Checkbox
            checked={autoSnapshot}
            onCheckedChange={(checked) => {
              void setJson(TERMINAL_SAFETY_AUTO_SNAPSHOT_KEY, checked === true);
            }}
          />
          {t('settings.terminal.autoSnapshot')}
        </label>

        <p className="border-line bg-surface-sunken rounded-md border p-3 text-12 leading-relaxed">
          {t('settings.terminal.boundaryNote')}
        </p>
      </section>

      <section className="flex flex-col gap-3">
        <h2 className="text-16 font-semibold">{t('settings.terminal.appearanceTitle')}</h2>
        <ToggleGroup
          label={t('settings.terminal.fontSize')}
          value={String(fontSize)}
          onValueChange={(value) => {
            const parsed = Number(value);
            if (Number.isFinite(parsed)) {
              void setJson(TERMINAL_FONT_SIZE_KEY, parsed);
            }
          }}
          options={[12, 13, 14, 16].map((size) => ({
            value: String(size),
            label: `${size}px`,
          }))}
        />
        <NumberSetting
          label={t('settings.terminal.fontSize')}
          value={fontSize}
          min={9}
          max={24}
          step={1}
          onChange={(value) => {
            void setJson(TERMINAL_FONT_SIZE_KEY, value);
          }}
        />
        <NumberSetting
          label={t('settings.terminal.lineHeight')}
          value={lineHeight}
          min={1}
          max={2}
          step={0.05}
          onChange={(value) => {
            void setJson(TERMINAL_LINE_HEIGHT_KEY, value);
          }}
        />
      </section>
    </div>
  );
}
