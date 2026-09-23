/**
 * 轻提示队列（Toast）。
 *
 * 为什么队列放在 store 而不是组件内部：提示的来源是**任意位置**——
 * 一次失败的 fetch、一个被拒绝的快照、一次成功的推送。若队列挂在组件里，
 * 就得让"离得最近的那个 Provider"来承担，跨路由的消息会丢；
 * 放在模块级 store 后，任何地方 `pushToast(...)` 都能到达同一个出口。
 *
 * 与 T0.6 的关系：错误统一模型落地后，`ErrorToast` 会复用本 store，
 * 但**不**复用文案——错误文案来自 AppError 的 i18n key，见 docs/PLAN.md §5.5。
 */
import { create } from 'zustand';

export const TOAST_TONES = ['info', 'success', 'warning', 'danger'] as const;
export type ToastTone = (typeof TOAST_TONES)[number];

export interface ToastRecord {
  readonly id: string;
  readonly tone: ToastTone;
  /** 标题（必填）：一句话说清楚发生了什么。 */
  readonly title: string;
  /** 补充说明（如错误提示里的"可能原因"）。 */
  readonly description?: string;
  /** 自动消失时间（ms）；0 表示需要用户手动关闭（用于错误等必须被看到的提示）。 */
  readonly duration: number;
  /** 可选动作（如"重试""查看日志"）。 */
  readonly actionLabel?: string;
  readonly onAction?: () => void;
}

export interface ToastInput {
  readonly tone?: ToastTone;
  readonly title: string;
  readonly description?: string;
  readonly duration?: number;
  readonly actionLabel?: string;
  readonly onAction?: () => void;
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
      duration: input.duration ?? DEFAULT_DURATION,
      ...(input.actionLabel === undefined ? {} : { actionLabel: input.actionLabel }),
      ...(input.onAction === undefined ? {} : { onAction: input.onAction }),
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
