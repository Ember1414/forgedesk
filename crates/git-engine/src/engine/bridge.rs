//! 同步 ↔ 异步的桥。
//!
//! # 为什么需要它
//!
//! `GitProcess`（T1.1）是 `async` 的：它用 `tokio::process` 做超时、取消与
//! 管道读取，这些能力在 `std::process` 上要自己写一遍且更容易写错。
//! 而 `GitEngine` 按 `docs/PLAN.md` §5.6 是**同步**接口，现有 6 个 Tauri 命令
//! 也全是同步的。两者之间需要一座桥。
//!
//! # 为什么在独立线程上驱动
//!
//! 最直接的写法是 `runtime.block_on(future)`，但它在**当前线程已经在驱动
//! 另一个运行时**时会 panic（"Cannot start a runtime from within a runtime"）。
//! Tauri 的异步命令线程正是这种情况，而"某个命令碰巧是 async 的"不应该让
//! 引擎崩掉。因此这里固定在一个独立线程上驱动：代价是一次线程创建（~50µs），
//! 相对于一次 git 进程调用（毫秒级起步）可以忽略。
//!
//! # 为什么用 `current_thread` 运行时
//!
//! 我们只需要"能跑 async 代码"（进程 + 定时器 + 取消），不需要多线程调度器：
//! 调用方本来就在阻塞等待，工作线程数是 1 还是 8 对吞吐没有影响，
//! 而 1 个线程意味着 1 份栈与 1 个事件循环。

//! # 为什么 `Drop` 走后台关闭
//!
//! tokio 的运行时不**允许在异步上下文里被 drop**，会直接 panic：
//!
//! ```text
//! Cannot drop a runtime in a context where blocking is not allowed.
//! This happens when a runtime is dropped from within an asynchronous context.
//! ```
//!
//! 而引擎完全可能被一个 async 的 Tauri 命令创建或销毁。因此这里用
//! [`tokio::runtime::Runtime::shutdown_background`] 显式关闭：不阻塞、不 panic。
//! 代价是"关闭是异步的"，而进程退出时本来也不需要等待它。

use std::future::Future;

use forgedesk_domain::{AppError, AppResult, ErrorCode};

/// 驱动异步代码的同步桥。
pub struct BlockingBridge {
    /// `Drop` 需要把运行时**取出来**才能调用 `shutdown_background`（它消费 self），
    /// 因此是 `Option`。只有 `Drop` 期间才可能为 `None`。
    runtime: Option<tokio::runtime::Runtime>,
}

impl BlockingBridge {
    /// 创建桥（建立一个单线程运行时）。
    pub fn new() -> AppResult<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                AppError::new(
                    ErrorCode::Internal,
                    "failed to create the runtime used to run git commands",
                )
                .with_detail(error.to_string())
            })?;

        Ok(Self {
            runtime: Some(runtime),
        })
    }

    /// 取运行时。
    ///
    /// 只有 `Drop` 会把它取走，而 `Drop` 需要 `&mut self`（独占借用），
    /// 因此持有 `&self` 时它必然存在——这条分支是为了不写 `unwrap` 而存在的。
    fn runtime(&self) -> AppResult<&tokio::runtime::Runtime> {
        self.runtime.as_ref().ok_or_else(|| {
            AppError::new(
                ErrorCode::Internal,
                "the git runtime has already been shut down",
            )
        })
    }

    /// 阻塞执行一个 future。
    ///
    /// 内部 future 的 panic 会被原样传播（不吞掉），与直接 `block_on` 的行为一致。
    pub fn block_on<F>(&self, future: F) -> AppResult<F::Output>
    where
        F: Future + Send,
        F::Output: Send,
    {
        let runtime = self.runtime()?;
        Ok(std::thread::scope(|scope| {
            match scope.spawn(|| runtime.block_on(future)).join() {
                Ok(output) => output,
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }))
    }
}

impl Drop for BlockingBridge {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

impl std::fmt::Debug for BlockingBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockingBridge").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::BlockingBridge;
    use std::time::Duration;

    #[test]
    fn block_on_returns_the_future_output() {
        let bridge = BlockingBridge::new().unwrap();

        assert_eq!(bridge.block_on(async { 7_u32 }).unwrap(), 7);
    }

    #[test]
    fn block_on_works_when_the_caller_is_already_inside_a_runtime() {
        // 这条是本模块存在的理由：Tauri 的异步命令线程上调用引擎必须不 panic
        let outer = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let value = outer.block_on(async {
            let bridge = BlockingBridge::new().unwrap();
            bridge.block_on(async { 42_u32 }).unwrap()
        });

        assert_eq!(value, 42);
    }

    #[test]
    fn dropping_the_bridge_inside_an_async_context_does_not_panic() {
        // tokio 默认的 Drop 会 panic（"Cannot drop a runtime in a context where
        // blocking is not allowed"），因此这里走 shutdown_background。
        let outer = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        outer.block_on(async {
            let bridge = BlockingBridge::new().unwrap();
            drop(bridge);
        });
    }

    #[test]
    fn timers_inside_the_bridge_actually_advance() {
        // 运行时没启用 time 驱动时 sleep 会直接返回，这个断言能抓住那种配置错误
        let bridge = BlockingBridge::new().unwrap();

        let elapsed = bridge
            .block_on(async {
                let start = std::time::Instant::now();
                tokio::time::sleep(Duration::from_millis(20)).await;
                start.elapsed()
            })
            .unwrap();

        assert!(elapsed >= Duration::from_millis(15), "实际耗时 {elapsed:?}");
    }

    #[test]
    fn panics_inside_the_future_are_propagated() {
        let bridge = BlockingBridge::new().unwrap();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            bridge.block_on(async { panic!("boom") })
        }));

        assert!(result.is_err(), "panic 不应被静默吞掉");
    }
}
