import type { HTMLAttributes } from 'react';

import { cn } from '@/lib/utils';

/**
 * 骨架占位。
 *
 * 约定：骨架只用于"已经知道大概形状"的加载（表格行、卡片、diff 行），
 * 并且必须与最终内容**尺寸接近**，否则加载完成时会发生跳动（CLS），
 * 对长时间盯屏的用户来说这种跳动比多等 100ms 更烦躁。
 *
 * 纯装饰元素，因此统一 aria-hidden；加载状态本身由外层用 aria-busy / Skeleton 的
 * `srLabel` 表达，避免读屏软件逐个朗读占位块。
 */
export interface SkeletonProps extends HTMLAttributes<HTMLDivElement> {
  /** 骨架形状：单行文本 / 多行段落 / 卡片块。 */
  readonly variant?: 'line' | 'block' | 'circle';
}

export function Skeleton({ className, variant = 'line', ...props }: SkeletonProps) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        'animate-pulse bg-surface-sunken',
        variant === 'line' && 'h-3 w-full rounded-xs',
        variant === 'block' && 'h-20 w-full rounded-md',
        variant === 'circle' && 'size-8 rounded-full',
        className,
      )}
      {...props}
    />
  );
}

/** 多行文本骨架（最后一行短一些，读起来更像一段话）。 */
export function SkeletonText({ lines = 3, className }: { lines?: number; className?: string }) {
  return (
    <div className={cn('flex flex-col gap-2', className)}>
      {Array.from({ length: lines }, (_, index) => (
        <Skeleton key={index} className={index === lines - 1 ? 'w-2/3' : undefined} />
      ))}
    </div>
  );
}
