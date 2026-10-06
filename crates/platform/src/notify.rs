//! 系统通知（T6.9）：后台任务完成、更新可用、冲突待处理时的桌面提醒。
//!
//! # 为什么本模块只有 trait 与日志实现
//!
//! 真正的桌面通知（Windows Toast 需要 AUMID——由安装器注册开始菜单快捷方式，
//! 属于 M7 的打包工作；Linux 依赖 dbus 会引入新的重型依赖；macOS 需要通知授权
//! 流程）。在 M6 阶段引入一条无法三平台验证的通知链路，等于把一个未验证的
//! 系统行为埋进代码。因此这里冻结**调用方接口**（services 层从现在就可以接入），
//! 真实实现随 T7.1（更新器）一起落地并做三平台手工验证。
//!
//! 日志实现不是占位废品：所有通知照常经过 tracing 落盘，"任务完成却没弹通知"
//! 的排查从此有证据可查（red line R8：日志内容不含敏感数据，标题/正文由调用方保证）。

/// 通知的语义类别（前端/实现可据此选择图标与声音策略）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    /// 中性信息（如"任务已排队"）。
    Info,
    /// 成功（如"后台任务完成"）。
    Success,
    /// 警告（如"监听器溢出，已降级轮询"）。
    Warning,
    /// 需要处理（如"冲突需要你决定"）。
    Danger,
}

/// 一条待展示的通知。标题与正文都应是 i18n 后的最终文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// 标题（短，一行）。
    pub title: String,
    /// 正文（可选）。
    pub body: Option<String>,
    /// 类别。
    pub kind: NotificationKind,
}

impl Notification {
    /// 快速构造（正文可选）。
    pub fn new(kind: NotificationKind, title: impl Into<String>, body: Option<String>) -> Self {
        Self {
            title: title.into(),
            body,
            kind,
        }
    }
}

/// 通知通道。
///
/// 实现必须**不阻塞、不失败**：通知送不出去不该影响主流程，
/// 所以方法无返回值，实现内部记日志兜底。
pub trait Notifier: Send + Sync {
    /// 发送一条通知（尽力而为）。
    fn notify(&self, notification: Notification);
}

/// 日志实现：当前默认。每次通知都写成结构化日志。
#[derive(Debug, Default)]
pub struct LoggingNotifier;

impl Notifier for LoggingNotifier {
    fn notify(&self, notification: Notification) {
        let Notification { title, body, kind } = notification;
        match kind {
            NotificationKind::Info => {
                tracing::info!(target: "notifier", title = %title, body = body.as_deref(), "notification")
            }
            NotificationKind::Success => {
                tracing::info!(target: "notifier", title = %title, body = body.as_deref(), "notification")
            }
            NotificationKind::Warning => {
                tracing::warn!(target: "notifier", title = %title, body = body.as_deref(), "notification")
            }
            NotificationKind::Danger => {
                tracing::error!(target: "notifier", title = %title, body = body.as_deref(), "notification")
            }
        }
    }
}

/// 当前平台的通知通道。
pub fn notifier_for_current_platform() -> Box<dyn Notifier> {
    // T7 引入真实 toast 前的统一入口；调用方现在就可以依赖这个函数
    Box::new(LoggingNotifier)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn the_default_notifier_never_panics_on_any_kind() {
        let notifier = notifier_for_current_platform();
        for kind in [
            NotificationKind::Info,
            NotificationKind::Success,
            NotificationKind::Warning,
            NotificationKind::Danger,
        ] {
            notifier.notify(Notification::new(kind, "标题", Some("正文".to_owned())));
            notifier.notify(Notification::new(kind, "无正文", None));
        }
    }
}
