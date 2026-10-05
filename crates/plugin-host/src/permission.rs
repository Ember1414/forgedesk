//! 插件权限白名单（T6.1）。
//!
//! # 为什么是封闭枚举而不是自由字符串
//!
//! 权限是安全模型的根：宿主函数入口的每一次校验（T6.2）、授权对话框的逐项展示（T6.4）
//! 都依赖"权限集合是有限且已知的"。开放字符串意味着一个拼写错误（`net:gihub`）
//! 就可能静默变成"请求了一个谁也不认识的权限"，要么被意外放行，要么让用户授权
//! 一个语义不明的条目。因此清单里的权限字符串在此处显式解析，白名单之外的值
//! 一律拒绝加载，错误里带上原始字符串供 UI 展示。
//!
//! # 为什么本模块不写权限的人类可读说明
//!
//! 说明文案是用户可见文案，必须走前端 i18n（CODING_STYLE §3.4）；后端只提供
//! 稳定的权限标识（[`Permission::as_str`]）与危险级别（[`Permission::is_dangerous`]），
//! 由前端映射为本地化描述。

use serde::{Deserialize, Serialize};

/// 插件可申请的全部权限（白名单，T6.1 定义，共 10 项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, schemars::JsonSchema)]
pub enum Permission {
    /// 只读访问用户显式选择的仓库目录。
    #[serde(rename = "fs:read")]
    FsRead,
    /// 修改用户显式选择的仓库目录中的文件。
    #[serde(rename = "fs:write")]
    FsWrite,
    /// 读取仓库状态与历史（status / log / 分支等）。
    #[serde(rename = "git:read")]
    GitRead,
    /// 执行改变仓库状态的 Git 操作（stage / commit，必须经 SnapshotManager 与审计）。
    #[serde(rename = "git:write")]
    GitWrite,
    /// 通过宿主 HTTP 客户端访问 api.github.com（走用户代理与限流，WASI 本身无网络）。
    #[serde(rename = "net:github")]
    NetGithub,
    /// 在界面中注册面板（sidebar / bottom / repo-tab）。
    #[serde(rename = "ui:panel")]
    UiPanel,
    /// 向命令面板注册命令。
    #[serde(rename = "ui:command")]
    UiCommand,
    /// 弹出 toast 通知。
    #[serde(rename = "ui:toast")]
    UiToast,
    /// 读取插件命名空间下的设置项。
    #[serde(rename = "settings:read")]
    SettingsRead,
    /// 写入插件命名空间下的设置项。
    #[serde(rename = "settings:write")]
    SettingsWrite,
}

impl Permission {
    /// 全部白名单权限，按稳定顺序排列（供文档与授权 UI 枚举）。
    pub const ALL: [Permission; 10] = [
        Permission::FsRead,
        Permission::FsWrite,
        Permission::GitRead,
        Permission::GitWrite,
        Permission::NetGithub,
        Permission::UiPanel,
        Permission::UiCommand,
        Permission::UiToast,
        Permission::SettingsRead,
        Permission::SettingsWrite,
    ];

    /// 清单与 API 中使用的稳定字符串标识。
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::FsRead => "fs:read",
            Permission::FsWrite => "fs:write",
            Permission::GitRead => "git:read",
            Permission::GitWrite => "git:write",
            Permission::NetGithub => "net:github",
            Permission::UiPanel => "ui:panel",
            Permission::UiCommand => "ui:command",
            Permission::UiToast => "ui:toast",
            Permission::SettingsRead => "settings:read",
            Permission::SettingsWrite => "settings:write",
        }
    }

    /// 是否为危险权限（授权对话框需要醒目样式 + 额外确认，T6.4）。
    ///
    /// `net:*` 目前只有 `net:github` 一项；未来新增 net 域权限时必须归入危险级。
    pub fn is_dangerous(self) -> bool {
        matches!(
            self,
            Permission::FsWrite | Permission::GitWrite | Permission::NetGithub
        )
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for Permission {
    type Error = crate::manifest::ManifestError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Permission::ALL
            .iter()
            .find(|p| p.as_str() == value)
            .copied()
            .ok_or_else(|| crate::manifest::ManifestError::UnknownPermission(value.to_owned()))
    }
}

impl TryFrom<String> for Permission {
    type Error = crate::manifest::ManifestError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Permission::try_from(value.as_str())
    }
}

impl Serialize for Permission {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Permission {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Permission::try_from(raw.as_str()).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn every_permission_round_trips_through_its_stable_string() {
        for permission in Permission::ALL {
            let parsed = Permission::try_from(permission.as_str()).unwrap();
            assert_eq!(parsed, permission);
        }
    }

    #[test]
    fn a_permission_outside_the_whitelist_is_rejected_with_the_original_value() {
        // 越权用例：请求一个白名单外的权限，必须被拒绝而不是被静默忽略
        let error = Permission::try_from("fs:admin").unwrap_err();
        assert!(
            error.to_string().contains("fs:admin"),
            "错误信息应包含原始权限字符串，便于 UI 展示: {error}"
        );
    }

    #[test]
    fn only_write_and_network_permissions_are_marked_dangerous() {
        assert!(Permission::FsWrite.is_dangerous());
        assert!(Permission::GitWrite.is_dangerous());
        assert!(Permission::NetGithub.is_dangerous());
        assert!(!Permission::FsRead.is_dangerous());
        assert!(!Permission::GitRead.is_dangerous());
        assert!(!Permission::UiToast.is_dangerous());
        assert!(!Permission::SettingsRead.is_dangerous());
    }

    #[test]
    fn the_whitelist_has_exactly_the_ten_documented_permissions() {
        assert_eq!(Permission::ALL.len(), 10);
        let strings: Vec<_> = Permission::ALL.iter().map(|p| p.as_str()).collect();
        for expected in [
            "fs:read",
            "fs:write",
            "git:read",
            "git:write",
            "net:github",
            "ui:panel",
            "ui:command",
            "ui:toast",
            "settings:read",
            "settings:write",
        ] {
            assert!(
                strings.contains(&expected),
                "白名单缺少文档承诺的权限 {expected}"
            );
        }
    }
}
