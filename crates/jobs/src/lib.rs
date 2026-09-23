//! 长任务系统：任务标识、进度广播、取消与结果上报。
//!
//! # 为什么需要它
//!
//! 克隆一个仓库动辄几十秒。同步执行意味着界面在整段时间里没有任何反馈——
//! 用户唯一能做的事就是怀疑程序卡死，然后强杀它（AGENTS.md §6「长任务可取消」）。
//!
//! 因此所有 > 500ms 的操作都走这里：**立即返回任务 id**，进度与结果通过
//! 事件推送（`job:progress` / `job:done` / `job:failed`，见 `docs/API.md` §3）。
//!
//! # 本 crate 不依赖 Tauri
//!
//! 事件的**投递方式**由宿主决定：这里只定义 [`JobReporter`] 这个出口，
//! `commands` 层实现一个"转成 Tauri 事件"的 reporter，测试实现一个
//! "收进 Vec"的 reporter。这样任务编排逻辑可以在纯 Rust 测试里跑完，
//! 不必启动桌面运行时。
//!
//! # 为什么用 `std::thread` 而不是 tokio
//!
//! 任务体是**阻塞**的（git CLI 通过 `BlockingBridge` 同步等待子进程）。
//! 放进 tokio 的异步工作线程会占死一个 worker，而 `spawn_blocking` 又要求
//! 调用点处在运行时上下文里。`std::thread` 没有这些前提：一个任务一条线程，
//! 数量由用户操作决定（同时最多几个），不会失控。
//!
//! [`CancellationToken`](tokio_util::sync::CancellationToken) 本身不依赖运行时，
//! 因此取消能力可以原样带过来。

#![forbid(unsafe_code)]

pub mod event;
pub mod runner;

pub use event::{JobEvent, JobId, JobProgress, JobReporter};
pub use runner::{JobContext, JobRegistry, JobRunner};

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
