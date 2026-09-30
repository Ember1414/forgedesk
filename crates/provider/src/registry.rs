//! Provider 注册表：把 host / 远端 URL 绑定到具体的托管平台实现。
//!
//! # 为什么需要注册表
//!
//! 同一个 provider 家族覆盖多个 host（`github.com`、`*.ghe.com`、自建 GHE）；
//! 而自建 Gitea / GHE Server 的 host 只有用户自己知道。注册表把
//! "host → [`ProviderId`]"这件事集中在一处：内置规则来自
//! [`forgedesk_domain::url::provider_for_host`]（与凭据分组共享同一套判定），
//! 用户配置的企业主机通过 [`ProviderRegistry::bind_host`] 追加。
//!
//! # 为什么 `resolve` 返回 `Option`
//!
//! 大多数 Git 远端不属于任何已实现的平台（`generic`）。返回 `None` 让调用方
//! 走"纯 Git、无平台功能"的路径；**绝不**默认猜一个 provider——
//! 猜错的代价是界面上出现一个点进去全是 404 的 GitHub 面板。

use std::collections::BTreeMap;

use forgedesk_domain::url::{parse_remote_url, provider_for_host, RemoteEndpoint};

use crate::model::ProviderId;

/// 一条远端 URL 的解析结果：归属的 provider 与原始端点。
///
/// `endpoint` 保留完整信息（协议、端口、路径），后续的凭据绑定（T4.4）
/// 与 clone 联动（T4.5）都需要它，而不是只留 owner/repo。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRemote {
    /// 该远端归属的托管平台。
    pub provider: ProviderId,
    /// 原始端点（协议、host、端口、路径）。
    pub endpoint: RemoteEndpoint,
}

/// host → provider 的绑定表。
///
/// 克隆廉价（内部只有一张 `BTreeMap`），services 层可以按仓库持有实例；
/// 也可作为共享单例。查询无副作用、无 IO，纯内存。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderRegistry {
    /// 用户显式配置的企业主机（自建 GHE / Gitea / Forgejo 等）。
    ///
    /// key 为 host（可带端口，如 `git.internal:8443`），value 为用户声明的归属。
    configured: BTreeMap<String, ProviderId>,
}

impl ProviderRegistry {
    /// 只带内置规则的空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一条用户配置的 host 绑定（构建者风格，便于启动时从设置装载）。
    ///
    /// 配置优先于内置规则：用户显式声明 `git.internal → gitea` 时，
    /// 即使内置规则另有判断，也以用户为准。
    #[must_use]
    pub fn bind_host(mut self, host: impl Into<String>, provider: ProviderId) -> Self {
        self.configured
            .insert(host.into().to_ascii_lowercase(), provider);
        self
    }

    /// 解析一个 host（可带端口）。
    ///
    /// 匹配顺序：配置的完整 host（含端口）→ 配置的裸 host（去端口）→
    /// 内置规则。带端口优先，因为同机不同端口可能是两套服务。
    #[must_use]
    pub fn resolve_host(&self, host: &str) -> Option<ProviderId> {
        let host = host.trim().to_ascii_lowercase();
        if host.is_empty() {
            return None;
        }
        if let Some(provider) = self.configured.get(&host) {
            return Some(*provider);
        }
        let bare = host.split(':').next().unwrap_or(&host);
        if bare != host {
            if let Some(provider) = self.configured.get(bare) {
                return Some(*provider);
            }
        }
        match provider_for_host(bare) {
            "github" => Some(ProviderId::GitHub),
            "gitlab" => Some(ProviderId::GitLab),
            // bitbucket / codeberg 已被凭据分组识别，但尚无 provider 实现：
            // 归 None（无平台面板），等实现落地再绑定，避免空壳功能
            _ => None,
        }
    }

    /// 解析一条远端 URL（git 的四种形态都支持，见 [`parse_remote_url`]）。
    #[must_use]
    pub fn resolve_remote(&self, url: &str) -> Option<ResolvedRemote> {
        let endpoint = parse_remote_url(url)?;
        let provider = self.resolve_host(&endpoint.credential_host()?)?;
        Some(ResolvedRemote { provider, endpoint })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::ProviderRegistry;
    use crate::model::ProviderId;
    use forgedesk_domain::url::RemoteScheme;

    /// docs/ARCHITECTURE.md §5 的要求：注册表必须用表驱动用例覆盖
    /// ≥ 15 种 URL 变体——git 远端的写法比直觉多，漏一种就是一类用户进不来。
    #[test]
    fn remote_urls_across_every_supported_shape_resolve_to_the_right_provider() {
        let registry = ProviderRegistry::new();
        // (URL, 期望归属)：None = 已识别但不属于任何已实现平台
        let cases: [(&str, Option<ProviderId>); 16] = [
            (
                "https://github.com/octocat/Hello-World.git",
                Some(ProviderId::GitHub),
            ),
            (
                "git@github.com:octocat/Hello-World.git",
                Some(ProviderId::GitHub),
            ),
            (
                "ssh://git@github.com:2222/octocat/repo.git",
                Some(ProviderId::GitHub),
            ),
            (
                "git://github.com/octocat/repo.git",
                Some(ProviderId::GitHub),
            ),
            (
                "https://GITHUB.com/octocat/repo.git",
                Some(ProviderId::GitHub),
            ),
            (
                "https://octocat@github.com/octocat/repo.git",
                Some(ProviderId::GitHub),
            ),
            (
                "https://github.com:8443/octocat/repo.git",
                Some(ProviderId::GitHub),
            ),
            (
                "https://acme.ghe.com/acme/repo.git",
                Some(ProviderId::GitHub),
            ),
            (
                "https://ghe.acme.github.com/acme/repo.git",
                Some(ProviderId::GitHub),
            ),
            (
                "https://gitlab.com/gitlab-org/gitlab.git",
                Some(ProviderId::GitLab),
            ),
            ("git@gitlab.com:group/project.git", Some(ProviderId::GitLab)),
            ("https://bitbucket.org/team/repo.git", None),
            ("https://codeberg.org/user/repo.git", None),
            ("https://git.internal:8443/team/repo.git", None),
            ("/srv/git/repo.git", None),
            ("C:/repos/repo.git", None),
        ];
        for (url, expected) in cases {
            assert_eq!(
                registry.resolve_remote(url).map(|r| r.provider),
                expected,
                "URL 归属判断错误：{url}"
            );
        }
    }

    #[test]
    fn configured_hosts_take_precedence_over_builtin_rules() {
        let registry = ProviderRegistry::new().bind_host("git.internal", ProviderId::Gitea);

        // 裸 host 与带端口 host 都命中同一条配置（同机不同端口按同一服务处理）
        assert_eq!(
            registry.resolve_host("git.internal"),
            Some(ProviderId::Gitea)
        );
        assert_eq!(
            registry.resolve_host("git.internal:8443"),
            Some(ProviderId::Gitea)
        );
        // 配置也参与 URL 解析
        let remote = registry
            .resolve_remote("https://git.internal/team/repo.git")
            .unwrap();
        assert_eq!(remote.provider, ProviderId::Gitea);
    }

    #[test]
    fn a_configured_host_with_a_port_only_matches_that_port_when_the_bare_host_is_unbound() {
        // 用户精确写了端口：8443 是 Gitea，443（不带端口）落在内置规则 → None
        let registry = ProviderRegistry::new().bind_host("git.internal:8443", ProviderId::Gitea);

        assert_eq!(
            registry.resolve_host("git.internal:8443"),
            Some(ProviderId::Gitea)
        );
        assert_eq!(registry.resolve_host("git.internal"), None);
    }

    #[test]
    fn configured_bindings_win_over_the_builtin_github_rule() {
        // 用户把 github.com 指到自建代理：显式配置压过内置规则
        let registry = ProviderRegistry::new().bind_host("github.com", ProviderId::Gitea);

        assert_eq!(registry.resolve_host("github.com"), Some(ProviderId::Gitea));
    }

    #[test]
    fn resolve_remote_keeps_the_endpoint_for_downstream_credential_binding() {
        let registry = ProviderRegistry::new();
        let remote = registry
            .resolve_remote("ssh://git@github.com:2222/octocat/repo.git")
            .unwrap();

        assert_eq!(remote.provider, ProviderId::GitHub);
        assert_eq!(remote.endpoint.scheme, RemoteScheme::Ssh);
        assert_eq!(remote.endpoint.port, Some(2222));
        assert_eq!(remote.endpoint.path, "octocat/repo.git");
    }

    #[test]
    fn junk_input_resolves_to_nothing_instead_of_a_guessed_provider() {
        let registry = ProviderRegistry::new();

        assert_eq!(registry.resolve_host(""), None);
        assert_eq!(registry.resolve_host("   "), None);
        assert!(registry.resolve_remote("").is_none());
        assert!(registry.resolve_remote("not a url").is_none());
    }
}
