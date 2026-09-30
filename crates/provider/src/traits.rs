//! `HostProvider` trait 树：业务层可见的平台抽象（docs/PLAN.md §5.7）。
//!
//! # 为什么拆成六个子 trait
//!
//! [`HostProvider`] 是聚合根，而调用方几乎总是只需要其中一个子服务
//! （PR 页面只要 [`PullService`]）。拆开之后：
//!
//! - 各子服务可以在**自己的任务里独立补齐方法**（T4.5/T4.7/T4.8/T4.9），
//!   不用反复改聚合 trait；
//! - mock 测试只需要实现用到的子 trait，而不是整个平台；
//! - `GitHubProvider` 内部按子服务分模块，与 trait 一一对应。
//!
//! # 对象安全（dyn 兼容）
//!
//! services 层持有 `Box<dyn HostProvider>`（"Git 引擎可替换"约定的平台版），
//! 因此 trait 必须是 dyn 兼容的：子 trait 方法返回具体类型而不是泛型，
//! 异步方法落地时使用 `async_trait`（原生 `async fn` in trait 目前不能进 `dyn`）。
//!
//! 当前的六个子 trait 是**骨架标记**：先锁定聚合形态，方法随各自任务落地
//! （见各 trait 的文档注释）。没有提前填方法，是因为方法签名应由该子服务
//! 的第一个真实用例驱动，而不是凭空设计。

use crate::model::{ProviderCapabilities, ProviderId};

/// 认证子服务：登录、令牌校验、Device Flow（docs/PLAN.md M4.1/T4.3）。
///
/// # 轮询循环归调用方
///
/// [`AuthFlow::poll_device_flow`] 只做**一次**查询并报告状态；"按 interval
/// 睡眠、收到 slow_down 后 +5s"的循环由 services 层驱动（T4.3 的登录向导）。
/// 这样轮询节奏可被取消令牌打断，而 trait 实现保持无状态。
#[async_trait::async_trait]
pub trait AuthFlow: Send + Sync {
    /// 发起 Device Flow：返回用户码与验证地址。
    ///
    /// UI 的三步引导（docs/PLAN.md M4 风险表）：复制 `user_code` →
    /// 打开 `verification_uri` → 应用按 `interval_secs` 轮询。
    async fn start_device_flow(
        &self,
        scopes: &[&str],
    ) -> Result<crate::auth::DeviceFlowStart, forgedesk_domain::AppError>;

    /// 轮询一次令牌换取结果。
    async fn poll_device_flow(
        &self,
        flow: &crate::auth::DeviceFlowStart,
    ) -> Result<crate::auth::DeviceFlowPoll, forgedesk_domain::AppError>;

    /// 校验一个 PAT 并返回账号信息（GitHub 的 `/user`）。
    async fn verify_pat(
        &self,
        token: secrecy::SecretString,
    ) -> Result<crate::auth::VerifiedAccount, forgedesk_domain::AppError>;
}

/// 仓库子服务：列表/搜索/星标/fork（T4.5 落地）。
///
/// 分页用"页码游标"（[`crate::repos::RepoPage::next_page`]）而不是泛型
/// cursor：GitHub 的分页就是 page 参数，UI 的无限滚动每次追加一页。
/// `token: Option<SecretString>` 的方法匿名也可用（搜索/公开仓库），
/// 其余方法传 `None` 会以 401 → `AUTH_REQUIRED`/`AUTH_EXPIRED` 失败。
#[async_trait::async_trait]
pub trait RepoService: Send + Sync {
    /// 列出已登录账号可见的仓库（按 [`crate::repos::RepoListScope`] 收窄）。
    async fn list_authenticated(
        &self,
        token: secrecy::SecretString,
        scope: crate::repos::RepoListScope,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> Result<crate::repos::RepoPage, forgedesk_domain::AppError>;

    /// 列出该账号星标的仓库。
    async fn list_starred(
        &self,
        token: secrecy::SecretString,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> Result<crate::repos::RepoPage, forgedesk_domain::AppError>;

    /// 搜索仓库（匿名可用）。
    async fn search(
        &self,
        query: &str,
        token: Option<secrecy::SecretString>,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> Result<crate::repos::RepoPage, forgedesk_domain::AppError>;

    /// 单个仓库详情。
    async fn get(
        &self,
        owner: &str,
        repo: &str,
        token: Option<secrecy::SecretString>,
    ) -> Result<crate::repos::RemoteRepo, forgedesk_domain::AppError>;

    /// 加星 / 取消加星。
    async fn set_starred(
        &self,
        token: secrecy::SecretString,
        owner: &str,
        repo: &str,
        starred: bool,
    ) -> Result<(), forgedesk_domain::AppError>;

    /// fork 到当前账号名下（GitHub 返回 202：副本异步创建中）。
    async fn fork(
        &self,
        token: secrecy::SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<crate::repos::RemoteRepo, forgedesk_domain::AppError>;
}

/// Pull Request 子服务：列表/详情/评论/review/合并（T4.7 落地方法）。
pub trait PullService: Send + Sync {}

/// Issue 子服务：列表/筛选/创建/评论/关闭（T4.8 落地方法）。
pub trait IssueService: Send + Sync {}

/// CI 子服务：workflow 列表、运行记录、日志、重跑/取消（T4.9 落地方法）。
pub trait CiService: Send + Sync {}

/// Release 子服务：列表与产物下载（M4 计划外，随需求落地）。
pub trait ReleaseService: Send + Sync {}

/// 托管平台的聚合抽象：业务层**只**依赖此 trait，不感知具体实现。
///
/// 实现方还必须实现 [`std::fmt::Debug`]（不含令牌——红线 R8），
/// 以便日志与审计能标识 provider 实例。
pub trait HostProvider: Send + Sync + std::fmt::Debug {
    /// 平台标识。
    fn id(&self) -> ProviderId;

    /// 该实例指向的 host（`github.com`、`ghe.acme.com`、自建 Gitea 的域名）。
    ///
    /// 为什么聚合 trait 需要 host：多账号 + 企业版（D4.2/D4.3）下，
    /// "同一个 GitHub 平台、不同 host"是两个完全独立的实例。
    fn host(&self) -> &str;

    /// 能力声明：UI 依据它显示/隐藏功能，禁止按 provider 名字写 if。
    fn capabilities(&self) -> ProviderCapabilities;

    /// 认证子服务。
    fn auth(&self) -> &dyn AuthFlow;
    /// 仓库子服务。
    fn repos(&self) -> &dyn RepoService;
    /// Pull Request 子服务。
    fn pulls(&self) -> &dyn PullService;
    /// Issue 子服务。
    fn issues(&self) -> &dyn IssueService;
    /// CI 子服务。
    fn actions(&self) -> &dyn CiService;
    /// Release 子服务。
    fn releases(&self) -> &dyn ReleaseService;
}
