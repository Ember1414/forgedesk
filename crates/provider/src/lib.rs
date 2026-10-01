//! 托管平台适配层：`HostProvider` trait 与 GitHub / GitLab / Gitea 实现。
//!
//! 归属里程碑：M4（docs/PLAN.md），trait 骨架与注册表自 T4.1 起落地。
//!
//! # 模块结构
//!
//! ```text
//! model.rs     ProviderId 与 ProviderCapabilities（能力声明）
//! registry.rs  host / 远端 URL → provider 的绑定表
//! traits.rs    HostProvider 聚合 trait 与六个子 trait
//! github/      GitHubProvider（T4.1b 起：HTTP、错误映射、Device Flow）
//! ```
//!
//! # 分层位置
//!
//! 本 crate 是 infra 层：可依赖 domain 与真实网络栈（reqwest/octocrab），
//! 但**不**依赖 commands / services（依赖只能由外向内）。
//! 凭据的存取不直接碰 keyring——一律经 `forgedesk-credentials` 的
//! `CredentialStore`，本 crate 只持有 `CredentialRef`（红线 R8：令牌不过界）。
//!
//! # 与 UI 的契约
//!
//! UI 依据 [`ProviderCapabilities`] 决定显示哪些功能面板；
//! 依据 [`ProviderRegistry`] 判断一个远端归属哪个平台。
//! 禁止在任何业务代码里写 `if provider.id() == ProviderId::GitHub` 这类判断
//! （docs/ARCHITECTURE.md §5 的扩展点规则）。

#![forbid(unsafe_code)]

/// Provider 的基础标识模型。
pub mod model;
pub mod pulls;
/// host / 远端 URL → provider 的绑定表。
pub mod registry;
/// `HostProvider` trait 树（业务层唯一可见的抽象）。
pub mod traits;

pub use model::{ProviderCapabilities, ProviderId};
pub use registry::{ProviderRegistry, ResolvedRemote};
pub use traits::{
    AuthFlow, CiService, HostProvider, IssueService, PullService, ReleaseService, RepoService,
};

/// Device Flow 与 PAT 校验的数据类型。
pub mod auth;
/// GitHub 的 HTTP 底座（UA / 超时 / 代理 / 重试 / 限流捕获）。
pub mod client;
/// `octocrab::Error` → `AppError` 的映射。
pub mod error;
pub mod github;
/// 限流头的解析与快照。
pub mod rate_limit;
/// 令牌脱敏（红线 R8 的 provider 侧兜底）。
pub mod redact;
/// GitHubProvider：GitHub / GHE 的平台实现。
pub mod repos;

pub use auth::{
    poll_until_authorized, AuthorizedLogin, DeviceFlowPoll, DeviceFlowStart, PollOptions,
    VerifiedAccount, DEFAULT_SCOPES,
};
pub use client::{ApiRequest, GitHubHttp, HttpConfig};
pub use error::map_octocrab_error;
pub use github::GitHubProvider;
pub use pulls::{
    MergeOutcome, MergePullRequest, MergeStrategy, PullComment, PullPage, PullRequestDetail,
    PullRequestSummary, PullReview, PullState, ReviewEvent,
};
pub use rate_limit::{RateLimitState, RateLimitTracker};
pub use repos::{RemoteRepo, RepoListScope, RepoPage};

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
