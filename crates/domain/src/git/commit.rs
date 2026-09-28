//! 提交模型（`git log --format=...` 的语义）。

use serde::{Deserialize, Serialize};

/// 作者 / 提交者身份。
///
/// 嵌套而不是把 `author_name`/`author_email`/`author_time` 平铺在 [`Commit`] 上：
/// PLAN §5.4 的数据模型就是 `author{name,email,time}`，而且"给谁署名"在
/// [`super::spec::CommitSpec`] 里也要用同一组字段（amend 保留原作者）。
/// 平铺会让同一组语义出现两种形状。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signature {
    /// 姓名。
    pub name: String,
    /// 邮箱。
    pub email: String,
    /// 时间（Unix 秒）。无法解析或未指定时为 `None`。
    pub time: Option<i64>,
}

impl Signature {
    /// 用姓名与邮箱创建（时间未知）。
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            email: email.into(),
            time: None,
        }
    }

    /// 附加时间。
    #[must_use]
    pub fn with_time(mut self, time: i64) -> Self {
        self.time = Some(time);
        self
    }

    /// `Name <email>` 形式，用于生成等价命令与日志。
    pub fn display(&self) -> String {
        format!("{} <{}>", self.name, self.email)
    }
}

/// `git log --format=%G?` 给出的签名校验状态。
///
/// T1.1 只做**占位解析**（把字符映射成枚举）；真正的校验与展示（可信度、
/// 密钥来源、撤销状态）属于 M2 的提交详情面板。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SignatureStatus {
    /// `G`：签名有效且可信。
    Good,
    /// `B`：签名无效（内容被改过或签名本身是坏的）。
    Bad,
    /// `U`：签名有效但密钥不可信。
    UntrustedGood,
    /// `X`：签名有效但已过期。
    Expired,
    /// `Y`：签名有效但密钥已过期。
    ExpiredKey,
    /// `R`：签名有效但密钥已被撤销。
    RevokedKey,
    /// `E`：无法校验（缺少公钥）。
    MissingKey,
    /// `N`：没有签名。
    Unsigned,
    /// `?` 或无法识别的字符：Git 无法判断。
    Unknown,
}

impl SignatureStatus {
    /// 从 `%G?` 的字符解析。
    pub const fn from_byte(byte: u8) -> Self {
        match byte {
            b'G' => Self::Good,
            b'B' => Self::Bad,
            b'U' => Self::UntrustedGood,
            b'X' => Self::Expired,
            b'Y' => Self::ExpiredKey,
            b'R' => Self::RevokedKey,
            b'E' => Self::MissingKey,
            b'N' => Self::Unsigned,
            _ => Self::Unknown,
        }
    }

    /// 该状态是否代表"存在签名且校验通过"。
    pub const fn is_valid(self) -> bool {
        matches!(
            self,
            Self::Good | Self::UntrustedGood | Self::Expired | Self::ExpiredKey | Self::RevokedKey
        )
    }
}

/// 一条提交记录。
///
/// 元数据字段（作者名、邮箱、subject）是 **lossy** 的：Git 允许提交信息使用任意编码，
/// 界面无法渲染任意编码，而这些字段也不参与文件系统操作（与路径相反，见
/// [`super::path`] 的说明）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    /// 提交 oid（十六进制字符串）。
    pub oid: String,
    /// 父提交 oid，顺序与 Git 一致（第一个是 first-parent）。根提交为空。
    pub parents: Vec<String>,
    /// 作者。
    pub author: Signature,
    /// 提交者。
    pub committer: Signature,
    /// 指向该提交的引用（`%D` 的输出，如 `HEAD -> main`、`tag: v1.0.0`）。
    pub refs: Vec<String>,
    /// 签名校验状态。
    pub signature: SignatureStatus,
    /// 提交信息的第一行（subject）。
    pub subject: String,
    /// 提交信息正文（subject 之后的内容）。
    ///
    /// T1.1 的 `LOG_FORMAT` 不含 `%b`（列表页不需要正文，带上它会让大仓库的
    /// 日志输出成倍增长）；只有 `show()` 单条查询会填这个字段，其余为 `None`。
    pub body: Option<String>,
}

impl Commit {
    /// 是否为根提交（没有父提交）。
    pub fn is_root(&self) -> bool {
        self.parents.is_empty()
    }

    /// 是否为合并提交。
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Signature, SignatureStatus};

    #[test]
    fn signature_characters_map_to_their_status() {
        assert_eq!(SignatureStatus::from_byte(b'G'), SignatureStatus::Good);
        assert_eq!(SignatureStatus::from_byte(b'B'), SignatureStatus::Bad);
        assert_eq!(SignatureStatus::from_byte(b'N'), SignatureStatus::Unsigned);
        assert_eq!(SignatureStatus::from_byte(b'?'), SignatureStatus::Unknown);
        assert_eq!(SignatureStatus::from_byte(b'z'), SignatureStatus::Unknown);
    }

    #[test]
    fn only_present_and_verifiable_signatures_are_valid() {
        assert!(SignatureStatus::Good.is_valid());
        assert!(SignatureStatus::ExpiredKey.is_valid());
        assert!(!SignatureStatus::Unsigned.is_valid());
        assert!(!SignatureStatus::Bad.is_valid());
        assert!(!SignatureStatus::MissingKey.is_valid());
    }

    #[test]
    fn signature_display_is_the_git_identity_form() {
        let signature = Signature::new("Ada", "ada@example.com").with_time(1_704_164_645);

        assert_eq!(signature.display(), "Ada <ada@example.com>");
        assert_eq!(signature.time, Some(1_704_164_645));
    }
}
