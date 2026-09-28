//! 远端 URL 的解析：从 URL 里取出"要拿哪个账号去登录"。
//!
//! # 为什么必须自己解析
//!
//! 凭据是按 **host** 存的（`github.com` 与 `gitlab.com` 是两条不同的凭据），
//! 而远端给我们的是一串 URL。git 支持的 URL 形态有四种写法（见下），
//! 其中 scp 风格（`git@github.com:owner/repo.git`）**不是**合法 URL，
//! 用 `url::Url` 解析会失败——因此这里自己切分，并且只用纯字符串逻辑（可单测）。
//!
//! ```text
//! https://github.com/owner/repo.git        → scheme https, host github.com
//! http://host:8080/owner/repo.git           → scheme http,  host host, port 8080
//! git@github.com:owner/repo.git             → scp 风格，   host github.com, user git
//! ssh://git@github.com:2222/owner/repo.git  → scheme ssh,  host github.com, port 2222
//! git://github.com/owner/repo.git           → 只读协议
//! file:///C:/repos/x.git, /srv/git/x.git    → 本地路径（不需要凭据）
//! ```
//!
//! # 与"凭据引用"的关系
//!
//! [`RemoteEndpoint::credential_host`] 给出该用它去查凭据的 host 字符串；
//! [`provider_for_host`] 推断 provider。两者合起来就是 [`crate::CredentialRef`] 的
//! provider + host 两段，login 由账号模型或用户输入补上。

use serde::{Deserialize, Serialize};

/// 远端 URL 的协议类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteScheme {
    /// HTTPS（走凭据库里的令牌/密码）。
    Https,
    /// 明文 HTTP（自建服务常见；同样需要凭据）。
    Http,
    /// SSH（走密钥或 agent，**不**经过凭据库里的令牌）。
    Ssh,
    /// `git://`（只读协议，无认证，已被多数托管平台关闭）。
    Git,
    /// 本地路径或 `file://`（没有主机，不需要凭据）。
    File,
}

/// 解析后的远端端点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEndpoint {
    /// 协议类别。
    pub scheme: RemoteScheme,
    /// 主机名（小写；本地路径为 `None`）。
    pub host: Option<String>,
    /// 端口（URL 里显式给出时）。
    pub port: Option<u16>,
    /// URL 里的用户名（`git@` 的 `git`、`https://user@host/` 的 `user`）。
    pub user: Option<String>,
    /// 仓库路径（`owner/repo.git`）。
    pub path: String,
}

impl RemoteEndpoint {
    /// 去查凭据时该用哪个 host。
    ///
    /// 规则：带端口的 HTTP(S) 用 `host:port` 作为凭据的 host 段——
    /// 同一台机器上的 8443 与 443 很可能是两个不同的服务、两套账号。
    /// SSH 不带端口进 host（SSH 的账号由密钥决定，端口不影响"用哪个 key"）。
    pub fn credential_host(&self) -> Option<String> {
        let host = self.host.as_ref()?;
        match (self.scheme, self.port) {
            (RemoteScheme::Https | RemoteScheme::Http, Some(port)) => {
                Some(format!("{host}:{port}"))
            }
            _ => Some(host.clone()),
        }
    }

    /// 这个远端是否需要凭据（本地路径不需要）。
    pub fn needs_credentials(&self) -> bool {
        matches!(
            self.scheme,
            RemoteScheme::Https | RemoteScheme::Http | RemoteScheme::Ssh
        )
    }

    /// 是否走 SSH（凭据库里存的令牌对它没用）。
    pub fn is_ssh(&self) -> bool {
        self.scheme == RemoteScheme::Ssh
    }
}

/// 解析远端 URL；无法识别时返回 `None`（宁可说"不认识"，也不要猜出一个错的主机名）。
pub fn parse_remote_url(raw: &str) -> Option<RemoteEndpoint> {
    let url = raw.trim();
    if url.is_empty() {
        return None;
    }

    // 1) 带 "://" 的标准形态
    if let Some((scheme, rest)) = url.split_once("://") {
        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "https" => RemoteScheme::Https,
            "http" => RemoteScheme::Http,
            "ssh" | "git+ssh" => RemoteScheme::Ssh,
            "git" => RemoteScheme::Git,
            "file" => {
                return Some(RemoteEndpoint {
                    scheme: RemoteScheme::File,
                    host: None,
                    port: None,
                    user: None,
                    path: trim_slashes(rest),
                });
            }
            // Windows 盘符形式（file:C:/x 不带 //）会被 split_once 漏到这里
            _ => return None,
        };

        // rest = [user@]host[:port]/path
        let (authority, path) = match rest.split_once('/') {
            Some((authority, path)) => (authority, path.to_owned()),
            None => (rest, String::new()),
        };
        if authority.is_empty() {
            return None;
        }

        let (user, host_port) = match authority.rsplit_once('@') {
            Some((user, host)) => (Some(user.to_owned()), host),
            None => (None, authority),
        };

        // 端口只在**非 IPv6** 的主机上用最后一个 ':' 判断（IPv6 字面量形如 [::1]:22）
        let (host, port) = if let Some(rest) = host_port.strip_prefix('[') {
            match rest.split_once(']') {
                Some((host, tail)) => (
                    host.to_owned(),
                    tail.strip_prefix(':').and_then(|p| p.parse::<u16>().ok()),
                ),
                None => (host_port.to_owned(), None),
            }
        } else {
            match host_port.rsplit_once(':') {
                Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => {
                    (host.to_owned(), port.parse::<u16>().ok())
                }
                // 端口非法（`host:abc`）时当作主机名的一部分是不对的：直接判为不认识
                Some(_) => return None,
                None => (host_port.to_owned(), None),
            }
        };

        if host.is_empty() {
            return None;
        }
        return Some(RemoteEndpoint {
            scheme,
            host: Some(host.to_ascii_lowercase()),
            port,
            user,
            path: path.trim_end_matches('/').to_owned(),
        });
    }

    // 2) scp 风格：git@github.com:owner/repo.git
    if let Some((before_path, path)) = split_scp(url) {
        let (user, host) = match before_path.rsplit_once('@') {
            Some((user, host)) => (Some(user.to_owned()), host.to_owned()),
            None => (None, before_path.to_owned()),
        };
        if host.is_empty() || path.is_empty() {
            return None;
        }
        return Some(RemoteEndpoint {
            scheme: RemoteScheme::Ssh,
            host: Some(host.to_ascii_lowercase()),
            port: None,
            user,
            path: path.trim_end_matches('/').to_owned(),
        });
    }

    // 3) 本地路径（绝对路径或相对路径）
    Some(RemoteEndpoint {
        scheme: RemoteScheme::File,
        host: None,
        port: None,
        user: None,
        path: trim_slashes(url),
    })
}

/// scp 风格的分割点：**第一个** `:`，但它必须在最后一个 `/` 之前，
/// 否则会把 `/srv/git:x` 这种本地路径当成 scp 形式。
///
/// 还要排除 Windows 盘符：`C:/repos/repo.git` 里 `:` 同样在 `/` 之前，
/// 但它显然不是"主机 C 上的 /repos/repo.git"。
fn split_scp(raw: &str) -> Option<(&str, &str)> {
    let colon = raw.find(':')?;
    let last_slash = raw.rfind('/')?;
    if colon > last_slash {
        return None;
    }
    let (before, after) = raw.split_at(colon);
    // 主机名至少两个字符：单个字母就是盘符（`C:`、`D:`）。
    // 还要求含字母或数字，避免把 `::/path` 这类输入当成 scp。
    if before.len() < 2 || !before.chars().any(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some((before, &after[1..]))
}

fn trim_slashes(raw: &str) -> String {
    raw.trim_start_matches("//").to_owned()
}

/// 从 host 推断 provider 标识。
///
/// 用途：凭据的 provider 段（`github:github.com:octocat`）要与托管平台对上，
/// 便于设置页分组与将来的账号模型（T4.4）。认不出的一律 `generic`——
/// 猜一个错的 provider 会让用户在设置页里找不到自己刚保存的凭据。
pub fn provider_for_host(host: &str) -> &'static str {
    let host = host.to_ascii_lowercase();
    // 去掉端口再比对（`github.com:8443` 也应按 github 处理）
    let bare = host.split(':').next().unwrap_or(&host);
    if bare == "github.com" || bare.ends_with(".github.com") || bare == "github.com.cnpmjs.org" {
        "github"
    } else if bare == "gitlab.com" || bare.ends_with(".gitlab.com") {
        "gitlab"
    } else if bare == "bitbucket.org" || bare.ends_with(".bitbucket.org") {
        "bitbucket"
    } else if bare == "codeberg.org" || bare.ends_with(".codeberg.org") {
        "codeberg"
    } else {
        "generic"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> RemoteEndpoint {
        parse_remote_url(raw).unwrap_or_else(|| panic!("应当可解析：{raw}"))
    }

    #[test]
    fn an_https_remote_keeps_its_host_in_lower_case() {
        let endpoint = parse("https://GitHub.com/octocat/Hello-World.git");

        assert_eq!(endpoint.scheme, RemoteScheme::Https);
        assert_eq!(endpoint.host.as_deref(), Some("github.com"));
        assert_eq!(endpoint.path, "octocat/Hello-World.git");
        assert_eq!(endpoint.credential_host().as_deref(), Some("github.com"));
        assert!(endpoint.needs_credentials());
        assert!(!endpoint.is_ssh());
    }

    #[test]
    fn an_https_remote_with_a_port_uses_host_and_port_as_the_credential_key() {
        // 同一台机器上的 8443 与 443 可能是两套账号
        let endpoint = parse("http://git.internal:8080/team/repo.git");

        assert_eq!(endpoint.scheme, RemoteScheme::Http);
        assert_eq!(endpoint.host.as_deref(), Some("git.internal"));
        assert_eq!(endpoint.port, Some(8080));
        assert_eq!(
            endpoint.credential_host().as_deref(),
            Some("git.internal:8080")
        );
    }

    #[test]
    fn a_url_with_a_user_keeps_it_separate_from_the_host() {
        let endpoint = parse("https://octocat@github.com/octocat/repo.git");

        assert_eq!(endpoint.user.as_deref(), Some("octocat"));
        assert_eq!(endpoint.host.as_deref(), Some("github.com"));
    }

    #[test]
    fn an_scp_style_remote_is_understood_even_though_it_is_not_a_valid_url() {
        let endpoint = parse("git@github.com:octocat/Hello-World.git");

        assert_eq!(endpoint.scheme, RemoteScheme::Ssh);
        assert_eq!(endpoint.host.as_deref(), Some("github.com"));
        assert_eq!(endpoint.user.as_deref(), Some("git"));
        assert_eq!(endpoint.path, "octocat/Hello-World.git");
        assert!(endpoint.is_ssh());
        // SSH 的凭据不由 host 段决定，端口也就不进 host
        assert_eq!(endpoint.credential_host().as_deref(), Some("github.com"));
    }

    #[test]
    fn an_ssh_url_with_a_port_is_understood() {
        let endpoint = parse("ssh://git@github.com:2222/octocat/repo.git");

        assert_eq!(endpoint.scheme, RemoteScheme::Ssh);
        assert_eq!(endpoint.host.as_deref(), Some("github.com"));
        assert_eq!(endpoint.port, Some(2222));
        assert_eq!(endpoint.path, "octocat/repo.git");
    }

    #[test]
    fn local_paths_are_recognised_and_need_no_credentials() {
        for raw in [
            "/srv/git/repo.git",
            "C:/repos/repo.git",
            "file:///srv/git/repo.git",
            "../sibling/repo",
        ] {
            let endpoint = parse(raw);
            assert_eq!(endpoint.scheme, RemoteScheme::File, "{raw}");
            assert!(!endpoint.needs_credentials(), "{raw}");
            assert_eq!(endpoint.credential_host(), None, "{raw}");
        }
    }

    #[test]
    fn a_windows_path_with_a_drive_letter_is_not_mistaken_for_an_scp_remote() {
        // 反例：C:/repos/x 里第一个 ':' 在最后一个 '/' 之前，但它是盘符不是 scp 分隔符
        let endpoint = parse("C:/repos/repo.git");

        assert_eq!(endpoint.scheme, RemoteScheme::File);
        assert_eq!(endpoint.host, None);
    }

    #[test]
    fn a_path_that_only_looks_like_an_scp_remote_is_still_treated_as_a_local_path() {
        // '/srv/git:x' 的 ':' 在最后一个 '/' 之后 → 不是 scp 形式
        let endpoint = parse("/srv/git:x");

        assert_eq!(endpoint.scheme, RemoteScheme::File);
    }

    #[test]
    fn an_ipv6_literal_host_is_parsed_without_treating_its_colons_as_ports() {
        let endpoint = parse("https://[::1]:8443/team/repo.git");

        assert_eq!(endpoint.host.as_deref(), Some("::1"));
        assert_eq!(endpoint.port, Some(8443));
    }

    #[test]
    fn junk_input_is_refused_instead_of_guessing_a_host() {
        assert!(parse_remote_url("").is_none());
        assert!(parse_remote_url("   ").is_none());
        // 非法端口：宁可拒绝，也不要把它当主机名的一部分去查凭据
        assert!(parse_remote_url("https://github.com:not-a-port/x.git").is_none());
        // 未知协议
        assert!(parse_remote_url("gopher://github.com/x").is_none());
    }

    #[test]
    fn the_git_protocol_is_listed_as_such_because_it_carries_no_credentials() {
        let endpoint = parse("git://github.com/octocat/repo.git");

        assert_eq!(endpoint.scheme, RemoteScheme::Git);
        assert_eq!(endpoint.host.as_deref(), Some("github.com"));
        // `git://` 是匿名只读协议：给它注入凭据只会在握手时被拒
        assert!(!endpoint.needs_credentials());
        assert!(!endpoint.is_ssh());
    }

    #[test]
    fn providers_are_inferred_from_the_host_so_credentials_group_correctly() {
        assert_eq!(provider_for_host("github.com"), "github");
        assert_eq!(provider_for_host("GitHub.com"), "github");
        assert_eq!(provider_for_host("github.com:8443"), "github");
        assert_eq!(provider_for_host("ghe.github.com"), "github");
        assert_eq!(provider_for_host("gitlab.com"), "gitlab");
        assert_eq!(provider_for_host("bitbucket.org"), "bitbucket");
        assert_eq!(provider_for_host("codeberg.org"), "codeberg");
        // 认不出就通用：猜错 provider 会让用户找不到刚保存的凭据
        assert_eq!(provider_for_host("git.internal"), "generic");
    }
}
