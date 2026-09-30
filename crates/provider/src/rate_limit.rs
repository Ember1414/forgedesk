//! GitHub 限流头的解析与最近一次快照的持有。
//!
//! # 为什么每个响应都要捕获
//!
//! REST 配额（5000/h）是 M4 一切网络功能的命脉（docs/PLAN.md M4 风险表）。
//! GitHub 在**每一个**响应（包括错误）里都带回 `x-ratelimit-*` 头，
//! 在响应层统一捕获一次，上层（T4.10 的降级 UI、T4.11 的 Dashboard）
//! 就不必各自解析。快照保存在 `Arc` 后面：client 被多个 service 共享，
//! 限流状态天然是"全局最新值"，不是按请求隔离的。
//!
//! # 语义
//!
//! - `remaining == 0` + 403 → 主配额耗尽，`reset` 是配额恢复的 UNIX 秒；
//! - 429 / `retry-after` → 次级限流（滥用保护），`Retry-After` 头的处理
//!   推迟到 T4.10（降级 UI 要显示"多久后重试"时一起做）。

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 一次解析出的 GitHub 限流状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitState {
    /// 配额桶名（`core` / `graphql` / `search`…），头缺失时为 `None`。
    pub resource: Option<String>,
    /// 配额上限（如 5000）。
    pub limit: u32,
    /// 剩余额度。
    pub remaining: u32,
    /// 当前窗口已用额度。
    pub used: u32,
    /// 配额恢复时间（UNIX 秒）。
    pub reset_unix_secs: u64,
}

impl RateLimitState {
    /// 从响应头解析；头不齐时返回 `None`（GitHub 网关 / 某些代理不带回这些头）。
    #[must_use]
    pub fn from_headers(headers: &reqwest::header::HeaderMap) -> Option<Self> {
        let header = |name: &str| headers.get(name)?.to_str().ok()?.trim().parse::<u64>().ok();
        let limit = header("x-ratelimit-limit")?;
        let remaining = header("x-ratelimit-remaining")?;
        let reset_unix_secs = header("x-ratelimit-reset")?;
        Some(Self {
            resource: headers
                .get("x-ratelimit-resource")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned),
            limit: limit.try_into().ok()?,
            remaining: remaining.try_into().ok()?,
            used: header("x-ratelimit-used")
                .unwrap_or(0)
                .try_into()
                .unwrap_or(0),
            reset_unix_secs,
        })
    }
}

/// 线程安全的"最近一次限流快照"持有者。
#[derive(Debug, Clone, Default)]
pub struct RateLimitTracker {
    state: Arc<RwLock<Option<RateLimitState>>>,
}

impl RateLimitTracker {
    /// 新建（无快照）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从一个响应头集合更新快照。解析失败保持原值——
    /// 一条没有限流头的响应不代表配额恢复，只是信息缺失。
    pub fn capture(&self, headers: &reqwest::header::HeaderMap) {
        if let Some(state) = RateLimitState::from_headers(headers) {
            *self.state.write() = Some(state);
        }
    }

    /// 当前快照（从未见过带限流头的响应时为 `None`）。
    #[must_use]
    pub fn snapshot(&self) -> Option<RateLimitState> {
        self.state.read().clone()
    }

    /// 是否处于"主配额耗尽"状态。
    ///
    /// 判定条件是 remaining == 0：这比"上次请求是不是 403"更可靠——
    /// 并发请求里 403 可能被别的错误盖过，而 remaining 是权威值。
    #[must_use]
    pub fn is_exhausted(&self) -> bool {
        self.snapshot().is_some_and(|s| s.remaining == 0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{RateLimitState, RateLimitTracker};
    use reqwest::header::HeaderMap;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                reqwest::header::HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    fn full_headers(remaining: &str) -> HeaderMap {
        headers(&[
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", remaining),
            ("x-ratelimit-reset", "1790000000"),
            ("x-ratelimit-used", "1"),
            ("x-ratelimit-resource", "core"),
        ])
    }

    #[test]
    fn rate_limit_headers_parse_into_a_snapshot() {
        let state = RateLimitState::from_headers(&full_headers("4999")).unwrap();

        assert_eq!(state.limit, 5000);
        assert_eq!(state.remaining, 4999);
        assert_eq!(state.used, 1);
        assert_eq!(state.reset_unix_secs, 1_790_000_000);
        assert_eq!(state.resource.as_deref(), Some("core"));
    }

    #[test]
    fn incomplete_headers_parse_to_none_instead_of_zeroes() {
        // GitHub 网关或某些代理会吞掉部分头：宁可说"不知道"，也不能
        // 拿 half 解析出的 0 去触发"配额耗尽"的降级 UI
        let sparse = headers(&[("x-ratelimit-limit", "5000")]);
        assert_eq!(RateLimitState::from_headers(&sparse), None);
        assert_eq!(RateLimitState::from_headers(&HeaderMap::new()), None);
    }

    #[test]
    fn tracker_keeps_the_latest_snapshot_and_reports_exhaustion() {
        let tracker = RateLimitTracker::new();
        assert_eq!(tracker.snapshot(), None);
        assert!(!tracker.is_exhausted());

        tracker.capture(&full_headers("42"));
        assert_eq!(tracker.snapshot().unwrap().remaining, 42);

        // 一条无头响应不覆盖已有快照
        tracker.capture(&HeaderMap::new());
        assert_eq!(tracker.snapshot().unwrap().remaining, 42);

        tracker.capture(&full_headers("0"));
        assert!(tracker.is_exhausted());
    }
}
