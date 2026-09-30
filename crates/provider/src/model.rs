//! Provider 的基础标识模型：`ProviderId` 与能力声明 `ProviderCapabilities`。
//!
//! # 为什么能力是"声明"而不是"判断"
//!
//! UI 必须依据 [`ProviderCapabilities`] 动态显示/隐藏功能（如 Gitea 无 Actions
//! 就隐藏 Actions 标签页），**禁止**按 provider 名字写 if 分支（docs/ARCHITECTURE.md §5）。
//! 这样接入一个新 HostProvider 时，前端零改动；漏实现的子服务在界面层就不可见，
//! 而不是运行时才报错。

use serde::{Deserialize, Serialize};

/// 托管平台标识。
///
/// 序列化为小写字符串（`"github"`）；新增成员属于**前后端契约变更**，
/// 前端的 provider 名称映射必须同步（T4.4 账号 UI 落地时一并登记）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ProviderId {
    /// GitHub（github.com、GitHub Enterprise Cloud/Server）。
    #[serde(rename = "github")]
    GitHub,
    /// GitLab（gitlab.com 与自建实例；实现待后续里程碑）。
    #[serde(rename = "gitlab")]
    GitLab,
    /// Gitea / Forgejo（自建实例；实现待后续里程碑）。
    #[serde(rename = "gitea")]
    Gitea,
}

impl ProviderId {
    /// 稳定字符串形式（与 serde 序列化一致）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::GitLab => "gitlab",
            Self::Gitea => "gitea",
        }
    }

    /// 从稳定字符串解析（大小写不敏感）。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "github" => Some(Self::GitHub),
            "gitlab" => Some(Self::GitLab),
            "gitea" | "forgejo" => Some(Self::Gitea),
            _ => None,
        }
    }

    /// 全部成员（用于遍历与前端枚举同步校验）。
    pub const ALL: &'static [Self] = &[Self::GitHub, Self::GitLab, Self::Gitea];
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一个 HostProvider 声明自己支持哪些子服务。
///
/// 布尔位而不是选项枚举：UI 的显示/隐藏判断是逐位的，嵌套枚举只会让
/// 前端多一层拆包。未来出现"部分支持"（如 GitLab 的 CI 与 GitHub 的
/// Actions 形态不同）时，再加更细的字段而不是改字段语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    /// Pull Request 列表与详情（[`crate::PullService`]）。
    pub pulls: bool,
    /// Issue 列表与详情（[`crate::IssueService`]）。
    pub issues: bool,
    /// CI 工作流与运行记录（[`crate::CiService`]）。
    pub actions: bool,
    /// Release 列表与产物（[`crate::ReleaseService`]）。
    pub releases: bool,
    /// Gist（GitHub 专属；其他平台无对应物）。
    pub gists: bool,
    /// GraphQL 端点可用（GitHub 有；Gitea 无）。
    pub graphql: bool,
    /// Commit/PR 状态检查（checks：CI 结果的统一视图）。
    pub checks: bool,
}

impl ProviderCapabilities {
    /// 全部不支持（未实现的子服务先用它占位）。
    pub const NONE: Self = Self {
        pulls: false,
        issues: false,
        actions: false,
        releases: false,
        gists: false,
        graphql: false,
        checks: false,
    };

    /// GitHub 的能力：PLAN M4 的全部子服务都支持。
    pub const GITHUB: Self = Self {
        pulls: true,
        issues: true,
        actions: true,
        releases: true,
        gists: true,
        graphql: true,
        checks: true,
    };
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{ProviderCapabilities, ProviderId};

    #[test]
    fn provider_id_serializes_to_the_stable_lowercase_form() {
        for id in ProviderId::ALL {
            assert_eq!(
                serde_json::to_string(id).unwrap(),
                format!("\"{}\"", id.as_str()),
                "{id:?} 的 serde 形式与 as_str 不一致"
            );
            assert_eq!(ProviderId::parse(id.as_str()), Some(*id));
        }
    }

    #[test]
    fn provider_id_parse_is_case_insensitive_and_accepts_forgejo_as_gitea() {
        assert_eq!(ProviderId::parse("GitHub"), Some(ProviderId::GitHub));
        assert_eq!(ProviderId::parse(" gitlab "), Some(ProviderId::GitLab));
        // Forgejo 是 Gitea 的硬分叉，API 兼容：用户不该被迫区分这两者
        assert_eq!(ProviderId::parse("forgejo"), Some(ProviderId::Gitea));
        assert_eq!(ProviderId::parse("sourcehut"), None);
    }

    #[test]
    fn capability_presets_declare_what_each_plan_promised() {
        // GitHub 是 M4 的目标平台：七项能力全开
        assert_eq!(
            ProviderCapabilities::GITHUB,
            ProviderCapabilities {
                pulls: true,
                issues: true,
                actions: true,
                releases: true,
                gists: true,
                graphql: true,
                checks: true,
            }
        );
        // 未实现的子服务占位：七项全关（用整表比较，避免对常量取反的断言）
        assert_eq!(
            ProviderCapabilities::NONE,
            ProviderCapabilities {
                pulls: false,
                issues: false,
                actions: false,
                releases: false,
                gists: false,
                graphql: false,
                checks: false,
            }
        );
    }
}
