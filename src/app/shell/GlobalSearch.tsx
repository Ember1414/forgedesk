import { useEffect, useRef, useState } from 'react';

import { Search } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { cn } from '@/lib/utils';

/**
 * 顶栏的全局搜索入口。
 *
 * 现状：本框只提供入口与键盘行为，**尚未**接通真正的跨仓库搜索；命令面板
 * 已在 M5 落地（Ctrl/Cmd+Shift+P），全局搜索仍待后续里程碑实现。因此这里
 * 不假装能搜（不渲染假结果），只在获得焦点时如实说明当前能力（见
 * `titleBar.search.hint`）。
 *
 * 键盘行为：
 *   - Ctrl/Cmd + K 聚焦搜索框（桌面应用的高频操作，先建好习惯）
 *   - Esc 清空并移出焦点
 */
export function GlobalSearch() {
  const { t } = useTranslation('shell');
  const inputRef = useRef<HTMLInputElement>(null);
  const [focused, setFocused] = useState(false);
  const [value, setValue] = useState('');

  useEffect(() => {
    function handleShortcut(event: KeyboardEvent): void {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
      }
    }
    window.addEventListener('keydown', handleShortcut);
    return () => {
      window.removeEventListener('keydown', handleShortcut);
    };
  }, []);

  return (
    <div className="relative w-full max-w-sm">
      <Search
        aria-hidden="true"
        className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-fg-subtle"
      />
      <input
        ref={inputRef}
        type="search"
        value={value}
        aria-label={t('titleBar.search.label')}
        placeholder={t('titleBar.search.placeholder')}
        onChange={(event) => {
          setValue(event.target.value);
        }}
        onFocus={() => {
          setFocused(true);
        }}
        onBlur={() => {
          setFocused(false);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            setValue('');
            event.currentTarget.blur();
          }
        }}
        className={cn(
          'fd-transition h-8 w-full rounded-md border border-line bg-surface pl-8 pr-16 text-13',
          'placeholder:text-fg-subtle hover:border-line-strong focus:border-brand',
        )}
      />
      <kbd
        aria-hidden="true"
        className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 rounded-xs border border-line bg-surface-sunken px-1.5 py-0.5 font-mono text-12 text-fg-subtle"
      >
        {t('titleBar.search.shortcut')}
      </kbd>

      {focused ? (
        <p
          role="status"
          className="absolute left-0 top-10 z-40 w-full rounded-md border border-line bg-surface-raised px-3 py-2 text-12 text-fg-muted shadow-md"
        >
          {t('titleBar.search.hint')}
        </p>
      ) : null}
    </div>
  );
}
