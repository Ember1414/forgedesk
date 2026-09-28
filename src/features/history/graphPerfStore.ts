/**
 * 提交图的性能采样（Zustand，T2.2；只给 dev 性能面板用）。
 *
 * # 为什么需要一个 store 而不是把数字打在 console 里
 *
 * 采样点在三个互不相通的地方：`useGraphQuery`（IPC 往返）、`GraphCanvas`
 * （静态层与动态层的绘制耗时、fps）。它们唯一的共同消费者是
 * `src/ui/__dev__/GraphPerfPanel.tsx`——一个只在 DEV 下挂到路由上的面板。
 * 用一个 store 把"生产端"与"消费端"解耦，面板就能独立挂载/卸载，
 * 而不必让 HistoryPage 把一堆数字用 props 传出去。
 *
 * # 为什么默认关闭
 *
 * `performance.now()` 本身很便宜，但为了量"静态层绘制耗时"必须在重绘前后
 * 各取一次时间戳，而重绘是每帧的事；fps 采样同理。默认关闭意味着
 * **生产构建里这些代码路径根本不会执行**（`enabled` 为 false 时直接返回），
 * 不会给十万节点的滚动加上任何常数开销。
 *
 * # 关于"布局耗时"的口径（重要）
 *
 * 面板上的"布局耗时"量的是 **`git_log_page` 的 IPC 往返时间**，其中包含了
 * Rust 侧对**这一页**（pageSize ≤ 500）的布局计算。它**不是**全量布局的耗时：
 * 后端基准实测（随机 DAG）5000 节点 ~1ms、50000 节点 ~8.4s、100000 节点 ~34.2s
 * ——布局在泳道数很多时是超线性的，但首屏路径永远只对一页做布局，
 * 因此那组数字不会出现在面板上，也不应该被当成首屏预期。
 * 把两件事混为一谈会得出"这个界面要 30 秒才能打开"的错误结论。
 */
import { create } from 'zustand';

/** 一次性能快照（面板直接渲染这个对象）。 */
export interface GraphPerfSnapshot {
  /** 已累积的全局行数。 */
  readonly rowCount: number;
  /** 实际画出来的节点数（视口裁剪后）。 */
  readonly nodeCount: number;
  /** 实际画出来的边数（视口裁剪后）。 */
  readonly edgeCount: number;
  readonly laneCount: number;
  readonly scale: number;
  /** `git_log_page` 的 IPC 往返毫秒数（含后端对这一页的布局）。 */
  readonly layoutMs: number | null;
  /** 静态层（连线 + 节点 + ref）一次重绘的毫秒数。 */
  readonly staticDrawMs: number | null;
  /** 动态层（hover + 选中环）一次重绘的毫秒数。 */
  readonly dynamicDrawMs: number | null;
  readonly fps: number | null;
  /** JS 堆占用（MB）；浏览器不暴露 `performance.memory` 时为 `null`（面板显示 n/a）。 */
  readonly heapMb: number | null;
  /** 最近一次采样的时间戳（`performance.now()`，仅用于显示"多久之前"）。 */
  readonly updatedAt: number | null;
}

/** 初始快照：全部 `null` / 0，面板据此显示"等待采样"。 */
export const initialGraphPerfSnapshot: GraphPerfSnapshot = {
  rowCount: 0,
  nodeCount: 0,
  edgeCount: 0,
  laneCount: 0,
  scale: 1,
  layoutMs: null,
  staticDrawMs: null,
  dynamicDrawMs: null,
  fps: null,
  heapMb: null,
  updatedAt: null,
};

/**
 * 一次采样的载荷：所有字段可选。
 *
 * 显式写 `| undefined` 而不是用 `Partial<GraphPerfSnapshot>`：
 * `exactOptionalPropertyTypes` 下 `Partial` 生成的属性不接受 `undefined`，
 * 于是调用方为了传一个"暂时没量到"的值就得先构造条件展开——
 * 而性能采样里"这一项没量到"是常态（例如动态层这一帧没重画）。
 */
export type GraphPerfSample = {
  readonly [K in keyof GraphPerfSnapshot]?: GraphPerfSnapshot[K] | undefined;
};

export interface GraphPerfState {
  /** 是否开启采样（面板上的开关；默认关）。 */
  readonly enabled: boolean;
  readonly snapshot: GraphPerfSnapshot;
  setEnabled(enabled: boolean): void;
  toggleEnabled(): void;
  /** 合并一次采样（`undefined` 的字段保持原值）。 */
  record(sample: GraphPerfSample): void;
  /** 清空快照（关掉面板或切换仓库时用；不改 `enabled`）。 */
  reset(): void;
}

/** 初始状态（导出供测试复位）。 */
export const initialGraphPerfState = {
  enabled: false,
  snapshot: initialGraphPerfSnapshot,
};

export const useGraphPerfStore = create<GraphPerfState>()((set, get) => ({
  ...initialGraphPerfState,

  setEnabled: (enabled) => {
    set({ enabled });
  },

  toggleEnabled: () => {
    set({ enabled: !get().enabled });
  },

  record: (sample) => {
    // 关闭时什么都不做：这条短路是"默认零开销"的全部保证，
    // 因此它必须在任何对象分配之前（不要先构造 merged 再判断）。
    if (!get().enabled) {
      return;
    }
    const previous = get().snapshot;
    // 逐字段 `??` 而不是一次展开 + 过滤 `undefined`：
    // 展开写法必须先把快照转成 `Record<string, unknown>` 再断言回去，
    // 而断言会让"新增字段忘了处理"从编译错误变成运行期的 `undefined`。
    set({
      snapshot: {
        rowCount: sample.rowCount ?? previous.rowCount,
        nodeCount: sample.nodeCount ?? previous.nodeCount,
        edgeCount: sample.edgeCount ?? previous.edgeCount,
        laneCount: sample.laneCount ?? previous.laneCount,
        scale: sample.scale ?? previous.scale,
        layoutMs: sample.layoutMs ?? previous.layoutMs,
        staticDrawMs: sample.staticDrawMs ?? previous.staticDrawMs,
        dynamicDrawMs: sample.dynamicDrawMs ?? previous.dynamicDrawMs,
        fps: sample.fps ?? previous.fps,
        heapMb: sample.heapMb ?? previous.heapMb,
        updatedAt: Date.now(),
      },
    });
  },

  reset: () => {
    set({ snapshot: initialGraphPerfSnapshot });
  },
}));

/** 便捷函数：不依赖 React 上下文即可采样（`queryFn` 与 rAF 回调里用）。 */
export function recordGraphPerf(sample: GraphPerfSample): void {
  useGraphPerfStore.getState().record(sample);
}

/** 便捷函数：采样开关是否打开（热路径上先判断再量，避免白量）。 */
export function isGraphPerfEnabled(): boolean {
  return useGraphPerfStore.getState().enabled;
}

// ---------------------------------------------------------------- fps 计量

/** fps 的滑动窗口帧数（30 帧 ≈ 0.5 秒，够平滑又不会滞后到看不出卡顿）。 */
export const FPS_WINDOW_FRAMES = 30;

/** 一个 fps 计量器（有内部状态，但不是 React 状态）。 */
export interface FpsMeter {
  /**
   * 记一帧。
   *
   * @returns 当前 fps；样本不足两帧时返回 `null`（面板显示 n/a）。
   *   返回 `null` 而不是 0：0 会被读成"完全卡死"，而真实情况是"还不知道"。
   */
  push(timestampMs: number): number | null;
  reset(): void;
}

/**
 * 建一个滑动窗口 fps 计量器。
 *
 * 为什么用"窗口首尾时间差 / 帧数"而不是逐帧取倒数再平均：
 * 逐帧倒数会把一次 200ms 的卡顿放大成"fps = 5"并长时间挂在面板上，
 * 而窗口平均给出的是这段时间里的**真实吞吐**，与用户感受到的流畅度一致。
 */
export function createFpsMeter(windowFrames: number = FPS_WINDOW_FRAMES): FpsMeter {
  const size = Math.max(2, windowFrames);
  let stamps: number[] = [];
  return {
    push: (timestampMs) => {
      if (!Number.isFinite(timestampMs)) {
        return null;
      }
      stamps.push(timestampMs);
      if (stamps.length > size) {
        stamps = stamps.slice(-size);
      }
      const first = stamps[0] ?? timestampMs;
      const span = timestampMs - first;
      if (stamps.length < 2 || span <= 0) {
        return null;
      }
      return ((stamps.length - 1) * 1000) / span;
    },
    reset: () => {
      stamps = [];
    },
  };
}

/**
 * 读 JS 堆占用（MB）；不可用时返回 `null`。
 *
 * `performance.memory` 是非标准扩展（Chromium 系有，Firefox / Safari 没有），
 * 因此这里全程用 `unknown` 逐层收窄，而不是断言出一个类型来——
 * 断言会让"字段不存在"这种情况绕过类型检查，在面板上印出 `NaN MB`。
 * 面板拿到 `null` 时显示 n/a，这是任务里明确要求的降级表现。
 */
export function readHeapMb(): number | null {
  const candidate: unknown = globalThis.performance;
  if (typeof candidate !== 'object' || candidate === null) {
    return null;
  }
  const memory: unknown = (candidate as { memory?: unknown }).memory;
  if (typeof memory !== 'object' || memory === null) {
    return null;
  }
  const used: unknown = (memory as { usedJSHeapSize?: unknown }).usedJSHeapSize;
  if (typeof used !== 'number' || !Number.isFinite(used)) {
    return null;
  }
  return used / (1024 * 1024);
}
