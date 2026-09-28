//! 把凭据交给 git：用**应用自身**作为 `GIT_ASKPASS` 辅助程序。
//!
//! # 为什么不写临时脚本（任务里允许的那条降级路线）
//!
//! `GIT_ASKPASS` 的契约是"程序被调用时把答案写到 stdout"。要把明文交给这个过程，
//! 两条路：
//!
//! | 方案 | 明文经过 | 残留风险 | 跨平台 |
//! | --- | --- | --- | --- |
//! | 临时脚本（任务文档给的降级方案） | **磁盘**（脚本里 `echo` 明文，或脚本读另一个临时文件） | 崩溃/被杀时明文留在磁盘上，且要处理权限位与 Windows 的可执行判定 | 需要分别写 sh / bat |
//! | 应用自身 + 环境变量（本实现） | 只经过**环境变量** | 同用户的进程可读该进程的环境（与临时脚本同级，且不留磁盘痕迹） | 天然一致：git 直接执行同一个可执行文件 |
//!
//! 选择后者的关键理由是**明文不落盘**：红线 R8 里最贵的事故就是凭据出现在磁盘上。
//! 环境变量的暴露面是"能读你进程环境的进程"，在单用户桌面机上与临时脚本的
//! 暴露面相同，但少了"忘记删文件"这一类失败模式。
//!
//! # 协议细节
//!
//! git 调用 askpass 程序时把提示语作为**参数**传进来（提示语可能被本地化成任意语言），
//! 我们只按其中的关键词判断它要用户名还是密码：
//!
//! ```text
//! Username for 'https://github.com':          → 用户名
//! Password for 'https://octocat@github.com':  → 令牌/密码
//! Enter passphrase for key '/home/u/.ssh/id_ed25519':  → 当作密码（SSH 口令）
//! ```
//!
//! 认不出的提示语返回 `None`（**不**回答）：把令牌回给一个我们没预期的提示，
//! 等于把它交给一个不明用途的输入框。

use std::path::{Path, PathBuf};

use crate::secret::Secret;

/// 令牌/密码所在的环境变量名。
pub const SECRET_ENV: &str = "FORGEDESK_ASKPASS_SECRET";
/// 用户名所在的环境变量名。
pub const USERNAME_ENV: &str = "FORGEDESK_ASKPASS_USERNAME";
/// askpass 模式的命令行开关。
pub const FLAG: &str = "--askpass";

/// askpass 提示语的类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskpassPrompt {
    /// 要用户名（HTTPS 的第一步）。
    Username,
    /// 要密码 / 令牌 / 私钥口令。
    Secret,
    /// 认不出的提示语——不回答。
    Unknown,
}

/// 按关键词判断提示语要什么。
///
/// 顺序：先判密码类关键词（`passphrase` 里含 `pass`，而 `username` 不含），
/// 再判用户名。两条都不像时返回 [`AskpassPrompt::Unknown`]。
pub fn classify_prompt(prompt: &str) -> AskpassPrompt {
    let text = prompt.to_ascii_lowercase();
    let has = |needle: &str| text.contains(needle);

    if has("password") || has("passphrase") || has("token") || has("secret") {
        return AskpassPrompt::Secret;
    }
    if has("username") || has("user name") || has("login") || has("account") {
        return AskpassPrompt::Username;
    }
    AskpassPrompt::Unknown
}

/// 一次网络操作要用的 askpass 方案：程序路径 + 要注入的环境变量。
///
/// `Debug` 手写：结构里含 [`Secret`]，不允许被顺手 `{:?}` 出去。
pub struct AskpassPlan {
    program: PathBuf,
    username: String,
    secret: Secret,
}

impl std::fmt::Debug for AskpassPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AskpassPlan")
            .field("program", &self.program)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

impl AskpassPlan {
    /// 构造方案；`secret` 为空时返回 `None`（空令牌等于没登录，不该注入）。
    pub fn new(
        program: impl Into<PathBuf>,
        username: impl Into<String>,
        secret: &Secret,
    ) -> Option<Self> {
        if secret.is_empty() {
            return None;
        }
        Some(Self {
            program: program.into(),
            username: username.into(),
            secret: secret.clone(),
        })
    }

    /// askpass 程序路径（传给 git 的 `GIT_ASKPASS`）。
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// 要注入到 git 进程的环境变量（**含明文**，只允许交给那一个子进程）。
    ///
    /// 调用方必须保证：这些变量只出现在这次网络操作的环境里，
    /// 不进日志、不进审计、不进错误详情。
    pub fn env(&self) -> Vec<(String, String)> {
        vec![
            (USERNAME_ENV.to_owned(), self.username.clone()),
            (SECRET_ENV.to_owned(), self.secret.expose().to_owned()),
        ]
    }
}

/// argv 里是否包含 askpass 开关（应用被 git 当作辅助程序拉起）。
///
/// 只看**第一个参数之后**的位置：`--askpass` 出现在参数列表里本身就说明
/// 这是一次 askpass 调用，但把它写在可执行文件路径位置上是不可能的，
/// 因此从索引 1 开始扫更稳（避免把路径里恰好含 `--askpass` 的怪情况算进来）。
pub fn is_askpass_invocation(args: &[String]) -> bool {
    args.iter().skip(1).any(|arg| arg == FLAG)
}

/// 从 argv 里取出提示语（`--askpass` 之后的全部内容，空格连接）。
pub fn prompt_from_args(args: &[String]) -> String {
    let position = args
        .iter()
        .position(|arg| arg == FLAG)
        .map(|index| index + 1)
        .unwrap_or(args.len());
    args.get(position..)
        .unwrap_or(&[])
        .join(" ")
        .trim()
        .to_owned()
}

/// 按提示语给出答案；`lookup` 是环境变量读取器（测试里注入）。
///
/// 认不出的提示语返回 `None`：调用方据此以非零码退出，让 git 报认证失败，
/// 而不是把令牌交给一个我们没预期的输入框。
pub fn answer_for(prompt: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    match classify_prompt(prompt) {
        AskpassPrompt::Username => lookup(USERNAME_ENV),
        AskpassPrompt::Secret => lookup(SECRET_ENV),
        AskpassPrompt::Unknown => None,
    }
}

/// askpass 模式下的应答：**这里是唯一允许直接写 stdout 的地方**。
///
/// `GIT_ASKPASS` 的契约就是"答案从 stdout 返回"，没有别的通道可用
/// （写 stderr 会被 git 当作错误输出）。因此这里显式豁免 `print_stdout`。
#[allow(clippy::print_stdout)]
pub fn write_answer(answer: Option<&str>) {
    match answer {
        Some(value) => {
            println!("{value}");
        }
        None => {
            // 不回答任何东西：git 会因此报认证失败，而不是拿到一个错的值
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn username_and_secret_prompts_are_told_apart() {
        assert_eq!(
            classify_prompt("Username for 'https://github.com': "),
            AskpassPrompt::Username
        );
        assert_eq!(
            classify_prompt("Password for 'https://octocat@github.com': "),
            AskpassPrompt::Secret
        );
        // SSH 私钥口令：必须当密码回答，否则会把它当用户名回给 ssh
        assert_eq!(
            classify_prompt("Enter passphrase for key '/home/u/.ssh/id_ed25519': "),
            AskpassPrompt::Secret
        );
        assert_eq!(
            classify_prompt("Token for 'https://gitlab.com': "),
            AskpassPrompt::Secret
        );
    }

    #[test]
    fn an_unexpected_prompt_is_never_answered() {
        // 把令牌回给一个没预期的输入框，等于把它交给未知用途
        assert_eq!(
            classify_prompt("What is the airspeed velocity of an unladen swallow?"),
            AskpassPrompt::Unknown
        );

        let answer = answer_for("What is 2 + 2?", &|_| Some("ghp_x".to_owned()));
        assert_eq!(answer, None);
    }

    #[test]
    fn answering_reads_the_documented_environment_variables() {
        let lookup = |key: &str| match key {
            USERNAME_ENV => Some("octocat".to_owned()),
            SECRET_ENV => Some("ghp_x".to_owned()),
            _ => None,
        };

        assert_eq!(
            answer_for("Username for 'https://github.com':", &lookup).as_deref(),
            Some("octocat")
        );
        assert_eq!(
            answer_for("Password for 'https://octocat@github.com':", &lookup).as_deref(),
            Some("ghp_x")
        );
    }

    #[test]
    fn the_plan_carries_the_program_and_the_two_environment_variables() {
        let plan = AskpassPlan::new("/opt/forgedesk/forgedesk", "octocat", &Secret::new("ghp_x"))
            .expect("plan");

        assert_eq!(plan.program(), Path::new("/opt/forgedesk/forgedesk"));
        let env: std::collections::BTreeMap<String, String> = plan.env().into_iter().collect();
        assert_eq!(env.get(USERNAME_ENV).map(String::as_str), Some("octocat"));
        assert_eq!(env.get(SECRET_ENV).map(String::as_str), Some("ghp_x"));
    }

    #[test]
    fn an_empty_secret_produces_no_plan_at_all() {
        // 注入一个空令牌只会让 git 拿它去登录并失败，掩盖"其实没有凭据"这个事实
        assert!(AskpassPlan::new("/x/forgedesk", "octocat", &Secret::new("")).is_none());
    }

    #[test]
    fn the_plan_debug_output_does_not_leak_the_secret() {
        let plan = AskpassPlan::new("/x/forgedesk", "octocat", &Secret::new("ghp_supersecret"))
            .expect("plan");

        let text = format!("{plan:?}");

        assert!(text.contains("octocat"));
        assert!(!text.contains("ghp_supersecret"), "{text}");
    }

    #[test]
    fn the_askpass_flag_is_recognised_only_after_the_executable_path() {
        let args: Vec<String> = ["forgedesk", "--askpass", "Password for 'x':"]
            .iter()
            .map(|value| (*value).to_owned())
            .collect();

        assert!(is_askpass_invocation(&args));
        assert_eq!(prompt_from_args(&args), "Password for 'x':");

        // 路径里含 --askpass 不算：位置 0 是程序自身
        let weird: Vec<String> = ["C:/tools/--askpass/forgedesk.exe"]
            .iter()
            .map(|value| (*value).to_owned())
            .collect();
        assert!(!is_askpass_invocation(&weird));
    }

    #[test]
    fn a_prompt_split_into_several_arguments_is_rejoined() {
        // git 在部分平台上会把提示语连同它的参数分开传
        let args: Vec<String> = ["forgedesk", "--askpass", "Password", "for", "'https://x':"]
            .iter()
            .map(|value| (*value).to_owned())
            .collect();

        assert_eq!(prompt_from_args(&args), "Password for 'https://x':");
        assert_eq!(
            classify_prompt(&prompt_from_args(&args)),
            AskpassPrompt::Secret
        );
    }
}
