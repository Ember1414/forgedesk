# Diff 查看器性能基准（T1.5）

- 环境：Windows 11 开发机、Vite dev server + Playwright（Chromium）、mock IPC。
- 方法：`e2e/workspace.spec.ts` 内的 T1.5 基准用例——3000 行 diff（2 hunk ×
  1500 上下文行）打开后 `scrollTop` 滚到底再回顶，计时到滚动完成。

| 场景 | 数值 | 结论 |
| --- | --- | --- |
| 3000 行 diff 两次全量滚动 | **约 150ms**（阈值 <1000ms） | 虚拟化只渲染可见行（25 行左右），滚动不随总行数增长 |
| 对照：10000 文件状态面板首屏 | 约 350-450ms（T1.4） | 同一套 VirtualList 方案的横向参照 |

## 为何不用渲染全部行做对比

全部渲染 3000 行 × 每行 3-5 个节点的 DOM 在中端机上会出现可感知的卡顿
（数百 ms 到秒级），这正是虚拟化存在的理由；基准只需证明"虚拟化后滚动是常数开销"。

## 字符级高亮的门槛

`CHAR_DIFF_MAX_CHANGED_LINES = 2000`（`src/features/diff/diffModel.ts`）：
变更行超过阈值时自动关闭修改行的词级高亮，只保留整行配色。
jsdiff 的 `diffWordsWithSpace` 是每行一次的 O(行长度) 计算，2000 行在
开发机上的延迟可感知；阈值调低以换稳定 60fps（`diff` 包 v8，MIT、零依赖）。