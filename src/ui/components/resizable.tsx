import { useRef } from 'react';
import type { KeyboardEvent, PointerEvent, ReactNode } from 'react';

import { cn } from '@/lib/utils';

/**
 * 可调整尺寸的容器（拖拽某一条边）。
 *
 * 为什么把手做成 `role="separator"`：这正是 WAI-ARIA 的"窗口分隔条"模式——
 * 读屏软件会播报"分隔条，可调整"，并且通过 aria-valuenow/min/max 报出当前尺寸；
 * 纯 div 的拖拽把手对键盘用户完全不可用。
 *
 * 键盘操作（与直觉一致）：方向键按 step 调整，Shift + 方向键按 4×step 粗调。
 * 拖动实现用 Pointer Capture，指针移出元素后事件仍然回到本元素，
 * 不依赖 window 级监听，组件卸载时也就不会有残留监听器。
 */
export type ResizeEdge = 'start' | 'end';

export interface ResizableProps {
  /** 手柄所在的边：'end' 表示右/下边，"尺寸"指本容器的宽/高。 */
  readonly edge?: ResizeEdge;
  readonly orientation?: 'horizontal' | 'vertical';
  /** 当前尺寸（px）。 */
  readonly size: number;
  readonly minSize?: number;
  readonly maxSize?: number;
  /** 每次方向键调整的步长（px）。 */
  readonly step?: number;
  onSizeChange(size: number): void;
  /** 手柄的无障碍名称（必填，来自上层 i18n，例如"调整详情面板宽度"）。 */
  readonly handleLabel: string;
  readonly children: ReactNode;
  readonly className?: string;
}

export function Resizable({
  edge = 'end',
  orientation = 'horizontal',
  size,
  minSize = 160,
  maxSize = 720,
  step = 16,
  onSizeChange,
  handleLabel,
  children,
  className,
}: ResizableProps) {
  const dragState = useRef<{ startPosition: number; startSize: number } | null>(null);

  const isHorizontal = orientation === 'horizontal';
  const min = Math.min(minSize, maxSize);
  const clamp = (value: number): number => Math.min(maxSize, Math.max(min, value));

  function handlePointerDown(event: PointerEvent<HTMLDivElement>): void {
    event.preventDefault();
    dragState.current = {
      startPosition: isHorizontal ? event.clientX : event.clientY,
      startSize: size,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function handlePointerMove(event: PointerEvent<HTMLDivElement>): void {
    const state = dragState.current;
    if (state === null) {
      return;
    }
    const current = isHorizontal ? event.clientX : event.clientY;
    const delta = current - state.startPosition;
    // edge === 'start' 时，把手在左/上边：向右拖动应当**变小**
    onSizeChange(clamp(state.startSize + (edge === 'end' ? delta : -delta)));
  }

  function handlePointerUp(event: PointerEvent<HTMLDivElement>): void {
    dragState.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>): void {
    const decreaseKey = isHorizontal ? 'ArrowLeft' : 'ArrowUp';
    const increaseKey = isHorizontal ? 'ArrowRight' : 'ArrowDown';

    if (event.key !== decreaseKey && event.key !== increaseKey) {
      return;
    }
    event.preventDefault();
    const magnitude = event.shiftKey ? step * 4 : step;
    // 向左/上：'end' 边把手意味着容器变小，'start' 边把手意味着容器变大的反向
    const towardsStart = event.key === decreaseKey;
    const grows = edge === 'end' ? !towardsStart : towardsStart;
    onSizeChange(clamp(size + (grows ? magnitude : -magnitude)));
  }

  return (
    <div
      className={cn(
        'relative flex min-h-0 min-w-0',
        isHorizontal ? 'flex-row' : 'flex-col',
        className,
      )}
      style={isHorizontal ? { width: size } : { height: size }}
    >
      {edge === 'start' ? (
        <ResizeHandle
          handleLabel={handleLabel}
          isHorizontal={isHorizontal}
          size={size}
          min={min}
          max={maxSize}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={handlePointerUp}
          onKeyDown={handleKeyDown}
        />
      ) : null}

      <div className="min-h-0 min-w-0 flex-1 overflow-hidden">{children}</div>

      {edge === 'end' ? (
        <ResizeHandle
          handleLabel={handleLabel}
          isHorizontal={isHorizontal}
          size={size}
          min={min}
          max={maxSize}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={handlePointerUp}
          onKeyDown={handleKeyDown}
        />
      ) : null}
    </div>
  );
}

interface ResizeHandleProps {
  readonly handleLabel: string;
  readonly isHorizontal: boolean;
  readonly size: number;
  readonly min: number;
  readonly max: number;
  onPointerDown(event: PointerEvent<HTMLDivElement>): void;
  onPointerMove(event: PointerEvent<HTMLDivElement>): void;
  onPointerUp(event: PointerEvent<HTMLDivElement>): void;
  onKeyDown(event: KeyboardEvent<HTMLDivElement>): void;
}

function ResizeHandle({
  handleLabel,
  isHorizontal,
  size,
  min,
  max,
  onPointerDown,
  onPointerMove,
  onPointerUp,
  onKeyDown,
}: ResizeHandleProps) {
  return (
    <div
      role="separator"
      tabIndex={0}
      aria-label={handleLabel}
      aria-orientation={isHorizontal ? 'vertical' : 'horizontal'}
      aria-valuenow={Math.round(size)}
      aria-valuemin={min}
      aria-valuemax={max}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onKeyDown={onKeyDown}
      className={cn(
        // 命中区 8px、可见部分 1px：4px 的实心条在实测里"几乎抓不住"
        // （用户反馈"拖不动"），而加宽命中区不影响视觉——分隔线看起来仍是 1px
        'fd-transition group flex shrink-0 items-center justify-center bg-transparent',
        isHorizontal ? 'w-2 cursor-col-resize' : 'h-2 cursor-row-resize',
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          'block bg-line group-hover:bg-brand',
          isHorizontal ? 'h-full w-px' : 'h-px w-full',
        )}
      />
    </div>
  );
}
