//! SSH 密钥盘点：本地有哪些密钥、agent 里加载了什么。
//!
//! # 红线：私钥内容一个字节都不读
//!
//! 本模块对私钥只做两件事：**看它存不存在**（`metadata`）与**看它的文件名**。
//! 不打开、不解析、不复制、不哈希。公钥（`*.pub`）是公开信息，可以读它的
//! 首行来拿到密钥类型与注释。
//!
//! # 为什么需要它
//!
//! `Permission denied (publickey)` 之后用户最常问的是"到底用没用我配的那把 key"。
//! 界面上能回答这个问题的前提是：列出候选密钥、指出私钥/公钥是否配对、
//! 以及 agent 里当前加载的是哪几把（agent 优先于文件，这是最常见的困惑来源）。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CredentialsError;

/// 一把本地密钥的**元信息**（不含私钥内容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKey {
    /// 公钥文件路径（存在时）。
    pub public_path: Option<String>,
    /// 私钥文件路径（存在时；只判断存在性，不读内容）。
    pub private_path: Option<String>,
    /// 密钥类型（来自公钥首行，如 `ssh-ed25519`）。
    pub key_type: Option<String>,
    /// 公钥里的注释（通常是 `user@host`）。
    pub comment: Option<String>,
}

impl SshKey {
    /// 是否公私钥配对齐全（配对齐全的 key 才可能被自动使用）。
    pub fn is_pair(&self) -> bool {
        self.public_path.is_some() && self.private_path.is_some()
    }

    /// 只有私钥文件、没有对应公钥：值得提示用户（`ssh-keygen` 通常成对生成）。
    pub fn is_private_only(&self) -> bool {
        self.private_path.is_some() && self.public_path.is_none()
    }

    /// 展示名（取文件名，不含目录：路径已脱敏，见 `hint` 只放数据的约定）。
    pub fn display_name(&self) -> String {
        let path = self
            .public_path
            .as_deref()
            .or(self.private_path.as_deref())
            .unwrap_or_default();
        Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// agent 里的一把密钥。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentKey {
    /// 位长（`ssh-add -l` 首列）。
    pub bits: Option<u32>,
    /// 指纹（`SHA256:...`）——用户拿它去跟服务端设置里的指纹核对。
    pub fingerprint: String,
    /// 注释（`ssh-add -l` 行尾括号前的部分）。
    pub comment: Option<String>,
}

/// `ssh-add -l` 的三种正常结局，加上"跑不起来"。
///
/// 把"没有身份"与"agent 没运行"分开：前者是正常的（用户没加载 key），
/// 后者要提示用户启动 agent。混在一起会让用户以为自己的密钥丢了。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "keys")]
pub enum AgentStatus {
    /// 已加载若干密钥。
    Ready(Vec<AgentKey>),
    /// agent 在跑，但没有加载任何密钥（`ssh-add -l` 退出码 1）。
    NoIdentities,
    /// agent 没有运行（`ssh-add -l` 退出码 2）。
    NotRunning,
    /// 无法判定（命令缺失、输出无法解析）。
    Unknown {
        /// 原始原因（命令名/退出码，不含私钥内容）。
        reason: String,
    },
}

/// 本地 SSH 盘点结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshInventory {
    /// 扫描的目录（`~/.ssh`；不存在时为 `None`）。
    pub directory: Option<String>,
    /// 目录里的密钥（按文件名排序，稳定输出便于界面比对）。
    pub keys: Vec<SshKey>,
    /// agent 状态。
    pub agent: AgentStatus,
}

impl SshInventory {
    /// 是否有任何可用线索（密钥文件或 agent 密钥）。
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
            && matches!(
                self.agent,
                AgentStatus::NoIdentities | AgentStatus::NotRunning
            )
    }
}

/// 默认的 SSH 目录（`$HOME/.ssh`）。
///
/// 用 `HOME`/`USERPROFILE` 而不是引入 `dirs` crate：只差一行代码，
/// 而多一个依赖要多审一次许可证与供应链（AGENTS §8）。
pub fn default_ssh_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".ssh"))
}

/// 一个文件名是否**可能**是私钥。
///
/// 为什么用启发式：私钥与普通文件的区别在内容里（`-----BEGIN ... PRIVATE KEY-----`），
/// 而我们承诺不读私钥内容。因此这里只按命名惯例判断，并把结果当作"候选"呈现给用户
/// ——界面上写"可能未配对"，而不是断言它是私钥。
fn is_private_candidate(name: &str) -> bool {
    if name.ends_with(".pub") {
        return false;
    }
    name.starts_with("id_") || name.ends_with(".pem") || name.ends_with(".key")
}

/// 解析公钥首行（`ssh-ed25519 AAAAC3Nza... user@host`）。
///
/// 只认已知的密钥类型前缀：`known_hosts` 之类的文件首行也是空格分隔的，
/// 但首段不是 `ssh-*`/`ecdsa-*`，因此不会被误认成公钥。
pub fn parse_public_key(line: &str) -> Option<(String, Option<String>)> {
    let mut fields = line.split_whitespace();
    let key_type = fields.next()?;
    let known = key_type.starts_with("ssh-")
        || key_type.starts_with("ecdsa-")
        || key_type.starts_with("sk-");
    if !known {
        return None;
    }
    let _body = fields.next()?;
    let comment = {
        let rest: Vec<&str> = fields.collect();
        if rest.is_empty() {
            None
        } else {
            Some(rest.join(" "))
        }
    };
    Some((key_type.to_owned(), comment))
}

/// 扫描一个目录里的密钥（只 stat 私钥，只读公钥首行）。
///
/// 目录不存在时返回空列表而不是错误：新机器上没配过 SSH 是**正常状态**，
/// 不是故障——界面应当显示"还没有生成 SSH 密钥"，而不是弹一个错误。
pub fn scan_keys(dir: &Path) -> Result<Vec<SshKey>, CredentialsError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<String> = fs::read_dir(dir)
        .map_err(|error| CredentialsError::Io(format!("read {}: {error}", dir.display())))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();

    let mut keys = Vec::new();
    // 先处理公钥（`.pub`），它们能给出密钥类型与注释
    for name in entries.iter().filter(|name| name.ends_with(".pub")) {
        let stem = name.trim_end_matches(".pub");
        let public_path = dir.join(name);
        let private_path = dir.join(stem);
        let (key_type, comment) = fs::read_to_string(&public_path)
            .ok()
            .and_then(|content| content.lines().next().and_then(parse_public_key))
            .map(|(key_type, comment)| (Some(key_type), comment))
            .unwrap_or((None, None));

        keys.push(SshKey {
            public_path: Some(public_path.to_string_lossy().into_owned()),
            // 只判断存在性：`is_file()` 不会读取内容
            private_path: private_path
                .is_file()
                .then(|| private_path.to_string_lossy().into_owned()),
            key_type,
            comment,
        });
    }

    // 再补上"只有私钥文件、没有公钥"的候选
    for name in entries.iter().filter(|name| is_private_candidate(name)) {
        let private_path = dir.join(name);
        keys.push(SshKey {
            public_path: None,
            private_path: private_path
                .is_file()
                .then(|| private_path.to_string_lossy().into_owned()),
            key_type: None,
            comment: None,
        });
    }

    Ok(keys)
}

/// 解析 `ssh-add -l` 的输出。
///
/// 退出码的语义（OpenSSH 手册）：0 = 列出密钥，1 = "The agent has no identities"，
/// 2 = "Could not open a connection to your authentication agent"。
pub fn parse_agent_listing(stdout: &str, exit_code: i32) -> AgentStatus {
    match exit_code {
        0 => {
            let keys: Vec<AgentKey> = stdout.lines().filter_map(parse_agent_line).collect();
            if keys.is_empty() {
                AgentStatus::Unknown {
                    reason: "ssh-add -l reported success but printed no keys".to_owned(),
                }
            } else {
                AgentStatus::Ready(keys)
            }
        }
        1 => AgentStatus::NoIdentities,
        2 => AgentStatus::NotRunning,
        other => AgentStatus::Unknown {
            reason: format!("ssh-add -l exited with {other}"),
        },
    }
}

/// 解析一行 `2048 SHA256:abc... user@host (RSA)`。
fn parse_agent_line(line: &str) -> Option<AgentKey> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut fields = trimmed.split_whitespace();
    let first = fields.next()?;
    // 首列可能是位长（新版本）或直接是指纹（个别实现）；两者都兼容。
    // 指纹列**必须**带 `SHA256:`/`MD5:` 前缀：否则一句普通的横幅
    // （`some unexpected banner`）会被当成"一把密钥，指纹叫 unexpected"，
    // 界面上就会出现一条根本不存在的 key。
    let (bits, fingerprint) = if first.starts_with("SHA256:") || first.starts_with("MD5:") {
        (None, first.to_owned())
    } else {
        let bits = first.parse::<u32>().ok()?;
        let candidate = fields.next()?;
        if !(candidate.starts_with("SHA256:") || candidate.starts_with("MD5:")) {
            return None;
        }
        (Some(bits), candidate.to_owned())
    };
    // 注释里常带平台的 `(RSA)` 后缀，去掉它让展示干净
    let rest: Vec<&str> = fields.collect();
    let joined = rest.join(" ");
    let comment = {
        let without_algorithm = joined
            .rsplit_once(" (")
            .map(|(comment, _)| comment.to_owned())
            .unwrap_or(joined);
        let trimmed_comment = without_algorithm.trim();
        (!trimmed_comment.is_empty()).then(|| trimmed_comment.to_owned())
    };

    Some(AgentKey {
        bits,
        fingerprint,
        comment,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// 临时目录（用例自己造密钥文件；**不碰**用户真实的 ~/.ssh）
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("forgedesk-ssh-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    const PUBLIC_LINE: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleKeyBody octocat@example.com";

    #[test]
    fn a_public_key_line_yields_its_type_and_comment() {
        let (key_type, comment) = parse_public_key(PUBLIC_LINE).expect("parsable");

        assert_eq!(key_type, "ssh-ed25519");
        assert_eq!(comment.as_deref(), Some("octocat@example.com"));
    }

    #[test]
    fn a_known_hosts_looking_line_is_not_mistaken_for_a_public_key() {
        let line = "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample";
        // 首段是主机名而不是密钥类型 → 不认
        assert!(parse_public_key(line).is_none());
        assert!(parse_public_key("").is_none());
        assert!(parse_public_key("ssh-ed25519").is_none());
    }

    #[test]
    fn scanning_finds_pairs_private_only_candidates_and_ignores_config_files() {
        let dir = temp_dir("scan");
        fs::write(
            dir.join("id_ed25519"),
            b"PRIVATE KEY NEVER READ BY THE SCANNER",
        )
        .expect("write");
        fs::write(dir.join("id_ed25519.pub"), PUBLIC_LINE).expect("write");
        fs::write(dir.join("id_rsa"), b"another private").expect("write");
        fs::write(dir.join("known_hosts"), b"github.com ssh-ed25519 AAAA").expect("write");
        fs::write(dir.join("config"), b"Host *\n  AddKeysToAgent yes\n").expect("write");

        let keys = scan_keys(&dir).expect("scan");

        // id_ed25519.pub + 两个私钥候选（id_ed25519 的私钥、单独的 id_rsa）
        assert_eq!(keys.len(), 3, "{keys:?}");
        let pair = keys.iter().find(|key| key.is_pair()).expect("pair");
        assert_eq!(pair.display_name(), "id_ed25519.pub");
        assert_eq!(pair.key_type.as_deref(), Some("ssh-ed25519"));
        assert_eq!(pair.comment.as_deref(), Some("octocat@example.com"));
        assert!(keys.iter().any(|key| key.is_private_only()));
        // known_hosts / config 不是密钥
        assert!(!keys.iter().any(|key| key.display_name() == "known_hosts"));
        assert!(!keys.iter().any(|key| key.display_name() == "config"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_ssh_directory_is_an_empty_inventory_not_an_error() {
        let dir = temp_dir("missing").join("does-not-exist");

        // 新机器上没配过 SSH 是正常状态，不该弹错
        assert_eq!(scan_keys(&dir).expect("scan"), Vec::new());
    }

    #[test]
    fn the_listing_of_loaded_agent_keys_is_parsed_with_bits_fingerprint_and_comment() {
        let stdout = "2048 SHA256:AbCdEf1234567890 octocat@example.com (RSA)\n\
                      256 SHA256:ZzYyXx0987654321 id_ed25519 (ED25519)\n";

        match parse_agent_listing(stdout, 0) {
            AgentStatus::Ready(keys) => {
                assert_eq!(keys.len(), 2);
                assert_eq!(keys[0].bits, Some(2048));
                assert_eq!(keys[0].fingerprint, "SHA256:AbCdEf1234567890");
                // 平台后缀 `(RSA)` 不该混进注释里
                assert_eq!(keys[0].comment.as_deref(), Some("octocat@example.com"));
                assert_eq!(keys[1].bits, Some(256));
                assert_eq!(keys[1].comment.as_deref(), Some("id_ed25519"));
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn an_agent_without_identities_is_distinguished_from_a_stopped_agent() {
        // 这两种情况的界面文案完全不同：一个引导 ssh-add，一个引导启动 agent
        assert_eq!(parse_agent_listing("", 1), AgentStatus::NoIdentities);
        assert_eq!(parse_agent_listing("", 2), AgentStatus::NotRunning);
    }

    #[test]
    fn an_unexpected_exit_code_is_reported_as_unknown_with_the_code() {
        match parse_agent_listing("ssh: command not found", 127) {
            AgentStatus::Unknown { reason } => assert!(reason.contains("127"), "{reason}"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn a_successful_listing_without_parseable_lines_is_unknown_rather_than_empty_ready() {
        // 报告"已加载 0 把 key"会让用户以为自己清空了 agent
        match parse_agent_listing("some unexpected banner\n", 0) {
            AgentStatus::Unknown { .. } => {}
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn a_fingerprint_only_line_is_accepted_even_without_a_bit_count() {
        let key = parse_agent_line("SHA256:OnlyFingerprintHere key-name").expect("parsable");

        assert_eq!(key.bits, None);
        assert_eq!(key.fingerprint, "SHA256:OnlyFingerprintHere");
        assert_eq!(key.comment.as_deref(), Some("key-name"));
    }

    #[test]
    fn the_default_directory_is_derived_from_home_without_touching_it() {
        let dir = default_ssh_dir();

        if let Some(dir) = dir {
            assert!(dir.ends_with(".ssh"));
        }
    }

    #[test]
    fn an_inventory_reports_itself_empty_only_when_there_is_nothing_to_show() {
        let empty = SshInventory {
            directory: None,
            keys: Vec::new(),
            agent: AgentStatus::NotRunning,
        };
        assert!(empty.is_empty());

        let with_agent = SshInventory {
            directory: None,
            keys: Vec::new(),
            agent: AgentStatus::Ready(Vec::new()),
        };
        assert!(!with_agent.is_empty());
    }
}
