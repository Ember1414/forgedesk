// i18n-ignore-file
// 本文件是开发专用页面（路由只在 dev 构建注册），文案面向开发者，
// 因此整文件豁免 i18n:lint；正式界面一律走 i18n key。
import { useState } from 'react';
import type { ReactNode } from 'react';

import { useTranslation } from 'react-i18next';

import { THEME_MODES } from '@/app/theme';
import type { ThemeMode } from '@/app/theme';
import { VersionBadge } from '@/features/system/VersionBadge';
import { cn } from '@/lib/utils';
import { ToggleGroup } from '@/ui/components/toggle-group';
import { useUiStore } from '@/stores/uiStore';

/**
 * 设计系统预览页（开发专用，路由 /__dev__/design 只在 dev 构建注册）。
 *
 * 作用：把所有 token 以**实际渲染效果 + 运行时解析值**的形式展示出来，
 * 这样调色或改字号时能立刻看到影响，而不是靠读 CSS 猜。
 * 页面上不写任何硬编码色值，全部通过语义工具类呈现（见 src/ui/tokens.css 的约定）。
 *
 * 主题来源：T0.4 起统一走 uiStore（此前本页自行读写 localStorage，
 * 与设置页会出现两个真相源）。页面自身的说明文案是开发者文案、
 * 且页面不进入生产构建，因此不走 i18n；T6.7 的 i18n:lint 需要把它列入白名单。
 */

interface TokenRow {
  readonly token: string;
  readonly className: string;
  readonly usage: string;
}

const SURFACE_TOKENS: readonly TokenRow[] = [
  { token: '--fd-canvas', className: 'bg-canvas', usage: '页面底色' },
  { token: '--fd-surface', className: 'bg-surface', usage: '面板 / 卡片' },
  { token: '--fd-surface-raised', className: 'bg-surface-raised', usage: '浮起面板（弹层）' },
  { token: '--fd-surface-sunken', className: 'bg-surface-sunken', usage: '凹陷区域（代码块底）' },
  { token: '--fd-line', className: 'bg-line', usage: '分隔线' },
  { token: '--fd-line-strong', className: 'bg-line-strong', usage: '强调分隔线' },
];

const ACCENT_TOKENS: readonly TokenRow[] = [
  { token: '--fd-brand', className: 'bg-brand', usage: '主强调 / 主按钮' },
  { token: '--fd-brand-subtle', className: 'bg-brand-subtle', usage: '品牌浅底' },
  { token: '--fd-spark', className: 'bg-spark', usage: '辅助强调' },
  { token: '--fd-success', className: 'bg-success', usage: '成功' },
  { token: '--fd-warning', className: 'bg-warning', usage: '警告' },
  { token: '--fd-danger', className: 'bg-danger', usage: '危险' },
  { token: '--fd-info', className: 'bg-info', usage: '信息' },
];

const TEXT_TOKENS: readonly TokenRow[] = [
  { token: '--fd-fg', className: 'text-fg', usage: '正文' },
  { token: '--fd-fg-muted', className: 'text-fg-muted', usage: '次要文字' },
  { token: '--fd-fg-subtle', className: 'text-fg-subtle', usage: '弱化文字' },
  { token: '--fd-brand', className: 'text-brand', usage: '链接 / 品牌文字' },
  { token: '--fd-danger', className: 'text-danger', usage: '错误文字' },
  { token: '--fd-success', className: 'text-success', usage: '成功文字' },
];

const ALL_TOKENS: readonly string[] = [...SURFACE_TOKENS, ...ACCENT_TOKENS, ...TEXT_TOKENS].map(
  (row) => row.token,
);

const TYPE_SCALE = [
  { className: 'text-32', label: '32 / 40', usage: '页面主标题' },
  { className: 'text-24', label: '24 / 32', usage: '区块标题' },
  { className: 'text-20', label: '20 / 28', usage: '卡片标题' },
  { className: 'text-16', label: '16 / 24', usage: '强调正文' },
  { className: 'text-14', label: '14 / 20', usage: '正文（默认）' },
  { className: 'text-13', label: '13 / 18', usage: '密集列表' },
  { className: 'text-12', label: '12 / 16', usage: '辅助说明 / 徽标' },
] as const;

const RADIUS_SCALE = [
  { className: 'rounded-xs', label: 'xs · 4px' },
  { className: 'rounded-sm', label: 'sm · 6px' },
  { className: 'rounded-md', label: 'md · 8px' },
  { className: 'rounded-lg', label: 'lg · 12px' },
  { className: 'rounded-xl', label: 'xl · 16px' },
] as const;

const SHADOW_SCALE = [
  { className: 'shadow-sm', label: 'sm · 微浮起' },
  { className: 'shadow-md', label: 'md · 卡片' },
  { className: 'shadow-lg', label: 'lg · 弹层' },
] as const;

const SPACING_SCALE = [
  { className: 'p-1', label: '1 · 4px' },
  { className: 'p-2', label: '2 · 8px' },
  { className: 'p-3', label: '3 · 12px' },
  { className: 'p-4', label: '4 · 16px' },
  { className: 'p-6', label: '6 · 24px' },
  { className: 'p-8', label: '8 · 32px' },
] as const;

/**
 * 读取 :root 上解析后的 token 实际值。
 *
 * 为什么不用 useEffect + setState：那属于"在 effect 中同步 setState"，
 * 会触发级联渲染（React 官方明确不推荐，且被 react-hooks/set-state-in-effect 拦截）。
 * 读计算样式本质是"与外部系统同步"，因此在首次渲染与切换主题时显式读取一次即可。
 */
function readTokenValues(): Record<string, string> {
  const style = getComputedStyle(document.documentElement);
  const values: Record<string, string> = {};
  for (const token of ALL_TOKENS) {
    values[token] = style.getPropertyValue(token).trim();
  }
  return values;
}

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="rounded-lg border border-line bg-surface p-6 shadow-sm">
      <h2 className="text-20 font-semibold">{title}</h2>
      {description !== undefined ? (
        <p className="mt-1 text-13 text-fg-muted">{description}</p>
      ) : null}
      <div className="mt-4">{children}</div>
    </section>
  );
}

function TokenGrid({
  rows,
  values,
  mode,
}: {
  rows: readonly TokenRow[];
  values: Readonly<Record<string, string>>;
  mode: 'swatch' | 'text';
}) {
  return (
    <ul className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {rows.map((row) => (
        <li key={row.token} className="flex items-center gap-3">
          {mode === 'swatch' ? (
            <span
              aria-hidden="true"
              className={cn('size-10 shrink-0 rounded-md border border-line-strong', row.className)}
            />
          ) : (
            <span
              aria-hidden="true"
              className={cn(
                'flex size-10 shrink-0 items-center justify-center rounded-md bg-surface-sunken text-16 font-semibold',
                row.className,
              )}
            >
              文
            </span>
          )}
          <div className="min-w-0">
            <div className="truncate font-mono text-12 text-fg">{row.token}</div>
            <div className="truncate text-12 text-fg-muted">
              {row.usage}
              <span className="ml-2 font-mono text-fg-subtle">{values[row.token] ?? '—'}</span>
            </div>
          </div>
        </li>
      ))}
    </ul>
  );
}

export function DesignSystemPage() {
  const { t } = useTranslation('common');
  const themeMode = useUiStore((state) => state.themeMode);
  const setThemeMode = useUiStore((state) => state.setThemeMode);
  const [values, setValues] = useState<Record<string, string>>(() => readTokenValues());

  function changeThemeMode(next: ThemeMode): void {
    // setThemeMode 同步写入 <html data-theme>，因此紧接着就能读到解析后的 token 值
    setThemeMode(next);
    setValues(readTokenValues());
  }

  return (
    <main className="h-full overflow-y-auto bg-canvas">
      <div className="mx-auto flex max-w-5xl flex-col gap-6 p-8">
        <header className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h1 className="text-32 font-semibold tracking-tight">ForgeDesk 设计系统</h1>
            <p className="mt-1 text-14 text-fg-muted">
              T0.3 预览页 · 所有色值来自 <span className="font-mono">src/ui/tokens.css</span> ·
              对比度由 <span className="font-mono">scripts/design/check-contrast.mjs</span> 校验
            </p>
          </div>
          <ToggleGroup
            label="主题切换"
            value={themeMode}
            options={THEME_MODES.map((mode) => ({ value: mode, label: t(`theme.${mode}`) }))}
            onValueChange={(next) => {
              changeThemeMode(next as ThemeMode);
            }}
            className="p-1"
          />
        </header>

        <Section title="背景与描边" description="表层层级：canvas → surface → surface-raised">
          <TokenGrid rows={SURFACE_TOKENS} values={values} mode="swatch" />
        </Section>

        <Section title="品牌与语义状态" description="语义色只用于传达状态，不用于装饰">
          <TokenGrid rows={ACCENT_TOKENS} values={values} mode="swatch" />
          <div className="mt-4 flex flex-wrap gap-2">
            <span className="rounded-sm bg-success px-2 py-1 text-12 font-medium text-fg-inverted">
              ✓ 已通过
            </span>
            <span className="rounded-sm bg-warning px-2 py-1 text-12 font-medium text-fg-inverted">
              ! 需注意
            </span>
            <span className="rounded-sm bg-danger px-2 py-1 text-12 font-medium text-fg-inverted">
              ✕ 失败
            </span>
            <span className="rounded-sm bg-info px-2 py-1 text-12 font-medium text-fg-inverted">
              i 提示
            </span>
          </div>
          <p className="mt-3 text-12 text-fg-subtle">
            状态标识同时使用图标与文字，不依赖颜色单独传达信息（无障碍要求）。
          </p>
        </Section>

        <Section title="文字颜色" description="正文对比度 ≥ 4.5:1（WCAG AA）">
          <TokenGrid rows={TEXT_TOKENS} values={values} mode="text" />
        </Section>

        <Section title="字号阶" description="12 / 13 / 14 / 16 / 20 / 24 / 32，行高已配对">
          <ul className="flex flex-col gap-2">
            {TYPE_SCALE.map((row) => (
              <li key={row.className} className="flex items-baseline gap-4">
                <span className={cn('shrink-0 font-semibold', row.className)}>看得见的 Git</span>
                <span className="font-mono text-12 text-fg-subtle">
                  {row.label} · {row.usage}
                </span>
              </li>
            ))}
          </ul>
        </Section>

        <Section title="圆角" description="卡片 8 / 面板 12 / 弹层 16">
          <ul className="flex flex-wrap gap-4">
            {RADIUS_SCALE.map((row) => (
              <li key={row.className} className="flex flex-col items-center gap-2">
                <span
                  className={cn(
                    'size-16 border border-line-strong bg-surface-sunken',
                    row.className,
                  )}
                />
                <span className="font-mono text-12 text-fg-muted">{row.label}</span>
              </li>
            ))}
          </ul>
        </Section>

        <Section title="阴影" description="3 级：微浮起 / 卡片 / 弹层">
          <ul className="flex flex-wrap gap-6">
            {SHADOW_SCALE.map((row) => (
              <li key={row.className} className="flex flex-col items-center gap-2">
                <span className={cn('size-20 rounded-lg bg-surface', row.className)} />
                <span className="font-mono text-12 text-fg-muted">{row.label}</span>
              </li>
            ))}
          </ul>
        </Section>

        <Section title="间距刻度" description="沿用 Tailwind 4px 基准：4 / 8 / 12 / 16 / 24 / 32">
          <ul className="flex flex-wrap items-end gap-4">
            {SPACING_SCALE.map((row) => (
              <li key={row.className} className="flex flex-col items-center gap-2">
                <span className={cn('bg-brand-subtle', row.className)}>
                  <span className="block size-2 bg-brand" />
                </span>
                <span className="font-mono text-12 text-fg-muted">{row.label}</span>
              </li>
            ))}
          </ul>
        </Section>

        <Section title="动效时长" description="120 / 200 / 320ms，且遵守 prefers-reduced-motion">
          <ul className="flex flex-wrap gap-4">
            {[
              { className: 'w-32', label: 'fast · 120ms' },
              { className: 'w-40', label: 'base · 200ms' },
              { className: 'w-48', label: 'slow · 320ms' },
            ].map((row) => (
              <li
                key={row.label}
                className="group flex flex-col items-center gap-2"
                title={row.label}
              >
                <span
                  className={cn(
                    'h-8 rounded-md bg-brand transition-all duration-300 ease-out group-hover:bg-spark',
                    row.className,
                  )}
                />
                <span className="font-mono text-12 text-fg-muted">{row.label}</span>
              </li>
            ))}
          </ul>
          <p className="mt-3 text-12 text-fg-subtle">悬停上方色块可观察过渡效果。</p>
        </Section>

        <Section
          title="运行时与 IPC 通路"
          description="验证「前端 → Tauri 命令（app_version）→ 前端」链路与 TanStack Query 接入"
        >
          <VersionBadge />
        </Section>
      </div>
    </main>
  );
}
