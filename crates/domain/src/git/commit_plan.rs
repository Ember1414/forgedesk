//! 提交计划（T1.7）：`prepare` 与 `execute` 之间的那份数据，以及围绕它的纯逻辑。
//!
//! # 为什么要有"计划"这个中间物
//!
//! 用户点"提交"时实际想做的是两件事：**先看清我要提交什么**，然后**执行**。
//! 只有一次调用的话，界面就无法在真正碰仓库之前把"将要发生的事"摆出来；
//! 而红线 R7 要求的正是"计划预览 → 快照 → 执行 → 可回滚"。
//!
//! 因此 `prepare` 产出一份 [`CommitPlan`]（纯数据），`execute` 只认 `plan_id`：
//! 用户看到的与最终执行的**是同一份数据**，而不是两次独立计算的结果
//! （后者会在两次计算之间悄悄变化——那正是提交出意料之外内容的经典成因）。
//!
//! # 有效期与指纹
//!
//! 计划有 [`COMMIT_PLAN_TTL_MS`] 的有效期，并记住准备时刻的**索引指纹**
//! （[`CommitPlan::index_fingerprint`]）。两者拦住的是两类不同的意外：
//!
//! - 有效期：用户开着预览对话框去喝了杯咖啡，回来时仓库可能已经被别处改过；
//! - 指纹：用户在另一个终端里又 `git add` 了一次。这一条更危险——它会让
//!   执行结果与预览里列出的文件清单不一致。
//!
//! # 本模块不做什么
//!
//! 不接触 IO（domain 层禁止 IO）、不生成 `plan_id`（由服务层给）、
//! 不写审计与快照（服务层的事）。

use super::commit::Signature;
use super::path::RepoPath;
use super::spec::AmendMode;
use super::status::ChangeKind;

/// 提交计划的有效期（毫秒）。
pub const COMMIT_PLAN_TTL_MS: i64 = 5 * 60 * 1000;

/// subject 的建议长度上限（**字符数**，不是字节数）。
///
/// 用字符数而不是字节数：中文一个字符占 3 字节，按字节判定会让 25 个汉字就报"过长"，
/// 而用户看到的首行明明只有 25 列。真正的显示宽度还要考虑全角/半角混合，
/// 那需要一张 Unicode 宽度表（额外依赖），对于"给个建议"来说不值得。
pub const SUBJECT_RECOMMENDED_MAX_CHARS: usize = 72;

/// 文件数超过它时，等价命令改用 `-F <file>` 形式。
///
/// 阈值来自本任务的定义：`-m` 形式在文件多时不再是"能直接粘贴执行的命令"，
/// 而是一条几百字符的长命令——那种命令用户不会去核对，等于没有等价命令。
pub const EQUIVALENT_COMMAND_FILE_LIMIT: usize = 20;

/// 空树的 oid：索引里没有任何条目时 `git write-tree` 给出的值。
///
/// 服务层用它判定"没有可提交的内容"——不需要多跑一次命令（见
/// [`CommitPlan::stages_nothing`]）。
pub const EMPTY_TREE_OID: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// GPG 签名模式。
///
/// 三态而不是 `bool`：`Auto`（跟随仓库/全局配置）与 `No`（显式不签名）在 git 里是
/// 两条不同的路径——`Auto` 不传任何签名参数，`No` 传 `--no-gpg-sign`（用于覆盖
/// 用户配置里的 `commit.gpgsign=true`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignMode {
    /// 跟随仓库 / 全局配置（不传签名参数）。
    #[default]
    Auto,
    /// 显式签名（`--gpg-sign`）。
    Yes,
    /// 显式不签名（`--no-gpg-sign`）。
    No,
}

impl SignMode {
    /// 稳定的短名（IPC 用；前端据此走 i18n）。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Yes => "yes",
            Self::No => "no",
        }
    }

    /// 从短名解析；未知值返回 `None`（调用方转成 `VALIDATION`，不静默降级）。
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "auto" => Some(Self::Auto),
            "yes" => Some(Self::Yes),
            "no" => Some(Self::No),
            _ => None,
        }
    }

    /// 映射到 [`super::spec::CommitSpec::sign`]。
    pub const fn as_commit_flag(self) -> Option<bool> {
        match self {
            Self::Auto => None,
            Self::Yes => Some(true),
            Self::No => Some(false),
        }
    }
}

/// 提交信息的审查结论（**只提示，不阻断**——除了空信息）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageReview {
    /// 首行（subject），已去掉行尾空白。
    pub subject: String,
    /// 首行的字符数。
    pub subject_chars: usize,
    /// 发现的问题（可能为空）。
    pub issues: Vec<MessageIssue>,
}

impl MessageReview {
    /// 是否存在**阻断性**问题（空信息）。其余问题只是建议。
    pub fn is_blocking(&self) -> bool {
        self.issues.iter().any(|issue| issue.is_blocking())
    }

    /// 不阻断的问题（供界面做提示）。
    pub fn warnings(&self) -> impl Iterator<Item = MessageIssue> + '_ {
        self.issues
            .iter()
            .copied()
            .filter(|issue| !issue.is_blocking())
    }
}

/// 提交信息的问题。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageIssue {
    /// 信息是空的。
    Empty,
    /// 信息只有空白字符。
    Blank,
    /// 首行超过建议长度（不阻断）。
    SubjectTooLong,
}

impl MessageIssue {
    /// 稳定的短名（IPC 用；前端据此走 i18n，后端不产出文案）。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Blank => "blank",
            Self::SubjectTooLong => "subjectTooLong",
        }
    }

    /// 是否阻断提交。只有"没有信息"才是硬错误，长度只是提醒。
    pub const fn is_blocking(self) -> bool {
        matches!(self, Self::Empty | Self::Blank)
    }
}

/// 计划里将要被提交的一个文件。
///
/// 带上索引侧状态（`A` / `M` / `D` / `R`）而不是只有路径：预览对话框要按
/// "新增 / 修改 / 删除"分组展示，而"再查一次状态"既多一次 IO，也可能与计划生成
/// 的那一刻不一致——后者正是"计划"这个机制要防的事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// 仓库内路径。
    pub path: RepoPath,
    /// 索引侧状态。
    pub index_status: ChangeKind,
}

/// 一份待执行的提交计划。
///
/// 字段顺序与 `docs/API.md` 的 DTO 一致，便于逐项核对（DTO 只是它的投影）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitPlan {
    /// 计划 id（同一个进程内唯一；`execute` 只认它）。
    pub plan_id: String,
    /// 存储层记录 id（`repositories.id`）。
    pub repo_id: i64,
    /// **索引里将要被提交的全部文件**（准备时刻的快照）。
    pub files: Vec<PlannedFile>,
    /// 已合成的完整提交信息（首行 + 空行 + 正文 + 结尾换行）。
    pub message: String,
    /// 正文（与 `message` 里的正文段一致；分开保存是为了预览对话框能分栏显示）。
    pub description: Option<String>,
    /// 覆盖作者身份（`--author`）。
    pub author: Option<Signature>,
    /// 签名模式。
    pub sign: SignMode,
    /// 是否加 `Signed-off-by`（`--signoff`）。
    pub sign_off: bool,
    /// 是否跳过钩子（`--no-verify`）。
    pub no_verify: bool,
    /// 是否 amend 上一个提交。
    pub amend: bool,
    /// amend 的语义（T1.8）。`amend` 为假时无语义，取默认值。
    pub amend_mode: AmendMode,
    /// HEAD 是否（可能）已经存在于某个远程跟踪分支上（T1.8）。
    ///
    /// 只用于**提示**：为真时界面要说清"改写这个提交会让本地与远端分歧、
    /// 之后推送需要 force-with-lease"。判定依据是本地 `refs/remotes/*`，
    /// 它可能过期，因此文案必须是"可能已推送"而不是"已推送"。
    pub head_pushed: bool,
    /// 将要执行的钩子名（仓库里存在且可执行的那些）。
    pub hooks: Vec<String>,
    /// 等价的 git 命令（可复制到终端执行）。
    pub equivalent_command: String,
    /// 准备时刻的 HEAD oid（空仓库为 `None`）。
    pub head_oid: Option<String>,
    /// 准备时刻的**索引指纹**（`git write-tree` 的树 oid）。
    pub index_fingerprint: String,
    /// 准备时刻（Unix 毫秒）。
    pub created_at_ms: i64,
    /// 消息审查结论（含不阻断的建议）。
    pub review: MessageReview,
}

impl CommitPlan {
    /// 计划是否已过期。
    ///
    /// `now_ms` 由调用方传入（domain 不读时钟，测试才能构造"刚好过期"这一瞬间）。
    /// 时钟回拨（`now < created_at`）按"未过期"处理：此时拒绝执行的伤害
    /// （用户刚写好的信息被丢掉）大于收益。
    pub const fn is_expired(&self, now_ms: i64) -> bool {
        now_ms - self.created_at_ms >= COMMIT_PLAN_TTL_MS
    }

    /// 这次提交是否会**什么都不提交**。
    ///
    /// 两种情况都算：索引是空的（[`EMPTY_TREE_OID`]），或索引内容与 HEAD 的树完全相同
    /// （用户 `git add` 之后又把内容改回去了）。后者用 `git commit` 自己的话说是
    /// "nothing to commit"——提前判出来才能给出"没有暂存内容"这种可操作的提示。
    /// amend 不适用：amend 允许只改提交信息。
    pub fn stages_nothing(&self, head_tree: Option<&str>) -> bool {
        if self.amend {
            // amend 可以只改信息：索引与 HEAD 相同完全是正常用法
            return false;
        }
        if self.index_fingerprint == EMPTY_TREE_OID {
            return true;
        }
        head_tree == Some(self.index_fingerprint.as_str())
    }
}

/// [`equivalent_command`] 的入参。
///
/// 用具名结构而不是一长串参数：这个函数的调用点要逐个核对"命令里到底会出现什么"，
/// 位置参数会让 `sign_off` 与 `no_verify` 这类同类型布尔量极易调错顺序。
#[derive(Debug, Clone, Copy)]
pub struct EquivalentCommandInput<'a> {
    /// 已合成的完整提交信息。
    pub message: &'a str,
    /// 是否 amend。
    pub amend: bool,
    /// 签名模式。
    pub sign: SignMode,
    /// 是否 `--signoff`。
    pub sign_off: bool,
    /// 是否 `--no-verify`。
    pub no_verify: bool,
    /// 覆盖作者。
    pub author: Option<&'a Signature>,
    /// 将要提交的文件数（决定用 `-m` 还是 `-F`）。
    pub file_count: usize,
}

/// 生成等价的 git 命令。
///
/// # 引号规则（只说一次，实现在 [`quote_argument`]）
///
/// 生成的命令按 **POSIX sh** 的规则引用：参数用双引号包起来，并转义 `\`、`"`、`$`
/// 与反引号——这四者在双引号内仍然有特殊含义。换行**不转义**（双引号内换行合法，
/// 转义反而会让用户看不懂）。
///
/// 为什么不按 Windows `cmd.exe` 生成：同一份文本在两种 shell 下的正确引用方式不同，
/// 而"发出去的文案只有一份"。选 POSIX 是因为它是可粘贴性最好的形态
/// （Git Bash / WSL / macOS / Linux 都一致），而 Windows 用户多数也用 Git Bash。
/// 需要 cmd 形态时应当由前端再包一层，而不是让 domain 猜用户在哪个 shell 里。
pub fn equivalent_command(input: EquivalentCommandInput<'_>) -> String {
    let mut flags: Vec<String> = Vec::new();
    if input.amend {
        flags.push("--amend".to_owned());
    }
    if input.sign_off {
        flags.push("--signoff".to_owned());
    }
    if input.no_verify {
        flags.push("--no-verify".to_owned());
    }
    match input.sign {
        SignMode::Yes => flags.push("--gpg-sign".to_owned()),
        SignMode::No => flags.push("--no-gpg-sign".to_owned()),
        SignMode::Auto => {}
    }
    if let Some(author) = input.author {
        flags.push(format!("--author={}", quote_argument(&author.display())));
    }

    if input.file_count > EQUIVALENT_COMMAND_FILE_LIMIT {
        // 文件多 → 命令太长，改成"先写文件再 -F"的两步形态
        let mut command = String::from("git commit");
        for flag in &flags {
            command.push(' ');
            command.push_str(flag);
        }
        command.push_str(" -F <message-file>");
        return format!(
            "# {file_count} 个文件，命令过长；把提交信息写入一个文件（UTF-8，无 BOM）后执行：\n{command}",
            file_count = input.file_count,
        );
    }

    let mut command = String::from("git commit");
    for flag in &flags {
        command.push(' ');
        command.push_str(flag);
    }
    for paragraph in message_paragraphs(input.message) {
        command.push_str(" -m ");
        command.push_str(&quote_argument(&paragraph));
    }
    command
}

/// 合成完整提交信息。
///
/// 规则：`subject` 与正文都去掉**两端**空白；正文为空时不追加空行；结尾统一一个换行。
///
/// 两端都要去：界面把首行与正文分成两个输入框，粘贴时带上前导/尾随空白是常事，
/// 而这会让 `git log --oneline` 里出现一个缩进的假象（用户会以为是 git 干的）。
/// 中间的空白一律保留——它是内容的一部分。
///
/// 结尾统一带一个换行是 git 的惯例（`git commit -m` 也这么写），
/// 也让用户在编辑器里打开 `.git/COMMIT_EDITMSG` 时看到正常结束的文件。
pub fn compose_message(subject: &str, description: Option<&str>) -> String {
    let subject = subject.trim();
    let description = description.map(str::trim).filter(|body| !body.is_empty());
    match description {
        Some(body) => format!("{subject}\n\n{body}\n"),
        None => format!("{subject}\n"),
    }
}

/// 审查提交信息（首行长度、是否为空）。
pub fn review_message(message: &str) -> MessageReview {
    let subject = message.lines().next().unwrap_or("").trim_end().to_owned();
    let subject_chars = subject.chars().count();

    let mut issues = Vec::new();
    if message.is_empty() {
        issues.push(MessageIssue::Empty);
    } else if message.trim().is_empty() {
        issues.push(MessageIssue::Blank);
    } else if subject_chars > SUBJECT_RECOMMENDED_MAX_CHARS {
        issues.push(MessageIssue::SubjectTooLong);
    }

    MessageReview {
        subject,
        subject_chars,
        issues,
    }
}

/// 按空行切分提交信息的段落（`-m` 的语义：每段之间会自动插入一个空行）。
fn message_paragraphs(message: &str) -> Vec<String> {
    message
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .map(str::to_owned)
        .collect()
}

/// 按 POSIX sh 规则引用一个参数（见 [`equivalent_command`] 的引号规则）。
fn quote_argument(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(character);
            }
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        compose_message, equivalent_command, review_message, AmendMode, CommitPlan,
        EquivalentCommandInput, MessageIssue, SignMode, EMPTY_TREE_OID,
        SUBJECT_RECOMMENDED_MAX_CHARS,
    };
    use crate::git::Signature;

    fn plan(amend: bool, fingerprint: &str) -> CommitPlan {
        CommitPlan {
            plan_id: "p".to_owned(),
            repo_id: 1,
            files: Vec::new(),
            message: "subject\n".to_owned(),
            description: None,
            author: None,
            sign: SignMode::Auto,
            sign_off: false,
            no_verify: false,
            amend,
            amend_mode: AmendMode::default(),
            head_pushed: false,
            hooks: Vec::new(),
            equivalent_command: String::new(),
            head_oid: Some("a".repeat(40)),
            index_fingerprint: fingerprint.to_owned(),
            created_at_ms: 1_000,
            review: review_message("subject\n"),
        }
    }

    // ---------------------------------------------------------------- 消息合成

    #[test]
    fn a_subject_without_a_body_gets_no_empty_paragraph() {
        assert_eq!(compose_message("  fix: thing  ", None), "fix: thing\n");
        assert_eq!(compose_message("fix: thing", Some("   ")), "fix: thing\n");
    }

    #[test]
    fn a_body_is_separated_by_exactly_one_empty_line() {
        assert_eq!(
            compose_message("fix: thing", Some("\n why it broke \n")),
            "fix: thing\n\nwhy it broke\n"
        );
    }

    #[test]
    fn only_the_outer_whitespace_of_the_subject_is_trimmed() {
        // 两端是粘贴带进来的空白（界面把首行与正文分成两个输入框，这很常见），
        // 中间的内容一个字都不许动
        assert_eq!(compose_message("\n  fix: a b  \n", None), "fix: a b\n");
    }

    // ---------------------------------------------------------------- 消息审查

    #[test]
    fn an_empty_message_is_blocking_and_a_long_subject_is_only_a_hint() {
        let empty = review_message("");
        assert!(empty.is_blocking());
        assert!(empty.issues.contains(&MessageIssue::Empty));

        let blank = review_message("   \n\t\n");
        assert!(blank.is_blocking());
        assert!(blank.issues.contains(&MessageIssue::Blank));

        let long = review_message(&"x".repeat(SUBJECT_RECOMMENDED_MAX_CHARS + 1));
        assert!(!long.is_blocking(), "首行过长只是建议，不能阻断提交");
        assert!(long.issues.contains(&MessageIssue::SubjectTooLong));
        assert_eq!(long.subject_chars, SUBJECT_RECOMMENDED_MAX_CHARS + 1);
    }

    #[test]
    fn the_subject_length_is_counted_in_characters_not_bytes() {
        // 25 个汉字是 75 字节，但只有 25 列：按字节判定会误报
        let review = review_message(&"中".repeat(25));
        assert_eq!(review.subject_chars, 25);
        assert!(!review.issues.contains(&MessageIssue::SubjectTooLong));
    }

    #[test]
    fn the_review_subject_ignores_trailing_whitespace_and_takes_the_first_line() {
        let review = review_message("fix: thing   \n\nbody\n");
        assert_eq!(review.subject, "fix: thing");
    }

    // ---------------------------------------------------------------- 有效期

    #[test]
    fn a_plan_expires_exactly_at_the_ttl_boundary() {
        let plan = plan(false, EMPTY_TREE_OID);

        assert!(!plan.is_expired(1_000));
        assert!(!plan.is_expired(1_000 + super::COMMIT_PLAN_TTL_MS - 1));
        assert!(plan.is_expired(1_000 + super::COMMIT_PLAN_TTL_MS));
    }

    #[test]
    fn a_clock_that_jumped_backwards_does_not_expire_the_plan() {
        // 时钟回拨（NTP 校正、用户改系统时间）不该让刚写好的提交信息被丢掉
        assert!(!plan(false, EMPTY_TREE_OID).is_expired(0));
    }

    // ---------------------------------------------------------------- 空提交判定

    #[test]
    fn an_empty_index_stages_nothing() {
        assert!(plan(false, EMPTY_TREE_OID).stages_nothing(None));
        assert!(plan(false, EMPTY_TREE_OID).stages_nothing(Some("whatever")));
    }

    #[test]
    fn an_index_identical_to_head_stages_nothing() {
        let head_tree = "b".repeat(40);
        assert!(plan(false, &head_tree).stages_nothing(Some(&head_tree)));
        assert!(!plan(false, &head_tree).stages_nothing(Some(&"c".repeat(40))));
    }

    #[test]
    fn amend_may_stage_nothing_because_it_can_only_change_the_message() {
        assert!(!plan(true, EMPTY_TREE_OID).stages_nothing(None));
    }

    // ---------------------------------------------------------------- 等价命令

    #[test]
    fn the_equivalent_command_uses_one_m_flag_per_paragraph() {
        let command = equivalent_command(EquivalentCommandInput {
            message: "fix: thing\n\nwhy it broke\n",
            amend: false,
            sign: SignMode::Auto,
            sign_off: false,
            no_verify: false,
            author: None,
            file_count: 3,
        });

        assert_eq!(command, "git commit -m \"fix: thing\" -m \"why it broke\"");
    }

    #[test]
    fn the_equivalent_command_quotes_the_characters_a_shell_would_expand() {
        let command = equivalent_command(EquivalentCommandInput {
            message: "fix: $HOME and `whoami` and \"quotes\" and back\\slash\n",
            amend: false,
            sign: SignMode::Auto,
            sign_off: false,
            no_verify: false,
            author: None,
            file_count: 1,
        });

        assert!(
            command.contains("\\$HOME"),
            "美元符号必须转义，否则复制执行时会展开成用户的主目录：{command}"
        );
        assert!(
            command.contains("\\`whoami\\`"),
            "反引号必须转义：{command}"
        );
        assert!(
            command.contains("\\\"quotes\\\""),
            "双引号必须转义：{command}"
        );
        assert!(
            command.contains("back\\\\slash"),
            "反斜杠必须转义：{command}"
        );
    }

    #[test]
    fn every_switch_the_plan_carries_appears_in_the_command() {
        let author = Signature::new("Ada", "ada@example.com");
        let command = equivalent_command(EquivalentCommandInput {
            message: "subject\n",
            amend: true,
            sign: SignMode::Yes,
            sign_off: true,
            no_verify: true,
            author: Some(&author),
            file_count: 1,
        });

        for expected in [
            "--amend",
            "--signoff",
            "--no-verify",
            "--gpg-sign",
            "--author=\"Ada <ada@example.com>\"",
        ] {
            assert!(command.contains(expected), "缺少 {expected}：{command}");
        }
    }

    #[test]
    fn the_auto_sign_mode_passes_no_signing_flag_at_all() {
        let command = equivalent_command(EquivalentCommandInput {
            message: "subject\n",
            amend: false,
            sign: SignMode::Auto,
            sign_off: false,
            no_verify: false,
            author: None,
            file_count: 1,
        });

        assert!(!command.contains("gpg-sign"));
    }

    #[test]
    fn many_files_fall_back_to_the_message_file_form() {
        let command = equivalent_command(EquivalentCommandInput {
            message: "subject\n",
            amend: false,
            sign: SignMode::Auto,
            sign_off: false,
            no_verify: false,
            author: None,
            file_count: super::EQUIVALENT_COMMAND_FILE_LIMIT + 1,
        });

        assert!(command.contains("-F <message-file>"));
        assert!(
            command.contains("21 个文件"),
            "说明里要带上文件数：{command}"
        );
        assert!(!command.contains("-m "), "-F 形式不再逐段 -m：{command}");
    }

    #[test]
    fn sign_mode_keys_round_trip() {
        for mode in [SignMode::Auto, SignMode::Yes, SignMode::No] {
            assert_eq!(SignMode::from_key(mode.key()), Some(mode));
        }
        assert_eq!(
            SignMode::from_key("sometimes"),
            None,
            "未知值必须报错而不是降级"
        );
        assert_eq!(SignMode::Auto.as_commit_flag(), None);
        assert_eq!(SignMode::Yes.as_commit_flag(), Some(true));
        assert_eq!(SignMode::No.as_commit_flag(), Some(false));
    }
}
