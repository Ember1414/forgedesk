//! 终端会话注册表（T5.2）：全进程唯一的"哪些终端开着"真相源。
//!
//! 与 [`crate::terminal::PtySession`] 的分工：会话只管"一个 PTY 的生死"，
//! 注册表管"谁开着、归属哪个仓库、关仓库/退应用时怎么收拾"。
//! 命令层把它放进 `AppState`（Arc 共享），`repo_close` 与应用退出路径
//! 都要能从这里找到该停的会话。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use forgedesk_domain::{AppError, ErrorCode};

use super::scrollback::Scrollback;
use super::shell::resolve_shell;
use super::{PtyConfig, PtySession, TerminalCallbacks};

/// 会话创建参数（命令层从 IPC 请求收敛而来）。
#[derive(Debug, Clone)]
pub struct TerminalSpawnSpec {
    /// 归属仓库（存储层记录 id）；`repo_close` 与 UI 提示都按它分组。
    pub repo_id: i64,
    /// 初始工作目录（命令层已校验在仓库根内）。
    pub cwd: PathBuf,
    /// shell id（`term_shell_list` 给出的选项；`None` / `"default"` = 平台默认）。
    pub shell: Option<String>,
    /// 初始列数。
    pub cols: u16,
    /// 初始行数。
    pub rows: u16,
    /// 额外环境变量。
    pub env: std::collections::BTreeMap<String, String>,
}

/// 一个注册在案的终端会话。
pub struct TerminalHandle {
    /// 稳定 id（`term-<n>`，进程内唯一）。
    pub id: String,
    /// 归属仓库。
    pub repo_id: i64,
    /// 实际启动的 shell 程序名。
    pub program: String,
    /// 会话本体（Arc：测试需要把会话递给"模拟 xterm.js 的 DSR 应答"回调）。
    pub session: Arc<PtySession>,
    /// 行式回滚缓冲（会话退出后仍可读最后 1000 行）。
    pub scrollback: Arc<Scrollback>,
}

impl TerminalHandle {
    /// 会话是否已退出。
    #[must_use]
    pub fn is_exited(&self) -> bool {
        self.session.is_exited()
    }
}

/// 会话列表条目（`term_list` 的返回元素）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSummary {
    /// 会话 id。
    pub id: String,
    /// 归属仓库 id。
    pub repo_id: i64,
    /// shell 程序名。
    pub program: String,
    /// 是否已退出。
    pub exited: bool,
}

/// 全进程会话注册表。
#[derive(Default)]
pub struct TerminalRegistry {
    sessions: Mutex<HashMap<String, Arc<TerminalHandle>>>,
    next_id: AtomicU64,
}

impl std::fmt::Debug for TerminalRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TerminalHandle（PTY 句柄）不可 Debug；数量足够诊断用了。
        formatter
            .debug_struct("TerminalRegistry")
            .field("sessions", &self.sessions.lock().len())
            .finish()
    }
}

impl TerminalRegistry {
    /// 空注册表。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建会话并登记。
    ///
    /// 输出回调先过回滚缓冲、再交给调用方的 sink：缓冲是"无论如何都要有"的
    /// （会话退出后查看），sink 是"给前端"的——两者的顺序与存亡互不影响。
    ///
    /// `auto_reply_dsr` 固定 `false`：正式前端的 xterm.js 会原生应答 DSR，
    /// 后端再答一份会把 `\x1b[..R` 混进 shell 的输入流（PTY-SPIKE §3.2）。
    pub fn create(
        &self,
        spec: &TerminalSpawnSpec,
        sink: TerminalCallbacks,
    ) -> Result<Arc<TerminalHandle>, AppError> {
        let scrollback = Arc::new(Scrollback::new());
        let buffer = Arc::clone(&scrollback);
        let user_output = Arc::clone(&sink.on_output);
        let callbacks = TerminalCallbacks {
            on_output: Arc::new(move |data| {
                buffer.push(data);
                user_output(data);
            }),
            on_exit: Arc::clone(&sink.on_exit),
        };
        let config = PtyConfig {
            cwd: spec.cwd.clone(),
            cols: spec.cols,
            rows: spec.rows,
            shell: Some(resolve_shell(spec.shell.as_deref())),
            env: spec.env.clone(),
            auto_reply_dsr: false,
        };
        let session = Arc::new(PtySession::spawn(&config, callbacks)?);
        let id = format!("term-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let handle = Arc::new(TerminalHandle {
            id: id.clone(),
            repo_id: spec.repo_id,
            program: session.program().to_string(),
            session,
            scrollback,
        });
        self.sessions.lock().insert(id, Arc::clone(&handle));
        Ok(handle)
    }

    /// 按 id 取会话。
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<TerminalHandle>> {
        self.sessions.lock().get(id).map(Arc::clone)
    }

    /// 按 id 取会话；不存在时返回 `NOT_FOUND`（命令层的常用形态）。
    pub fn get_or_err(&self, id: &str) -> Result<Arc<TerminalHandle>, AppError> {
        self.get(id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "no such terminal session"))
    }

    /// 移除会话（`term_close`：关标签页）。返回被移除的会话，便于调用方补 close。
    pub fn remove(&self, id: &str) -> Option<Arc<TerminalHandle>> {
        self.sessions.lock().remove(id)
    }

    /// 全部会话的快照（`term_list`）。
    #[must_use]
    pub fn list(&self) -> Vec<TerminalSummary> {
        self.sessions
            .lock()
            .values()
            .map(|handle| TerminalSummary {
                id: handle.id.clone(),
                repo_id: handle.repo_id,
                program: handle.program.clone(),
                exited: handle.is_exited(),
            })
            .collect()
    }

    /// 某仓库还活着的会话数（`repo_close` 的安全网检查用）。
    #[must_use]
    pub fn active_for_repo(&self, repo_id: i64) -> usize {
        self.sessions
            .lock()
            .values()
            .filter(|handle| handle.repo_id == repo_id && !handle.is_exited())
            .count()
    }

    /// 关闭并移除某仓库的全部会话（用户在提示后选择"仍然关闭仓库"）。
    /// 返回关掉了几个。
    pub fn close_for_repo(&self, repo_id: i64) -> usize {
        let mut sessions = self.sessions.lock();
        let ids: Vec<String> = sessions
            .values()
            .filter(|handle| handle.repo_id == repo_id)
            .map(|handle| handle.id.clone())
            .collect();
        let count = ids.len();
        for id in ids {
            if let Some(handle) = sessions.remove(&id) {
                handle.session.close();
            }
        }
        count
    }

    /// 应用退出：关闭全部会话（`src-tauri` 的 RunEvent::Exit 调用）。
    pub fn stop_all(&self) {
        let mut sessions = self.sessions.lock();
        for handle in sessions.values() {
            handle.session.close();
        }
        sessions.clear();
    }
}
