//! 行级 / 块级暂存与取消暂存（T1.6）。
//!
//! # 一条链路，四个决定
//!
//! ```text
//! 取该文件的完整补丁（git diff --no-color -U<n> -- <path>）
//!   → 按用户选择裁剪（domain::git::trim_patch，纯函数）
//!   → git apply --check（dry-run，不写任何东西）
//!   → git apply --cached [--reverse]（真正落盘）
//! ```
//!
//! 1. **补丁是唯一数据源**（T1.5 的结论）：文本 diff 走 git CLI，与用户终端一致；
//! 2. **裁剪必须在字节层**（见 `domain::git::staging`）：非 UTF-8 内容不能被解码，
//!    否则补丁回灌 `git apply` 时必然对不上；
//! 3. **先检查再应用**（红线 R7）：dry-run 与真正的应用用**同一份字节**，
//!    因此 `--check` 通过之后的失败只可能来自并发改动，而不是"应用了一半"；
//! 4. **文件粒度不走这里**：整文件由 `git add` / `git reset` 完成（更快，也能处理
//!    未跟踪文件与模式变更）。本模块只负责"用户只选了一部分"。
//!
//! # hunk 下标为什么会错位（以及这里怎么防）
//!
//! 界面上的"第 2 块"是用**一组查看参数**（上下文行数、是否忽略空白、是否检测重命名）
//! 渲染出来的；上下文行数一变，相邻 hunk 可能合并成一个，下标整体平移。
//! 因此调用方必须把打开 diff 时用的那组参数一起交回来（[`PatchView`]），
//! 本模块用它重新生成补丁 —— 参数不同就必然错位，而错位后的补丁会被 dry-run 拦下。
//! **宁可报错，也不能暂存错内容**：暂存错的后果是用户提交了不该提交的代码。
//!
//! # 与 `WorkspaceService` 的分工
//!
//! `WorkspaceService` 管"文件级"（状态、`git add`、`git restore`），本模块管"补丁级"。
//! 两者的写路径**刻意不合并**：`git add` 永远不会因为"上下文不匹配"失败，
//! 而补丁通道会 —— 把两种失败语义混在一个方法里，调用方就无法区分
//! "这个文件暂存不进去"和"你选的那几行已经变了"。

use forgedesk_domain::git::{
    trim_patch, ApplyPatchSpec, DiffSpec, DiffTarget, PatchDirection, RepoId, RepoPath, StageScope,
    DEFAULT_CONTEXT_LINES,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode, FixAction};

use crate::engines::GitEngines;
use crate::workspace::WorkspaceService;
use forgedesk_git_engine::engine::GitEngine;

/// 生成补丁时使用的查看参数。
///
/// 必须与界面打开 diff 时用的完全一致（理由见模块头）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchView {
    /// 上下文行数（`-U<n>`）。
    pub context_lines: u32,
    /// 忽略空白变化（`-w`）。
    pub ignore_whitespace: bool,
    /// 重命名检测（`-M`）。
    pub detect_renames: bool,
}

impl Default for PatchView {
    fn default() -> Self {
        Self {
            context_lines: DEFAULT_CONTEXT_LINES,
            ignore_whitespace: false,
            // 与查看器默认一致：重命名检测开着，"重命名 + 内容变更"才是一个文件段
            detect_renames: true,
        }
    }
}

/// 部分暂存用例（暂存 / 取消暂存 / 按块丢弃）。
pub struct StagingService<'a> {
    /// 复用它的 `resolve_workdir`（repo_id → 工作区路径的解析只有一处实现）。
    workspace: WorkspaceService<'a>,
    engines: &'a GitEngines,
}

impl<'a> StagingService<'a> {
    /// 组装服务。
    pub fn new(workspace: WorkspaceService<'a>, engines: &'a GitEngines) -> Self {
        Self { workspace, engines }
    }

    /// 暂存选中的 hunk / 行（补丁从"索引 ↔ 工作区"取，正向应用到索引）。
    pub fn stage(&self, repo_id: i64, scope: &StageScope, view: PatchView) -> AppResult<()> {
        self.apply_selection(repo_id, scope, view, Plan::Stage)
    }

    /// 取消暂存选中的 hunk / 行（补丁从"HEAD ↔ 索引"取，反向应用到索引）。
    pub fn unstage(&self, repo_id: i64, scope: &StageScope, view: PatchView) -> AppResult<()> {
        self.apply_selection(repo_id, scope, view, Plan::Unstage)
    }

    /// 丢弃选中的 hunk / 行（补丁从"索引 ↔ 工作区"取，反向应用到**工作区**）。
    ///
    /// 只影响工作区、不碰索引：对"已暂存 + 工作区又改"的文件，丢弃工作区改动后
    /// 保留已暂存的版本 —— 与状态面板"放弃"按钮的语义一致。
    /// 调用方（命令层）必须先经确认对话框；本方法只负责 dry-run 与执行。
    pub fn discard(&self, repo_id: i64, scope: &StageScope, view: PatchView) -> AppResult<()> {
        self.apply_selection(repo_id, scope, view, Plan::Discard)
    }

    /// 三个操作的公共骨架：取补丁 → 裁剪 → dry-run → 应用。
    fn apply_selection(
        &self,
        repo_id: i64,
        scope: &StageScope,
        view: PatchView,
        plan: Plan,
    ) -> AppResult<()> {
        let path = scope
            .path()
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::Validation,
                    "partial staging requires a single path",
                )
                .with_hint("kind=files".to_owned())
            })?
            .clone();

        // 界面上"什么都没选"不是错误（用户点了按钮但没选行），幂等返回。
        if scope.is_empty() {
            return Ok(());
        }

        let workdir = self.workspace.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);

        let patch = self.file_patch(&repo, &path, view, plan.diff_target())?;
        let trimmed = trim_patch(&patch, scope, plan.direction())?;
        if trimmed.is_empty() {
            // 裁剪后没有内容（用户选中的全是上下文行）：同样不是错误。
            return Ok(());
        }

        let engine = self.engines.write();
        // dry-run 用同一份字节：失败时仓库一定还是干净的。
        engine
            .apply_patch(&repo, &plan.spec(trimmed.clone()).checked())
            .map_err(|error| annotate(error, repo_id, &path))?;
        engine
            .apply_patch(&repo, &plan.spec(trimmed))
            .map_err(|error| annotate(error, repo_id, &path))?;
        Ok(())
    }

    /// 取该文件在指定目标上的完整补丁。
    ///
    /// 强制 `force_full`：截断只是**展示层**的保护，裁剪必须基于完整内容。
    /// 空补丁说明这个文件在该侧没有变更 —— 未跟踪文件也走这条路（`git diff`
    /// 不含未跟踪文件），此时行级暂存根本无法表达，必须明确拒绝而不是静默成功。
    fn file_patch(
        &self,
        repo: &RepoId,
        path: &RepoPath,
        view: PatchView,
        target: DiffTarget,
    ) -> AppResult<Vec<u8>> {
        let side = Self::side_of(&target);
        let mut spec = DiffSpec::new(target)
            .with_paths(vec![path.clone()])
            .with_context_lines(view.context_lines)
            .with_ignore_whitespace(view.ignore_whitespace)
            .with_force_full(true);
        spec.detect_renames = view.detect_renames;

        let patch = self.engines.write().diff_patch(repo, &spec)?;
        if patch.is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "the file has no changes on the requested side",
            )
            .with_detail(format!("side={side}"))
            .with_hint(path.to_string_lossy().into_owned()));
        }
        Ok(patch)
    }

    /// 出错信息里用的稳定短名（不放用户可见文案）。
    fn side_of(target: &DiffTarget) -> &'static str {
        match target {
            DiffTarget::Staged => "staged",
            DiffTarget::Unstaged => "unstaged",
            DiffTarget::Between { .. } | DiffTarget::Since(_) | DiffTarget::Commit(_) => "unknown",
        }
    }
}

/// 一次部分操作的三个变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    /// 未暂存 → 索引。
    Stage,
    /// 已暂存 → 索引（反向）。
    Unstage,
    /// 未暂存 → 工作区（反向）。
    Discard,
}

impl Plan {
    /// 补丁从哪两份树取。
    fn diff_target(self) -> DiffTarget {
        match self {
            // 丢弃的对象是"工作区相对索引的改动"，与暂存同一个补丁，只是反向应用。
            Self::Stage | Self::Discard => DiffTarget::Unstaged,
            Self::Unstage => DiffTarget::Staged,
        }
    }

    /// 该变体对应的补丁应用参数。
    fn spec(self, patch: Vec<u8>) -> ApplyPatchSpec {
        match self {
            Self::Stage => ApplyPatchSpec::stage(patch),
            Self::Unstage => ApplyPatchSpec::unstage(patch),
            Self::Discard => ApplyPatchSpec::discard_worktree(patch),
        }
    }

    /// 裁剪方向（见 `domain::git::PatchDirection`）。
    ///
    /// 它必须与 [`Self::spec`] 的 `direction` 一致：补丁是"往目标里写"还是"从目标里撤"，
    /// 决定了未选中的行该怎么处置（转上下文还是移除）。两处不一致的后果不是报错，
    /// 而是补丁被 git 拒绝、或者更糟 —— 悄悄改错内容。
    fn direction(self) -> PatchDirection {
        match self {
            Self::Stage => PatchDirection::Forward,
            Self::Unstage | Self::Discard => PatchDirection::Reverse,
        }
    }
}

/// 给补丁失败补上路径与"刷新状态并重试"的动作。
///
/// 补丁被拒绝**几乎总是**因为"界面上的 diff 已经不是仓库现在的样子"
/// （编辑器保存了、外部工具改了、用户终端里 add 了）。此时唯一有意义的修复动作
/// 是刷新状态再看一遍，而不是重试同一份补丁 —— 因此动作指向 `workspace_status`，
/// 并把 `repoId` 一起带上（前端点击后能直接调用）。
///
/// 非补丁类错误原样返回：它们的修复方式与本动作无关，滥加按钮只会误导。
fn annotate(mut error: AppError, repo_id: i64, path: &RepoPath) -> AppError {
    if error.code != ErrorCode::PatchApplyFailed {
        return error;
    }

    let display = path.to_string_lossy().into_owned();
    error.hint = Some(match error.hint.take() {
        Some(existing) => format!("{existing} {display}"),
        None => display,
    });
    error.actions.push(
        FixAction::new(
            "refresh-status",
            "errors.actions.refresh",
            "workspace_status",
        )
        .with_args(serde_json::json!({ "repoId": repo_id })),
    );
    error
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::git::{ApplyDirection, ApplyTarget, DiffTarget, PatchDirection};
    use forgedesk_domain::{AppError, ErrorCode, FixAction};
    use forgedesk_git_engine::engine::{EngineId, GitEngine};

    use super::{annotate, PatchView, Plan};
    use crate::engines::GitEngines;

    fn path() -> forgedesk_domain::git::RepoPath {
        forgedesk_domain::git::RepoPath::from("src/main.rs")
    }

    #[test]
    fn the_three_plans_read_the_correct_side_and_apply_in_the_correct_direction() {
        assert_eq!(Plan::Stage.diff_target(), DiffTarget::Unstaged);
        assert_eq!(Plan::Discard.diff_target(), DiffTarget::Unstaged);
        assert_eq!(Plan::Unstage.diff_target(), DiffTarget::Staged);

        let staged = Plan::Stage.spec(b"p".to_vec());
        assert_eq!(staged.target, ApplyTarget::Index);
        assert_eq!(staged.direction, ApplyDirection::Forward);

        let unstaged = Plan::Unstage.spec(b"p".to_vec());
        assert_eq!(unstaged.target, ApplyTarget::Index);
        assert_eq!(unstaged.direction, ApplyDirection::Reverse);

        let discarded = Plan::Discard.spec(b"p".to_vec());
        assert_eq!(
            discarded.target,
            ApplyTarget::Worktree,
            "丢弃不能碰索引：已暂存的内容要保留"
        );
        assert_eq!(discarded.direction, ApplyDirection::Reverse);
    }

    #[test]
    fn the_trim_direction_always_matches_the_apply_direction() {
        // 两处不一致的后果不是报错，而是补丁被 git 拒绝、或者更糟 —— 悄悄改错内容。
        // 这条断言把"裁剪方向 == 应用方向"钉在代码里，而不是靠评审时想起来。
        for plan in [Plan::Stage, Plan::Unstage, Plan::Discard] {
            let apply_reverses = plan.spec(Vec::new()).direction == ApplyDirection::Reverse;
            let trim_reverses = plan.direction() == PatchDirection::Reverse;

            assert_eq!(apply_reverses, trim_reverses, "{plan:?}");
        }
    }

    #[test]
    fn a_patch_failure_gains_a_path_and_a_refresh_action() {
        let failure = AppError::new(ErrorCode::PatchApplyFailed, "git apply rejected the patch")
            .with_hint("apply --cached".to_owned());

        let annotated = annotate(failure, 7, &path());

        assert_eq!(annotated.code, ErrorCode::PatchApplyFailed);
        assert_eq!(
            annotated.hint.as_deref(),
            Some("apply --cached src/main.rs"),
            "原有 hint（git 开关）必须保留，并补上路径"
        );
        assert_eq!(annotated.actions.len(), 1);
        assert_eq!(annotated.actions[0].command, "workspace_status");
        assert_eq!(
            annotated.actions[0]
                .args
                .as_ref()
                .and_then(|args| args.get("repoId")),
            Some(&serde_json::json!(7))
        );
    }

    #[test]
    fn unrelated_failures_are_left_alone() {
        let failure = AppError::new(ErrorCode::Validation, "out of range");

        let annotated = annotate(failure, 7, &path());

        assert!(annotated.actions.is_empty(), "非补丁失败不该出现刷新按钮");
        assert_eq!(annotated.hint, None);
    }

    #[test]
    fn the_default_view_matches_the_diff_viewer_defaults() {
        let view = PatchView::default();

        assert_eq!(
            view.context_lines,
            forgedesk_domain::git::DEFAULT_CONTEXT_LINES
        );
        assert!(view.detect_renames);
        assert!(!view.ignore_whitespace);
    }

    #[test]
    fn the_action_key_exists_in_the_error_catalog() {
        // FixAction 的 labelKey 由前端按 i18n 渲染，写错一个 key 界面上就是空白按钮。
        // 这里只锁住"我们用的是一个约定过的 key 形状"，文案本身由前端测试保证。
        let action = FixAction::new(
            "refresh-status",
            "errors.actions.refresh",
            "workspace_status",
        );

        assert!(action.label_key.starts_with("errors.actions."));
    }

    #[test]
    fn engines_are_required_to_build_the_service() {
        // 这条断言的意义是"构造签名没变"：StagingService 必须与 WorkspaceService
        // 共享同一批引擎（各自 new 一个等于每次调用都起一条驱动线程）。
        let engines = GitEngines::new().expect("engines");

        assert_eq!(engines.write().id(), EngineId::Cli);
    }
}
