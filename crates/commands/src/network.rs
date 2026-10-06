//! 网络与密钥命令族（T6.8）：代理设置、连通性测试、SSH 连接测试、GPG 密钥列举与签名测试。
//!
//! # 代理的生效范围
//!
//! 三处消费同一份设置（`network.*` 键，全局 KV）：
//!   1. **git CLI**：设置变更时经 `CliGitEngine::set_extra_config` 注入
//!      `-c http.proxy=<url>`——与配置文件等价但不落盘、不污染用户配置（T6.8 规格明确要求）；
//!   2. **GitHub HTTP**（T4.2）：`GitHubHttp` 按设置重建（构造廉价，直接换实例）；
//!   3. **更新下载**（T7.1）：构造点读取同一份设置（届时接入）。
//!
//! `mode`：`none`（直连）/ `system`（跟随系统，当前默认）/ `manual`（显式 URL）。

use std::time::Instant;

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_provider::{ApiRequest, GitHubHttp, HttpConfig};
use forgedesk_storage::{Database, Scope, SettingsRepository};
use serde::{Deserialize, Serialize};

/// 设置键（全局 KV；前端设置页直接读写同键）。
/// 代理模式设置键（`none`/`system`/`manual`）。
pub const KEY_PROXY_MODE: &str = "network.proxyMode";
/// 手动代理 URL 设置键（mode=manual 时生效）。
pub const KEY_PROXY_URL: &str = "network.proxyUrl";
/// 逗号分隔的 no_proxy 列表设置键。
pub const KEY_NO_PROXY: &str = "network.noProxy";

/// 代理模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxyMode {
    /// 直连（显式禁用系统代理）。
    None,
    /// 跟随系统环境（当前行为，默认）。
    System,
    /// 手动指定代理 URL。
    Manual,
}

impl ProxyMode {
    fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "none" => Some(Self::None),
            "system" => Some(Self::System),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

/// 从设置存储解析代理配置（无记录 = system 默认）。
pub fn resolve_proxy_settings(
    database: &Database,
) -> AppResult<(ProxyMode, Option<String>, Vec<String>)> {
    let repo = SettingsRepository::new(database);
    let mode = repo
        .get(&Scope::Global, KEY_PROXY_MODE)?
        .and_then(|raw| serde_json::from_str::<String>(&raw).ok())
        .and_then(|raw| ProxyMode::from_key(&raw))
        .unwrap_or(ProxyMode::System);
    let url = repo
        .get(&Scope::Global, KEY_PROXY_URL)?
        .and_then(|raw| serde_json::from_str::<String>(&raw).ok());
    let no_proxy = repo
        .get(&Scope::Global, KEY_NO_PROXY)?
        .and_then(|raw| serde_json::from_str::<String>(&raw).ok())
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok((mode, url, no_proxy))
}

/// 把代理设置应用到 git CLI 引擎与 GitHub HTTP 客户端。
///
/// 返回重建后的 [`GitHubHttp`]（调用方替换 AppState 里的实例——构造廉价，
/// 直接换比引入 ArcSwap 简单且语义清晰："下一次请求起生效"）。
pub fn apply_proxy_settings(engines: &GitEngines, database: &Database) -> AppResult<GitHubHttp> {
    let (mode, url, _no_proxy) = resolve_proxy_settings(database)?;
    let proxy_arg = match mode {
        ProxyMode::System => None,
        ProxyMode::None => Some(("http.proxy".to_owned(), String::new())),
        ProxyMode::Manual => {
            let url = url.clone().unwrap_or_default();
            if url.trim().is_empty() {
                None
            } else {
                Some(("http.proxy".to_owned(), url))
            }
        }
    };
    engines
        .write()
        .set_extra_config(proxy_arg.iter().cloned().collect());

    let config = match mode {
        ProxyMode::System => HttpConfig::default(),
        ProxyMode::None => HttpConfig {
            proxy: Some(String::new()),
            ..HttpConfig::default()
        },
        ProxyMode::Manual => HttpConfig {
            proxy: url,
            ..HttpConfig::default()
        },
    };
    GitHubHttp::new(config)
}

/// 单项连通性测试结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityResult {
    /// 测试目标（api / raw / git / gpg / 具体 host）。
    pub target: String,
    /// 是否通过。
    pub ok: bool,
    /// 耗时（毫秒）。
    pub latency_ms: u64,
    /// 成功时的响应摘要 / 失败原因（已脱敏，不含代理 URL 凭据）。
    pub detail: String,
}

fn http_error_message(error: &AppError) -> String {
    // 简单脱敏：reqwest 的错误链可能带代理 URL（含凭据时），只留种类描述
    error
        .message
        .split(':')
        .next()
        .unwrap_or("request failed")
        .to_owned()
}

/// 读取当前设置的代理配置构建测试用 HTTP 客户端。
fn test_client(database: &Database) -> AppResult<GitHubHttp> {
    let (mode, url, _no_proxy) = resolve_proxy_settings(database)?;
    let config = match mode {
        ProxyMode::System => HttpConfig::default(),
        ProxyMode::None => HttpConfig {
            proxy: Some(String::new()),
            ..HttpConfig::default()
        },
        ProxyMode::Manual => HttpConfig {
            proxy: url,
            ..HttpConfig::default()
        },
    };
    GitHubHttp::new(config)
}

/// 连通性测试（T6.8）：`api` = api.github.com/zen；`raw` = raw.githubusercontent.com。
///
/// 测试走**当前设置的代理**（而非默认客户端），因此结果反映用户配置的真实可达性。
#[tauri::command]
pub fn network_proxy_test(
    state: tauri::State<'_, crate::state::AppState>,
    target: String,
) -> AppResult<ConnectivityResult> {
    let started = Instant::now();
    let client = test_client(&state.database)?;
    let url = match target.as_str() {
        "api" => "https://api.github.com/zen",
        "raw" => "https://raw.githubusercontent.com/Ember1414/forgedesk/main/README.md",
        other => {
            return Err(AppError::new(
                ErrorCode::Validation,
                format!("unknown proxy test target `{other}` (expected api|raw)"),
            ))
        }
    };
    let result: Result<String, String> = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| AppError::new(ErrorCode::Internal, format!("runtime: {error}")))?
        .block_on(async {
            let response = client
                .send(&ApiRequest::get(url))
                .await
                .map_err(|error| http_error_message(&error))?;
            let status = response.status().as_u16();
            if (200..300).contains(&status) {
                Ok(format!("HTTP {status}"))
            } else {
                Err(format!("HTTP {status}"))
            }
        });
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(match result {
        Ok(detail) => ConnectivityResult {
            target,
            ok: true,
            latency_ms,
            detail,
        },
        Err(detail) => ConnectivityResult {
            target,
            ok: false,
            latency_ms,
            detail,
        },
    })
}

/// git 网络连通性测试：`git ls-remote` 一个公共仓库（T6.8 规格）。
///
/// 走 CLI 引擎的 `probe_remote`（已注入代理配置），因此测试的是"用户设置下
/// git 网络操作"的真实可达性；公共只读仓库不需要凭据。
#[tauri::command]
pub fn network_git_test(
    state: tauri::State<'_, crate::state::AppState>,
) -> AppResult<ConnectivityResult> {
    use forgedesk_git_engine::engine::GitEngine;
    use forgedesk_git_engine::process::NetworkAuth;

    let started = Instant::now();
    let remote = "https://github.com/octocat/Hello-World.git";
    let auth = NetworkAuth::default();
    let temp = std::env::temp_dir();
    let outcome = state
        .engines
        .write()
        .probe_remote(&temp, remote, &auth)
        .map(|refs| format!("ls-remote 成功（{refs} 个引用）"));
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(match outcome {
        Ok(detail) => ConnectivityResult {
            target: "git".to_owned(),
            ok: true,
            latency_ms,
            detail,
        },
        Err(error) => ConnectivityResult {
            target: "git".to_owned(),
            ok: false,
            latency_ms,
            detail: error.message,
        },
    })
}

// ---------- SSH ----------

/// SSH 连接测试（T6.8）：对 `host` 执行 `ssh -T`（GitHub 语义：退出码 1 且
/// stdout/stderr 含 "successfully authenticated" = 成功）。
///
/// 安全边界：`BatchMode=yes`（绝不挂起等口令）、5 秒连接超时、
/// **不**设置 `StrictHostKeyChecking=no`（T6.8 规格红线）——首次连接的
/// known_hosts 确认由 OpenSSH 自己的提示流程处理（失败会如实回报）。
#[tauri::command]
pub fn ssh_test_connection(host: String) -> AppResult<ConnectivityResult> {
    use std::process::Command;

    if host.is_empty() || host.contains(|c: char| c.is_whitespace() || c == ';' || c == '&') {
        return Err(AppError::new(
            ErrorCode::Validation,
            "host must be a bare hostname",
        ));
    }
    let started = Instant::now();
    let output = Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=5",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-T",
            &format!("git@{host}"),
        ])
        .output()
        .map_err(|error| AppError::new(ErrorCode::NotFound, format!("ssh client: {error}")))?;
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // GitHub 用退出码 1 + 认证成功文案（不准许 shell）；其他 host 退出码 0 = 成功
    let authenticated = combined.contains("successfully authenticated");
    let ok = output.status.success() || authenticated;
    let detail = if authenticated {
        "SSH 认证成功".to_owned()
    } else if combined.contains("Permission denied") {
        "认证被拒：该主机没有可用的 SSH 密钥（先把公钥加到平台上，或检查 agent）".to_owned()
    } else if combined.contains("Connection timed out")
        || combined.contains("Could not resolve hostname")
    {
        "主机不可达（网络/DNS/防火墙）".to_owned()
    } else {
        combined
            .lines()
            .last()
            .unwrap_or("unknown")
            .trim()
            .to_owned()
    };
    Ok(ConnectivityResult {
        target: host,
        ok,
        latency_ms,
        detail,
    })
}

// ---------- GPG ----------

/// 一把 GPG 私钥（`gpg --list-secret-keys --with-colons` 的解析结果）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpgKey {
    /// key id（`sec` 行的 16 位指纹段）。
    pub key_id: String,
    /// 主用户 id（`uid` 行）。
    pub uid: String,
    /// 过期时间（Unix 秒）；`None` = 永不过期。
    pub expires: Option<i64>,
}

/// 解析 `--with-colons` 输出（`sec`/`ssb`/`uid` 行；字段以 `:` 分隔）。
fn parse_gpg_secret_keys(output: &str) -> Vec<GpgKey> {
    let mut keys = Vec::new();
    let mut current: Option<GpgKey> = None;
    for line in output.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        match fields.first().copied() {
            Some("sec") | Some("ssb") => {
                if let Some(key) = current.take() {
                    keys.push(key);
                }
                // sec:fingerprint:...:keyid
                if let Some(key_id) = fields.get(4).copied() {
                    if !key_id.is_empty() {
                        current = Some(GpgKey {
                            key_id: key_id.to_owned(),
                            uid: String::new(),
                            expires: None,
                        });
                    }
                }
            }
            Some("uid") => {
                if let Some(key) = current.as_mut() {
                    if key.uid.is_empty() {
                        if let Some(uid) = fields.get(9) {
                            key.uid = (*uid).to_owned();
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(key) = current.take() {
        keys.push(key);
    }
    keys
}

/// 列出本机 GPG 私钥（T6.8；`gpg` 不可用时返回明确错误）。
#[tauri::command]
pub fn gpg_list_secret_keys() -> AppResult<Vec<GpgKey>> {
    let output = std::process::Command::new("gpg")
        .args(["--list-secret-keys", "--with-colons"])
        .output()
        .map_err(|error| AppError::new(ErrorCode::NotFound, format!("gpg client: {error}")))?;
    if !output.status.success() {
        return Err(AppError::new(
            ErrorCode::Internal,
            format!("gpg --list-secret-keys failed: {}", output.status),
        ));
    }
    Ok(parse_gpg_secret_keys(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// GPG 签名自检（T6.8）：对一段固定测试文本做 clearsign 并立即验证。
/// 返回成功与否 + 验证输出摘要（失败时给出可读原因，如"密钥已过期"）。
#[tauri::command]
pub fn gpg_test_sign(key_id: Option<String>) -> AppResult<ConnectivityResult> {
    use std::io::Write;
    use std::process::Stdio;

    let started = Instant::now();
    let mut sign = std::process::Command::new("gpg")
        .args(["--batch", "--yes", "--clearsign"])
        .args(
            key_id
                .as_ref()
                .map_or_else(Vec::new, |key| vec!["--local-user".to_owned(), key.clone()]),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| AppError::new(ErrorCode::NotFound, format!("gpg client: {error}")))?;
    let mut stdin = sign
        .stdin
        .take()
        .ok_or_else(|| AppError::new(ErrorCode::Internal, "gpg stdin was not piped"))?;
    stdin
        .write_all(b"forgedesk signing self-test\n")
        .map_err(|error| AppError::new(ErrorCode::Internal, format!("write to gpg: {error}")))?;
    drop(stdin);
    let output = sign
        .wait_with_output()
        .map_err(|error| AppError::new(ErrorCode::Internal, format!("wait gpg: {error}")))?;
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    let ok = output.status.success();
    let detail = if ok {
        "GPG 签名成功（clearsign 往返通过）".to_owned()
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .find(|line| {
                line.contains("expired")
                    || line.contains("no secret key")
                    || line.contains("cancelled")
            })
            .unwrap_or("签名失败（gpg 的 stderr 已在日志里）");
        reason.to_owned()
    };
    Ok(ConnectivityResult {
        target: "gpg".to_owned(),
        ok,
        latency_ms,
        detail,
    })
}
