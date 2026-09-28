//! 引用模型：分支、标签、远端，以及引用更新结果。

/// 远端 URL 的协议类别。
///
/// 为什么要分类而不是直接用字符串：凭据策略完全由协议决定
/// （HTTPS 走 keyring，SSH 走 agent，本地路径不需要凭据），
/// 而"用 URL 前缀做 if 判断"会散落在多个模块里且各写各的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteKind {
    /// `https://` 或 `http://`。
    Https,
    /// `ssh://` 或 `git@host:path` 形式。
    Ssh,
    /// `git://`（无认证的 git 协议）。
    Git,
    /// 本地文件路径或 `file://`。
    File,
    /// 无法识别的形式。
    Other,
}

impl RemoteKind {
    /// 从远端 URL 推断协议类别。
    ///
    /// 这是**纯字符串逻辑**，放在领域层是为了能单测：`git@github.com:org/repo.git`
    /// 这种 scp 风格写法没有 scheme，只看 `://` 会误判成 File。
    pub fn from_url(url: &str) -> Self {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Self::Other;
        }

        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("https://") || lower.starts_with("http://") {
            return Self::Https;
        }
        if lower.starts_with("ssh://") {
            return Self::Ssh;
        }
        if lower.starts_with("git://") {
            return Self::Git;
        }
        if lower.starts_with("file://") {
            return Self::File;
        }
        // scp 风格：`[user@]host:path`（冒号前没有斜杠）
        if let Some(colon) = trimmed.find(':') {
            let before = &trimmed[..colon];
            let after = &trimmed[colon + 1..];
            // `C:\repo` 与 `C:/repo` 里冒号前是一个字母——那是 Windows 盘符，不是主机名。
            // 反斜杠开头的"路径部分"同样只可能来自本地路径（scp 语法里没有它）。
            let is_windows_drive = before.len() == 1
                && before
                    .chars()
                    .all(|character| character.is_ascii_alphabetic());
            let is_windows_path = is_windows_drive || after.starts_with('\\');

            if !before.is_empty()
                && !before.contains('/')
                && !before.contains('\\')
                && !is_windows_path
            {
                return Self::Ssh;
            }
        }
        // Windows 盘符（`C:\repo`）与 POSIX 路径都落到这里
        Self::File
    }

    /// 该协议是否需要凭据。
    pub const fn needs_credentials(self) -> bool {
        matches!(self, Self::Https | Self::Ssh)
    }
}

/// 一个远端。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    /// 远端名（通常是 `origin`）。
    pub name: String,
    /// fetch 用的 URL。
    pub fetch_url: String,
    /// push 用的 URL；与 fetch 相同（或未单独配置）时为 `None`。
    pub push_url: Option<String>,
    /// 由 fetch URL 推断的协议类别。
    pub kind: RemoteKind,
}

impl Remote {
    /// 实际用于 push 的 URL。
    pub fn effective_push_url(&self) -> &str {
        self.push_url.as_deref().unwrap_or(&self.fetch_url)
    }
}

/// 一个本地或远程跟踪分支。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    /// 分支短名。远程跟踪分支形如 `origin/main`（不含 `refs/remotes/`）。
    pub name: String,
    /// 是否为远程跟踪分支。
    pub is_remote: bool,
    /// 是否为当前 HEAD 指向的分支。
    pub is_head: bool,
    /// 指向的提交 oid。
    pub target: String,
    /// 上游短名（本地分支才有）。
    pub upstream: Option<String>,
    /// 相对上游领先的提交数。
    pub ahead: Option<i64>,
    /// 相对上游落后的提交数。
    pub behind: Option<i64>,
    /// 上游已被删除（`[gone]`）。
    ///
    /// 必须单独表示：它与"没有配置上游"不同——前者要提示用户
    /// "上游没了，可能已被删除"，后者只是本地分支。
    pub upstream_gone: bool,
}

impl Branch {
    /// 是否配置了上游且上游仍然存在。
    pub fn has_live_upstream(&self) -> bool {
        self.upstream.is_some() && !self.upstream_gone
    }
}

/// 一个标签。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    /// 标签名。
    pub name: String,
    /// 标签直接指向的对象 oid（附注标签指向 tag 对象）。
    pub target: String,
    /// 解引用后的提交 oid。轻量标签与 [`Tag::target`] 相同。
    pub commit: Option<String>,
    /// 是否为附注标签（annotated）。
    pub annotated: bool,
    /// 标签信息首行（附注标签才有）。
    pub message: Option<String>,
    /// 打标签时间（Unix 秒）。
    pub created_at: Option<i64>,
}

/// 引用更新的结果类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RefUpdateKind {
    /// 新建引用。
    New,
    /// 更新已有引用。
    Updated,
    /// 删除引用（如 `--prune` 删掉已消失的远程分支）。
    Deleted,
    /// 引用没有变化。
    UpToDate,
    /// 被拒绝（非快进、权限不足、hook 拒绝等）。
    Rejected,
}

/// 一次引用变更的明细。
///
/// 单独建模而不是只回一个布尔：界面需要告诉用户"哪个分支从哪个提交变到了哪个提交"，
/// 而"失败"与"没变化"必须区分——前者要报错，后者不该弹任何提示。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefUpdate {
    /// 引用短名（如 `main`、`origin/main`）。
    pub name: String,
    /// 变更前的 oid；新建时为 `None`。
    pub old_oid: Option<String>,
    /// 变更后的 oid；删除时为 `None`。
    pub new_oid: Option<String>,
    /// 结果类别。
    pub kind: RefUpdateKind,
    /// 被拒绝的原因（原始 stderr 片段，已脱敏）。
    pub reason: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Branch, RefUpdate, RefUpdateKind, Remote, RemoteKind, Tag};

    #[test]
    fn remote_kind_recognises_https_and_http() {
        assert_eq!(
            RemoteKind::from_url("https://github.com/org/repo.git"),
            RemoteKind::Https
        );
        assert_eq!(
            RemoteKind::from_url("http://localhost:3000/repo.git"),
            RemoteKind::Https
        );
        assert!(RemoteKind::Https.needs_credentials());
    }

    #[test]
    fn remote_kind_recognises_scp_style_ssh_without_a_scheme() {
        // 没有 `://`，只看 scheme 会误判成 File
        assert_eq!(
            RemoteKind::from_url("git@github.com:org/repo.git"),
            RemoteKind::Ssh
        );
        assert_eq!(
            RemoteKind::from_url("ssh://git@host:22/org/repo.git"),
            RemoteKind::Ssh
        );
    }

    #[test]
    fn windows_drive_letters_are_files_not_ssh_hosts() {
        // `C:\repo` 里冒号前是盘符，但含反斜杠 —— 不能被当成 scp 主机
        assert_eq!(RemoteKind::from_url(r"C:\repo\bare.git"), RemoteKind::File);
        assert_eq!(RemoteKind::from_url("/srv/git/repo.git"), RemoteKind::File);
        assert_eq!(
            RemoteKind::from_url("file:///srv/git/repo.git"),
            RemoteKind::File
        );
        assert!(!RemoteKind::File.needs_credentials());
    }

    #[test]
    fn scp_style_ssh_with_an_absolute_path_is_still_ssh() {
        // 冒号后面以 `/` 开头在 scp 语法里是合法的（远端绝对路径）
        assert_eq!(
            RemoteKind::from_url("git@host:/srv/git/repo.git"),
            RemoteKind::Ssh
        );
    }

    #[test]
    fn git_protocol_and_empty_urls_are_handled() {
        assert_eq!(RemoteKind::from_url("git://host/repo.git"), RemoteKind::Git);
        assert_eq!(RemoteKind::from_url("   "), RemoteKind::Other);
        assert!(!RemoteKind::Git.needs_credentials());
    }

    #[test]
    fn gone_upstream_is_not_the_same_as_no_upstream() {
        let gone = Branch {
            name: "main".to_owned(),
            is_remote: false,
            is_head: true,
            target: "abc".to_owned(),
            upstream: Some("origin/main".to_owned()),
            ahead: None,
            behind: None,
            upstream_gone: true,
        };
        let plain = Branch {
            upstream: None,
            upstream_gone: false,
            ..gone.clone()
        };

        assert!(!gone.has_live_upstream());
        assert!(!plain.has_live_upstream());
        assert!(
            gone.upstream.is_some(),
            "gone 分支仍然记得上游名，便于提示用户"
        );
    }

    #[test]
    fn lightweight_tag_points_at_the_commit_itself() {
        let tag = Tag {
            name: "v1.0.0".to_owned(),
            target: "abc".to_owned(),
            commit: Some("abc".to_owned()),
            annotated: false,
            message: None,
            created_at: None,
        };

        assert_eq!(tag.commit.as_deref(), Some(tag.target.as_str()));
        assert!(!tag.annotated);
    }

    #[test]
    fn ref_types_serialise_in_the_documented_camel_case_shape() {
        // 契约（docs/API.md §1）：DTO 字段一律 camelCase，前端 `src/lib/ipc/*.ts`
        // 就是按这个形状声明的。漏掉 `rename_all` 时，字段在界面上全是 undefined
        // ——列表照样渲染，但"当前分支标记 / ahead-behind 徽标 / 远端 URL"全是空的，
        // 视觉验收根本看不出来。这条测试把形状钉死。
        let branch = Branch {
            name: "main".to_owned(),
            is_remote: false,
            is_head: true,
            target: "abc".to_owned(),
            upstream: Some("origin/main".to_owned()),
            ahead: Some(2),
            behind: Some(1),
            upstream_gone: false,
        };
        let json = serde_json::to_value(&branch).unwrap();
        assert_eq!(json["isHead"], true, "{json}");
        assert_eq!(json["upstream"], "origin/main");
        assert!(json.get("is_head").is_none(), "不能是 snake_case：{json}");

        let remote = Remote {
            name: "origin".to_owned(),
            fetch_url: "https://example.com/r.git".to_owned(),
            push_url: None,
            kind: RemoteKind::Https,
        };
        let json = serde_json::to_value(&remote).unwrap();
        assert_eq!(json["fetchUrl"], "https://example.com/r.git");
        assert_eq!(json["kind"], "https", "枚举值也要 camelCase：{json}");

        let update = RefUpdate {
            name: "origin/main".to_owned(),
            old_oid: Some("a".to_owned()),
            new_oid: Some("b".to_owned()),
            kind: RefUpdateKind::Updated,
            reason: None,
        };
        let json = serde_json::to_value(&update).unwrap();
        assert_eq!(json["oldOid"], "a");
        assert_eq!(json["newOid"], "b");
        assert_eq!(json["kind"], "updated");

        let tag = Tag {
            name: "v1.0.0".to_owned(),
            target: "abc".to_owned(),
            commit: Some("abc".to_owned()),
            annotated: false,
            message: None,
            created_at: Some(1_700_000_000),
        };
        assert_eq!(
            serde_json::to_value(&tag).unwrap()["createdAt"],
            1_700_000_000_i64
        );
    }
}
