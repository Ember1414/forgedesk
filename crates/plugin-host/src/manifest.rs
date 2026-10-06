//! `plugin.json` 清单规范：解析、校验与 JSON Schema 生成（T6.1）。
//!
//! # 设计要点
//!
//! - **parse 即校验**：[`PluginManifest::parse`] 返回 [`ValidatedManifest`]，
//!   调用方拿到的清单必然已通过全部规则，不存在"忘了 validate"的路径。
//!   原始字段仍可通过 Deref 读取。
//! - **apiVersion 主版本不匹配拒绝加载**（T6.1/T6.2 共同约定）；次版本只允许
//!   "宿主 ≥ 插件"，因为 MINOR 升级承诺只是"只增不改"，新次版本插件可能调用
//!   旧宿主没有的函数，所以插件 MINOR 高于宿主时也必须拒绝。
//! - **`main` 是文件名而非路径**：插件随目录分发，wasm 入口固定在其安装目录内；
//!   拒绝路径分隔符与 `..`，杜绝清单把加载器引向插件目录之外（manifest 作为
//!   攻击面的第一道关）。
//! - 1.0 之前（apiVersion `0.x`）不承诺任何兼容性，文档与 schema 中都要体现。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 宿主支持的插件 API 主版本。不等于此值一律拒绝加载。
pub const SUPPORTED_API_MAJOR: u32 = 0;
/// 宿主支持的插件 API 次版本。插件声明的次版本必须 ≤ 此值。
pub const SUPPORTED_API_MINOR: u32 = 1;

/// 清单校验错误。`message` 英文（开发者可检索），原始值随错误携带供 UI 展示。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    /// JSON 解析失败（含未知字段、类型不符、白名单外权限）。
    #[error("manifest is not valid JSON with the expected shape: {0}")]
    Parse(String),
    /// `id` 不是合法的反向域名风格标识。
    #[error("invalid plugin id `{0}`: expected reverse-domain style like `com.example.plugin`")]
    InvalidId(String),
    /// `version` 不是合法的 SemVer（MAJOR.MINOR.PATCH，可带预发布后缀）。
    #[error("invalid plugin version `{0}`: expected semantic version like `1.2.3`")]
    InvalidVersion(String),
    /// `apiVersion` 不是 `MAJOR.MINOR` 形式。
    #[error("invalid apiVersion `{0}`: expected `MAJOR.MINOR` like `0.1`")]
    InvalidApiVersion(String),
    /// `apiVersion` 主版本不匹配（拒绝加载）。
    #[error("apiVersion `{found}` does not match supported major `{supported}`")]
    ApiMajorMismatch {
        /// 插件声明的主次版本。
        found: String,
        /// 宿主支持的主次版本。
        supported: String,
    },
    /// `apiVersion` 次版本高于宿主（插件可能调用宿主没有的函数，拒绝加载）。
    #[error("apiVersion `{found}` is newer than supported `{supported}`")]
    ApiMinorNewer {
        /// 插件声明的主次版本。
        found: String,
        /// 宿主支持的主次版本。
        supported: String,
    },
    /// 权限不在白名单内。
    #[error("unknown permission `{0}`: must be one of the documented whitelist entries")]
    UnknownPermission(String),
    /// `main` 不是安全的 wasm 文件名。
    #[error("invalid main `{0}`: expected a plain `*.wasm` file name inside the plugin directory")]
    InvalidMain(String),
    /// 必填字段为空。
    #[error("field `{0}` must not be empty")]
    EmptyField(&'static str),
    /// 字段超长（防止荒谬清单拖垮 UI 与日志）。
    #[error("field `{field}` exceeds {max} characters")]
    FieldTooLong {
        /// 超长的字段名。
        field: &'static str,
        /// 允许的最大长度。
        max: usize,
    },
    /// `homepage` 存在时必须是 https://（避免把非链接内容塞进 UI）。
    #[error("invalid homepage `{0}`: must be an https:// URL")]
    InvalidHomepage(String),
    /// 贡献点 id 不合法（kebab 风格，最终全名形如 `<plugin-id>.<command-id>`）。
    #[error("invalid contribution id `{0}`: expected kebab-case like `insert-template`")]
    InvalidContributionId(String),
    /// 同一清单内贡献点 id 重复。
    #[error("duplicate contribution id `{0}`")]
    DuplicateContributionId(String),
}

/// `plugin.json` 清单（serde 结构即 schema 来源，T6.1）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PluginManifest {
    /// 插件唯一 id，反向域名风格（如 `com.example.commit-template`）。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// 插件自身版本（SemVer）。
    pub version: String,
    /// 目标插件 API 版本（`MAJOR.MINOR`，当前 `0.1`）。
    pub api_version: String,
    /// 作者。
    pub author: String,
    /// SPDX 许可证标识（合规审计与插件页展示用）。
    pub license: String,
    /// 一句话描述。
    pub description: String,
    /// 主页（可选，必须 https）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// wasm 入口文件名（相对插件目录，必须是普通 `*.wasm` 文件名）。
    pub main: String,
    /// 申请的权限（白名单见 [`crate::permission::Permission`]）。
    #[serde(default)]
    pub permissions: Vec<crate::permission::Permission>,
    /// 贡献点（命令 / 面板）。
    #[serde(default)]
    pub contributes: Contributes,
}

/// 命令与面板贡献点。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Contributes {
    /// 注册到命令面板的命令。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandContribution>,
    /// 注册到界面的面板。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panels: Vec<PanelContribution>,
}

/// 命令贡献点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CommandContribution {
    /// 命令短 id（kebab 风格；全名由宿主拼为 `<plugin-id>.<id>`）。
    pub id: String,
    /// 命令面板中的展示标题（前端 i18n 不覆盖插件文案，由插件自己负责本地化）。
    pub title: String,
    /// 默认快捷键（可选，用户可在设置中改绑）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keybinding: Option<String>,
}

/// 面板贡献点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PanelContribution {
    /// 面板短 id（kebab 风格；全名由宿主拼为 `<plugin-id>.<id>`）。
    pub id: String,
    /// 面板标题。
    pub title: String,
    /// 呈现位置。
    pub location: PanelLocation,
}

/// 面板呈现位置（T6.3 审批项聚焦的是渲染方式，位置集合在此固定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PanelLocation {
    /// 侧栏。
    Sidebar,
    /// 底部面板区。
    Bottom,
    /// 仓库详情中的新增 Tab。
    RepoTab,
}

/// 已通过全部校验的清单（[`PluginManifest::parse`] 的产物）。
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedManifest(PluginManifest);

impl std::ops::Deref for ValidatedManifest {
    type Target = PluginManifest;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PluginManifest {
    /// 从 JSON 文本解析并校验清单。
    pub fn parse(text: &str) -> Result<ValidatedManifest, ManifestError> {
        let manifest: PluginManifest =
            serde_json::from_str(text).map_err(|error| ManifestError::Parse(error.to_string()))?;
        manifest.validate()?;
        Ok(ValidatedManifest(manifest))
    }

    /// 执行全部校验规则。
    pub fn validate(&self) -> Result<(), ManifestError> {
        self.validate_required_fields()?;
        validate_id(&self.id)?;
        validate_semver(&self.version)?;
        self.validate_api_version()?;
        validate_main(&self.main)?;
        if let Some(homepage) = &self.homepage {
            validate_homepage(homepage)?;
        }
        self.validate_contributes()
    }

    fn validate_required_fields(&self) -> Result<(), ManifestError> {
        // (字段名, 值, 长度上限)；上限防止荒谬清单拖垮 UI 展示与日志
        let required: [(&str, &str, usize); 7] = [
            ("id", &self.id, 128),
            ("name", &self.name, 100),
            ("author", &self.author, 100),
            ("license", &self.license, 100),
            ("description", &self.description, 1_000),
            ("main", &self.main, 255),
            ("apiVersion", &self.api_version, 16),
        ];
        for (field, value, max) in required {
            if value.trim().is_empty() {
                return Err(ManifestError::EmptyField(field));
            }
            if value.chars().count() > max {
                return Err(ManifestError::FieldTooLong { field, max });
            }
        }
        validate_semver(&self.version)
            .map_err(|_| ManifestError::InvalidVersion(self.version.clone()))?;
        Ok(())
    }

    fn validate_api_version(&self) -> Result<(), ManifestError> {
        let parts: Vec<&str> = self.api_version.split('.').collect();
        if parts.len() != 2 {
            return Err(ManifestError::InvalidApiVersion(self.api_version.clone()));
        }
        let (major, minor) = match (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
            (Ok(major), Ok(minor)) => (major, minor),
            _ => return Err(ManifestError::InvalidApiVersion(self.api_version.clone())),
        };
        let supported = format!("{SUPPORTED_API_MAJOR}.{SUPPORTED_API_MINOR}");
        if major != SUPPORTED_API_MAJOR {
            return Err(ManifestError::ApiMajorMismatch {
                found: self.api_version.clone(),
                supported,
            });
        }
        if minor > SUPPORTED_API_MINOR {
            return Err(ManifestError::ApiMinorNewer {
                found: self.api_version.clone(),
                supported,
            });
        }
        Ok(())
    }

    fn validate_contributes(&self) -> Result<(), ManifestError> {
        let mut seen = std::collections::BTreeSet::new();
        for command in &self.contributes.commands {
            validate_contribution_id(&command.id)?;
            if command.title.trim().is_empty() {
                return Err(ManifestError::EmptyField("contributes.commands[].title"));
            }
            if command.title.chars().count() > 200 {
                return Err(ManifestError::FieldTooLong {
                    field: "contributes.commands[].title",
                    max: 200,
                });
            }
            if let Some(keybinding) = &command.keybinding {
                if keybinding.trim().is_empty() {
                    return Err(ManifestError::EmptyField(
                        "contributes.commands[].keybinding",
                    ));
                }
            }
            if !seen.insert(command.id.clone()) {
                return Err(ManifestError::DuplicateContributionId(command.id.clone()));
            }
        }
        for panel in &self.contributes.panels {
            validate_contribution_id(&panel.id)?;
            if panel.title.trim().is_empty() {
                return Err(ManifestError::EmptyField("contributes.panels[].title"));
            }
            if !seen.insert(panel.id.clone()) {
                return Err(ManifestError::DuplicateContributionId(panel.id.clone()));
            }
        }
        Ok(())
    }
}

/// 反向域名风格 id：≥ 2 个小写标签，标签内 `[a-z0-9-]`，字母开头，不以连字符开头/结尾。
fn validate_id(id: &str) -> Result<(), ManifestError> {
    let labels: Vec<&str> = id.split('.').collect();
    if labels.len() < 2 {
        return Err(ManifestError::InvalidId(id.to_owned()));
    }
    for label in labels {
        let chars: Vec<char> = label.chars().collect();
        if chars.is_empty()
            || chars.len() > 63
            || !chars[0].is_ascii_lowercase()
            || chars[chars.len() - 1] == '-'
            || !chars
                .iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        {
            return Err(ManifestError::InvalidId(id.to_owned()));
        }
    }
    Ok(())
}

/// 贡献点短 id：kebab 风格，字母开头，`[a-z0-9-]`。
fn validate_contribution_id(id: &str) -> Result<(), ManifestError> {
    let chars: Vec<char> = id.chars().collect();
    let valid = !chars.is_empty()
        && chars.len() <= 64
        && chars[0].is_ascii_lowercase()
        && chars
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-');
    if valid {
        Ok(())
    } else {
        Err(ManifestError::InvalidContributionId(id.to_owned()))
    }
}

/// 宽松 SemVer：`X.Y.Z`（禁止前导零）+ 可选 `-prerelease`。
/// 不引入 `semver` crate：清单里只需要"形状正确"这一层判断，构建元数据等
/// 完整语义对插件清单没有用途。
fn validate_semver(version: &str) -> Result<(), ManifestError> {
    let invalid = || ManifestError::InvalidVersion(version.to_owned());
    let (core, _) = version
        .split_once('-')
        .map_or((version, None), |(core, pre)| {
            if pre.is_empty() {
                return (version, None);
            }
            (core, Some(pre))
        });
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return Err(invalid());
    }
    for part in parts {
        let numeric = !part.is_empty()
            && part.len() <= 10
            && part.chars().all(|c| c.is_ascii_digit())
            && !(part.len() > 1 && part.starts_with('0'));
        if !numeric {
            return Err(invalid());
        }
    }
    if let Some(pre) = version.split_once('-').map(|(_, pre)| pre) {
        let pre_ok = !pre.is_empty()
            && pre.len() <= 32
            && pre
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
        if !pre_ok {
            return Err(invalid());
        }
    }
    Ok(())
}

/// wasm 入口：必须是插件目录内的普通文件名，拒绝一切路径穿越形态。
fn validate_main(main: &str) -> Result<(), ManifestError> {
    let valid = main.len() > 6
        && main.ends_with(".wasm")
        && !main.contains('/')
        && !main.contains('\\')
        && !main.contains("..");
    if valid {
        Ok(())
    } else {
        Err(ManifestError::InvalidMain(main.to_owned()))
    }
}

fn validate_homepage(homepage: &str) -> Result<(), ManifestError> {
    if homepage.starts_with("https://") && homepage.len() <= 2_000 {
        Ok(())
    } else {
        Err(ManifestError::InvalidHomepage(homepage.to_owned()))
    }
}

/// 生成清单的 JSON Schema（供 docs 与第三方校验工具使用，T6.1）。
pub fn manifest_json_schema() -> Value {
    // schema_for! 的产物内部就是 JSON Value，转换不会失败（schemars 提供 Into<Value>）
    schemars::schema_for!(PluginManifest).into()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::permission::Permission;

    /// 一份完全合法的清单，各用例在其基础上做变异。
    fn valid_manifest_json() -> String {
        r#"{
            "id": "com.example.commit-template",
            "name": "Commit Template",
            "version": "1.2.3",
            "apiVersion": "0.1",
            "author": "ForgeDesk contributors",
            "license": "MIT",
            "description": "Fill commit messages from templates.",
            "homepage": "https://github.com/example/commit-template",
            "main": "plugin.wasm",
            "permissions": ["git:read", "ui:command", "ui:toast"],
            "contributes": {
                "commands": [{ "id": "fill-template", "title": "Fill from template" }],
                "panels": [{ "id": "stats", "title": "Stats", "location": "sidebar" }]
            }
        }"#
        .to_owned()
    }

    fn mutate_json<F: FnOnce(&mut Value)>(mutate: F) -> String {
        let mut value: Value = serde_json::from_str(&valid_manifest_json()).unwrap();
        mutate(&mut value);
        value.to_string()
    }

    #[test]
    fn a_well_formed_manifest_parses_with_typed_permissions_and_contributions() {
        let validated = PluginManifest::parse(&valid_manifest_json()).unwrap();

        assert_eq!(validated.id, "com.example.commit-template");
        assert_eq!(
            validated.permissions,
            vec![
                Permission::GitRead,
                Permission::UiCommand,
                Permission::UiToast
            ]
        );
        assert_eq!(validated.contributes.commands.len(), 1);
        assert_eq!(
            validated.contributes.panels[0].location,
            PanelLocation::Sidebar
        );
    }

    #[test]
    fn an_id_with_a_single_dns_label_is_rejected() {
        let json = mutate_json(|v| {
            v["id"] = "commit-template".into();
        });
        assert!(matches!(
            PluginManifest::parse(&json),
            Err(ManifestError::InvalidId(_))
        ));
    }

    #[test]
    fn an_id_with_uppercase_or_leading_hyphen_labels_is_rejected() {
        for bad in [
            "Com.example.plugin",
            "com.-example.plugin",
            "com.example_.plugin",
        ] {
            let json = mutate_json(|v| {
                v["id"] = bad.into();
            });
            assert!(
                matches!(
                    PluginManifest::parse(&json),
                    Err(ManifestError::InvalidId(_))
                ),
                "id `{bad}` 应被拒绝"
            );
        }
    }

    #[test]
    fn a_manifest_requesting_a_permission_outside_the_whitelist_is_rejected() {
        // 越权用例：清单要求白名单外权限，加载必须失败并指明是哪一项
        let json = mutate_json(|v| {
            v["permissions"] = serde_json::json!(["git:read", "fs:admin"]);
        });
        let error = PluginManifest::parse(&json).unwrap_err();
        assert!(error.to_string().contains("fs:admin"), "{error}");
    }

    #[test]
    fn a_manifest_with_a_mismatched_api_major_is_rejected() {
        let json = mutate_json(|v| {
            v["apiVersion"] = "1.0".into();
        });
        assert!(matches!(
            PluginManifest::parse(&json),
            Err(ManifestError::ApiMajorMismatch { .. })
        ));
    }

    #[test]
    fn a_manifest_with_a_newer_api_minor_is_rejected_but_an_older_minor_is_accepted() {
        let newer = mutate_json(|v| {
            v["apiVersion"] = "0.2".into();
        });
        assert!(matches!(
            PluginManifest::parse(&newer),
            Err(ManifestError::ApiMinorNewer { .. })
        ));

        let older = mutate_json(|v| {
            v["apiVersion"] = "0.0".into();
        });
        assert!(PluginManifest::parse(&older).is_ok());
    }

    #[test]
    fn a_main_field_pointing_outside_the_plugin_directory_is_rejected() {
        // 穿越用例：任何分隔符 / .. 都不允许，加载器不会走出插件目录
        for bad in [
            "../evil.wasm",
            "sub/dir/plugin.wasm",
            "C:\\evil.wasm",
            ".wasm",
        ] {
            let json = mutate_json(|v| {
                v["main"] = bad.into();
            });
            assert!(
                matches!(
                    PluginManifest::parse(&json),
                    Err(ManifestError::InvalidMain(_))
                ),
                "main `{bad}` 应被拒绝"
            );
        }
    }

    #[test]
    fn a_non_semver_version_is_rejected_including_leading_zeros() {
        for bad in ["1.2", "01.2.3", "1.2.3.4", "not-a-version", "1.2.3-"] {
            let json = mutate_json(|v| {
                v["version"] = bad.into();
            });
            assert!(
                matches!(
                    PluginManifest::parse(&json),
                    Err(ManifestError::InvalidVersion(_))
                ),
                "version `{bad}` 应被拒绝"
            );
        }
        let pre_release = mutate_json(|v| {
            v["version"] = "1.2.3-beta.1".into();
        });
        assert!(PluginManifest::parse(&pre_release).is_ok());
    }

    #[test]
    fn an_empty_or_overlong_required_field_is_rejected() {
        let empty = mutate_json(|v| {
            v["description"] = "  ".into();
        });
        assert!(matches!(
            PluginManifest::parse(&empty),
            Err(ManifestError::EmptyField("description"))
        ));

        let overlong = "x".repeat(129);
        let json = mutate_json(|v| {
            v["id"] = serde_json::Value::String(overlong);
        });
        // 129 个 x 也是非法 id（无点分标签），但长度上限先触发；用 name 验证标签校验的边界
        let long_name = mutate_json(|v| {
            v["name"] = serde_json::Value::String("x".repeat(101));
        });
        assert!(matches!(
            PluginManifest::parse(&json),
            Err(ManifestError::FieldTooLong {
                field: "id",
                max: 128
            })
        ));
        assert!(matches!(
            PluginManifest::parse(&long_name),
            Err(ManifestError::FieldTooLong {
                field: "name",
                max: 100
            })
        ));
    }

    #[test]
    fn a_homepage_without_https_is_rejected() {
        let json = mutate_json(|v| {
            v["homepage"] = "http://example.com/plugin".into();
        });
        assert!(matches!(
            PluginManifest::parse(&json),
            Err(ManifestError::InvalidHomepage(_))
        ));
    }

    #[test]
    fn duplicate_or_malformed_contribution_ids_are_rejected() {
        let duplicate = mutate_json(|v| {
            v["contributes"]["commands"] = serde_json::json!([
                { "id": "fill-template", "title": "A" },
                { "id": "fill-template", "title": "B" }
            ]);
        });
        assert!(matches!(
            PluginManifest::parse(&duplicate),
            Err(ManifestError::DuplicateContributionId(_))
        ));

        let malformed = mutate_json(|v| {
            v["contributes"]["commands"] = serde_json::json!([
                { "id": "Fill Template", "title": "A" }
            ]);
        });
        assert!(matches!(
            PluginManifest::parse(&malformed),
            Err(ManifestError::InvalidContributionId(_))
        ));
    }

    #[test]
    fn unknown_fields_are_rejected_so_the_schema_cannot_silently_drift() {
        let json = mutate_json(|v| {
            v["telemetryEndpoint"] = "https://evil.example.com".into();
        });
        assert!(matches!(
            PluginManifest::parse(&json),
            Err(ManifestError::Parse(_))
        ));
    }

    #[test]
    fn the_generated_json_schema_covers_required_fields_and_the_permission_whitelist() {
        let schema = manifest_json_schema();

        let required = schema["required"].as_array().unwrap();
        // permissions/contributes 带 default，不进 required（"无权限"是合法清单）
        for field in ["id", "name", "version", "apiVersion", "main"] {
            assert!(
                required.iter().any(|r| r == field),
                "schema required 应包含 {field}"
            );
        }
        assert!(
            !required.iter().any(|r| r == "permissions"),
            "permissions 是可选字段"
        );
        // schemars 1.x 把类型定义收进 $defs；单元枚举序列化为 oneOf + const
        let has_whitelist_variant = schema["$defs"]["Permission"]["oneOf"]
            .as_array()
            .is_some_and(|variants| variants.iter().any(|variant| variant["const"] == "fs:read"));
        assert!(has_whitelist_variant, "schema 应枚举白名单权限");
        // deny_unknown_fields 必须落到 schema 上，否则第三方校验会放过未知字段
        assert_eq!(
            schema["additionalProperties"],
            serde_json::json!(false),
            "未知字段必须被 schema 拒绝"
        );
    }
}
