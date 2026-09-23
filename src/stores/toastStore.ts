/**
 * 轻提示队列（Toast）。
 *
 * 为什么队列放在 store 而不是组件内部：提示的来源是**任意位置**——
 * 一次失败的 fetch、一个被拒绝的快照、一次成功的推送。若队列挂在组件里，
 * 就得让"离得最近的那个 Provider"来承担，跨路由的消息会丢；
 * 放在模块级 store 后，任何地方 `pushToast(...)` 都能到达同一个出口。
 *
 * 与错误模型的关系：错误提示走同一条队列，但额外携带 `detail`（折叠展示的原始信息）
 * 与 `actions`（可点击的修复入口）——它们由 `useAppError()` 从 AppError 映射而来
 * （见 src/lib/errors.ts）。
 */
import { create } from 'zustand';

export const TOAST_TONES = ['info', 'success', 'warning', 'danger'] as const;
export type ToastTone = (typeof TOAST_TONES)[number];

/**
 * 提示上的可执行动作。
 *
 * 两种执行方式二选一：
 * - `onClick`：纯前端动作（如"复制到剪贴板""跳到冲突页"）；
 * - `command`：后端命令（如"刷新状态"），由 Toaster 统一经 src/lib/ipc 调用，
 *   失败时自动转成新的错误提示。
 */
export interface ToastAction {
  readonly id: string;
  readonly label: string;
  readonly onClick?: () => void;
  readonly command?: string;
  readonly args?: Record<string, unknown>;
}

export interface ToastRecord {
  readonly id: string;
  readonly tone: ToastTone;
  /** 标题（必填）：一句话说清楚发生了什么。 */
  readonly title: string;
  /** 补充说明（错误提示里通常是"可能的原因 / 下一步建议"）。 */
  readonly description?: string;
  /** 原始详情（已脱敏）。默认折叠——技术细节不该淹没普通用户。 */
  readonly detail?: string;
  /** 可点击的修复动作。 */
  readonly actions?: readonly ToastAction[];
  /**
   * 该提示的产生时间（Unix 毫秒）。
   *
   * 错误提示会带这个值：用户点"查看相关日志"时，日志查看器据此高亮
   * "错误发生时间附近的行"——没有它就只能在几百行里自己找。
   */
  readonly occurredAt?: number;
  /** 自动消失时间（ms）；0 表示需要用户手动关闭（错误提示一律用 0）。 */
  readonly duration: number;
}

export interface ToastInput {
  readonly tone?: ToastTone;
  readonly title: string;
  readonly description?: string;
  readonly detail?: string;
  readonly actions?: readonly ToastAction[];
  readonly occurredAt?: number;
  readonly duration?: number;
}

/**
 * 同屏最多保留的提示数。
 * 超出时丢弃**最早**的一条：用户最关心最近发生的事，
 * 而且无限堆叠会让提示区盖住界面。
 */
const MAX_VISIBLE_TOASTS = 4;

/** 默认自动消失时间：够看清一行标题，又不至于挡视线。 */
const DEFAULT_DURATION = 5000;

let toastSequence = 0;

export interface ToastStoreState {
  readonly toasts: readonly ToastRecord[];
  pushToast(input: ToastInput): string;
  dismissToast(toastId: string): void;
  clearToasts(): void;
}

/** 初始状态（导出供测试复位；store 是模块级单例）。 */
export const initialToastState = { toasts: [] as readonly ToastRecord[] };

export const useToastStore = create<ToastStoreState>()((set) => ({
  ...initialToastState,

  pushToast: (input) => {
    toastSequence += 1;
    const id = `toast-${String(toastSequence)}`;
    const record: ToastRecord = {
      id,
      tone: input.tone ?? 'info',
      title: input.title,
      // exactOptionalPropertyTypes 下不能显式赋 undefined，故按存在性展开
      ...(input.description === undefined ? {} : { description: input.description }),
      ...(input.detail === undefined ? {} : { detail: input.detail }),
      ...(input.actions === undefined ? {} : { actions: input.actions }),
      ...(input.occurredAt === undefined ? {} : { occurredAt: input.occurredAt }),
      duration: input.duration ?? DEFAULT_DURATION,
    };

    set((state) => ({
      toasts: [...state.toasts, record].slice(-MAX_VISIBLE_TOASTS),
    }));

    return id;
  },

  dismissToast: (toastId) => {
    set((state) => ({ toasts: state.toasts.filter((toast) => toast.id !== toastId) }));
  },

  clearToasts: () => {
    set({ toasts: [] });
  },
}));

/** 便捷函数：不依赖 React 上下文，从任意模块即可推送提示。 */
export function pushToast(input: ToastInput): string {
  return useToastStore.getState().pushToast(input);
}
