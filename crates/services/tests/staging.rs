//! 部分暂存的对拍测试（T1.6）。
//!
//! # 什么是对拍，以及为什么必须要它
//!
//! 被测实现是"取补丁 → 字节级裁剪 → `git apply`"；本文件的期望值由**另一条完全独立
//! 的实现**算出：直接用结构化 hunk 把选择套用到旧内容上，全程不碰补丁文本。
//! 两条路径没有共享代码，因此"同时出错且错得一样"的概率极低。
//!
//! 固定种子随机循环的意义在于组合空间：`多 hunk × 部分行 × 增删交错` 的合法组合
//! 远超人工能想出的例子数，而这里恰恰最容易写出"看起来对、实际把用户想保留的删除行
//! 一起吞掉"的实现（见 `domain::git::staging` 的模块头）。
//!
//! # 断言的落点是索引里的字节
//!
//! `git diff --cached --numstat` 的数字相同而内容不同是完全可能的（把一行替换成
//! 另一行、行内空白不同…）。所以除了数字，最后一公里必须断言 `git show :<path>`
//! 的**内容**：那才是用户下一步会提交出去的东西。
//!
//! # 覆盖的场景
//!
//! 随机的行级 / 块级暂存与取消暂存之外，本文件还固定覆盖：新增文件（含 `git add -N`
//! 的"意图添加"）、未跟踪文件被行级暂存、删除文件的部分暂存与整文件删除、带内容变更的
//! 重命名、CRLF + `core.autocrlf`、文件末尾无换行、二进制拒绝、下标越界、空选择幂等、
//! 以及**补丁失效时索引必须纹丝不动**。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use forgedesk_domain::git::{
    ApplyPatchSpec, DiffHunk, DiffLineKind, DiffSpec, DiffTarget, LineSelection, RepoId, RepoPath,
    StageScope,
};
use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_services::{
    GitEngines, OpenRepoRegistry, PatchView, RepositoryService, StagingService, WorkspaceService,
};
use forgedesk_storage::{Database, RepositoryStore};

mod support;

use support::{init_repo, memory_database, write, TempDir};

/// 随机用例数。任务要求 ≥ 200 组（暂存与取消暂存各 200 组）。
const RANDOM_CASES: usize = 200;

/// 被测文件的路径（所有用例共用）。
const FILE: &str = "base.txt";

/// 随机用例的基线行数：足够产生多 hunk，又不至于让 200 组用例变慢。
const BASE_LINES: usize = 18;

// ---------------------------------------------------------------- 夹具

struct Fixture {
    engines: GitEngines,
    database: Database,
    open: OpenRepoRegistry,
    dir: TempDir,
    repo_id: i64,
}

impl Fixture {
    /// 建仓库 + 提交一个初始文件。
    fn new(label: &str, initial: &[u8]) -> Self {
        let dir = TempDir::new(label);
        init_repo(dir.path());
        write(dir.path(), FILE, initial);
        support::commit_all(dir.path(), "base");

        let engines = GitEngines::new().expect("engines");
        let database = memory_database();
        let open = OpenRepoRegistry::default();
        let repository = RepositoryService::new(&engines, RepositoryStore::new(&database), &open);
        let opened = repository.open(dir.path()).expect("open");

        Self {
            engines,
            database,
            open,
            dir,
            repo_id: opened.record_id,
        }
    }

    /// 随机用例用的固定基线内容（每个用例只改工作区，基点不变，省下大量提交开销）。
    fn with_baseline(label: &str) -> Self {
        let lines: Vec<Vec<u8>> = (0..BASE_LINES)
            .map(|index| format!("base-{index:02}").into_bytes())
            .collect();
        Self::new(label, &join_lines(&lines, true))
    }

    fn dir(&self) -> &Path {
        self.dir.path()
    }

    fn repo(&self) -> RepoId {
        RepoId::new(self.dir.path().to_path_buf())
    }

    fn workspace(&self) -> WorkspaceService<'_> {
        WorkspaceService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            &self.open,
        )
    }

    fn staging(&self) -> StagingService<'_> {
        StagingService::new(self.workspace(), &self.engines)
    }

    /// 该文件在指定目标上的 hunk（与界面看到的是同一份）。
    fn hunks_of(&self, target: DiffTarget, path: &str) -> Vec<DiffHunk> {
        let report = self
            .workspace()
            .diff(
                self.repo_id,
                DiffSpec::new(target).with_paths(vec![RepoPath::from(path)]),
            )
            .expect("diff");
        report
            .files
            .into_iter()
            .find(|file| file.path.to_string() == path)
            .map(|file| file.hunks)
            .unwrap_or_default()
    }

    fn unstaged_hunks(&self, path: &str) -> Vec<DiffHunk> {
        self.hunks_of(DiffTarget::Unstaged, path)
    }

    fn staged_hunks(&self, path: &str) -> Vec<DiffHunk> {
        self.hunks_of(DiffTarget::Staged, path)
    }

    /// 索引里该文件的字节内容（`git show :<path>`）。
    fn index_content(&self, path: &str) -> Vec<u8> {
        git_bytes(self.dir(), &["show", &format!(":{path}")])
    }

    /// 索引里该文件是否存在。
    fn index_has(&self, path: &str) -> bool {
        support::git(self.dir(), &["show", &format!(":{path}")]).success()
    }

    /// HEAD 里该文件的字节内容。
    fn head_content(&self, path: &str) -> Vec<u8> {
        git_bytes(self.dir(), &["show", &format!("HEAD:{path}")])
    }

    /// `git diff [--cached] --numstat -- <path>` 的 (增加, 删除)。
    fn numstat(&self, cached: bool, path: &str) -> (u64, u64) {
        let mut args = vec!["diff", "--numstat"];
        if cached {
            args.push("--cached");
        }
        args.push("--");
        args.push(path);
        let raw = git_bytes(self.dir(), &args);
        let text = String::from_utf8_lossy(&raw);
        let mut fields = text.split_whitespace();
        match (fields.next(), fields.next()) {
            (Some(added), Some(removed)) => {
                (added.parse().unwrap_or(0), removed.parse().unwrap_or(0))
            }
            _ => (0, 0),
        }
    }

    /// 把工作区与索引恢复到 HEAD（用例之间互不影响）。
    fn reset(&self) {
        support::git_ok(self.dir(), &["reset", "--hard", "HEAD"]);
    }
}

fn git_bytes(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = support::git(dir, args);
    assert!(
        output.success(),
        "git {args:?} 失败（退出码 {:?}）：{}",
        output.exit_code,
        output.stderr_lossy()
    );
    output.stdout
}

// ---------------------------------------------------------------- 期望计算器（独立实现）

/// 一组选择：hunk 下标 → 该 hunk 内被选中的行位置。
type Selection = BTreeMap<usize, BTreeSet<usize>>;

/// 一个 hunk 里的"变更行位置"（added / removed）。
fn changeable_positions(hunk: &DiffHunk) -> Vec<usize> {
    hunk.lines
        .iter()
        .enumerate()
        .filter(|(_, line)| matches!(line.kind, DiffLineKind::Added | DiffLineKind::Removed))
        .map(|(position, _)| position)
        .collect()
}

/// 期望的索引内容：从 `base`（旧侧全文，逐行且不含行尾）出发，
/// 把 `applied` 里的变更行**应用**上去。
///
/// 这条路径完全不碰补丁文本 —— 它按 hunk 的 `old_start`/`old_lines` 走文件，
/// 因此与被测的"裁剪 + `git apply`"是两条独立实现。
///
/// - `added` 且 applied：这一行进入新内容；
/// - `removed` 且 applied：这一行从新内容里消失；
/// - 未 applied 的行保持原样（新增行不进、删除行留下）。
fn expected_lines(base: &[Vec<u8>], hunks: &[DiffHunk], applied: &Selection) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut cursor = 0_usize;

    for (hunk_index, hunk) in hunks.iter().enumerate() {
        let start = (hunk.old_start as usize).saturating_sub(1);
        // hunk 之间未变更的部分原样带过去
        while cursor < start && cursor < base.len() {
            out.push(base[cursor].clone());
            cursor += 1;
        }

        let selected = applied.get(&hunk_index);
        for (position, line) in hunk.lines.iter().enumerate() {
            let is_applied = selected.is_some_and(|set| set.contains(&position));
            match line.kind {
                DiffLineKind::Context => {
                    out.push(line.content.as_bytes().to_vec());
                }
                DiffLineKind::Added => {
                    if is_applied {
                        out.push(line.content.as_bytes().to_vec());
                    }
                }
                DiffLineKind::Removed => {
                    if !is_applied {
                        out.push(line.content.as_bytes().to_vec());
                    }
                }
                // 标记不占行数（两侧都没有末尾换行时才出现）
                DiffLineKind::NoNewlineMarker => {}
            }
        }

        // 与解析器同一口径：hunk 消费完后旧侧前进 old_lines 行
        // （上下文 + 删除行 = old_lines，所以这一个赋值就是全部推进）
        cursor = start + hunk.old_lines as usize;
    }

    while cursor < base.len() {
        out.push(base[cursor].clone());
        cursor += 1;
    }
    out
}

/// 把逐行内容拼回字节（`trailing_newline` 表示末尾是否有换行）。
fn join_lines(lines: &[Vec<u8>], trailing_newline: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push(b'\n');
        }
        out.extend_from_slice(line);
    }
    // 空内容就是空文件：此时"有没有末尾换行"没有意义
    if trailing_newline && !lines.is_empty() {
        out.push(b'\n');
    }
    out
}

/// 把字节内容切成"行"（丢掉末尾换行带来的空元素，同时记住它是否存在）。
fn split_lines(bytes: &[u8]) -> (Vec<Vec<u8>>, bool) {
    if bytes.is_empty() {
        return (Vec::new(), false);
    }
    let trailing_newline = bytes.last() == Some(&b'\n');
    let mut lines: Vec<Vec<u8>> = bytes
        .split(|byte| *byte == b'\n')
        .map(<[u8]>::to_vec)
        .collect();
    if trailing_newline {
        lines.pop();
    }
    (lines, trailing_newline)
}

/// 选择 → 行级 scope。
fn lines_scope(path: &str, selection: &Selection) -> StageScope {
    StageScope::lines(
        RepoPath::from(path),
        selection
            .iter()
            .map(|(hunk_index, lines)| LineSelection {
                hunk_index: *hunk_index,
                lines: lines.iter().copied().collect(),
            })
            .collect(),
    )
}

/// 选择 → 块级 scope（只保留有选中行的 hunk）。
fn hunks_scope(path: &str, selection: &Selection) -> StageScope {
    StageScope::hunks(
        RepoPath::from(path),
        selection.keys().copied().collect::<Vec<usize>>(),
    )
}

/// 选中整块时，"被应用"的就是该块的全部变更行。
fn applied_from_hunks(hunks: &[DiffHunk], selection: &Selection) -> Selection {
    selection
        .keys()
        .map(|hunk_index| {
            let positions: BTreeSet<usize> = changeable_positions(&hunks[*hunk_index])
                .into_iter()
                .collect();
            (*hunk_index, positions)
        })
        .collect()
}

/// 行级取消暂存：未选中的**行**留在索引里。
fn complement_lines(hunks: &[DiffHunk], selection: &Selection) -> Selection {
    let mut applied = Selection::new();
    for (hunk_index, hunk) in hunks.iter().enumerate() {
        let selected = selection.get(&hunk_index);
        let positions: BTreeSet<usize> = changeable_positions(hunk)
            .into_iter()
            .filter(|position| !selected.is_some_and(|set| set.contains(position)))
            .collect();
        if !positions.is_empty() {
            applied.insert(hunk_index, positions);
        }
    }
    applied
}

/// 块级取消暂存：未选中的**整块**留在索引里。
///
/// 与行级的区别是实质性的：块级下一个块的去留是整体的，不能按"这一块里哪些行
/// 被随机选中"来推 —— 那会让期望值算出"半个块留在索引里"这种无法表达的状态。
fn complement_hunks(hunks: &[DiffHunk], selection: &Selection) -> Selection {
    hunks
        .iter()
        .enumerate()
        .filter(|(hunk_index, _)| !selection.contains_key(hunk_index))
        .map(|(hunk_index, hunk)| (hunk_index, changeable_positions(hunk).into_iter().collect()))
        .collect()
}

// ---------------------------------------------------------------- 随机数据

/// 固定种子的线性同余生成器。
///
/// 刻意不引入 `proptest`：本任务需要的是"确定性、可复现、200 组"，
/// 而 LCG 十行就能做到，不必为它增加一个依赖与编译期成本。
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(
            seed.wrapping_mul(2_862_933_555_777_941_757)
                .wrapping_add(3_037_000_493),
        )
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 16
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next() % bound as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

/// 一行随机内容（纯 ASCII：本文件的对拍关心"行的位置与去留"，
/// 编码问题由 `domain::git::staging` 的字节级单测覆盖）。
fn random_line(rng: &mut Rng, index: usize) -> String {
    format!("line-{index:02}-{}", rng.below(1000))
}

/// 随机改动基线，保证结果与输入不同（否则 diff 为空，用例失去意义），
/// 且不会把文件删空（空文件是另一类边界，本循环不负责）。
fn mutate(rng: &mut Rng, base: &[String]) -> Vec<String> {
    let mut lines = base.to_vec();
    let edits = 1 + rng.below(4);
    for _ in 0..edits {
        match rng.below(3) {
            0 => {
                let index = rng.below(lines.len());
                lines[index] = random_line(rng, index);
            }
            1 => {
                let index = rng.below(lines.len() + 1);
                let value = random_line(rng, index);
                lines.insert(index, value);
            }
            _ => {
                if lines.len() > 2 {
                    let index = rng.below(lines.len());
                    lines.remove(index);
                } else {
                    let index = rng.below(lines.len());
                    lines[index] = random_line(rng, index);
                }
            }
        }
    }

    if lines == base {
        let replacement = random_line(rng, 0);
        lines[0] = replacement;
    }
    lines
}

/// 随机选择：对每个变更行独立抛硬币，并保证至少选中一行
/// （否则暂存是空操作，对拍会退化成"断言什么都没变"）。
fn random_selection(rng: &mut Rng, hunks: &[DiffHunk]) -> Selection {
    let mut selection = Selection::new();
    for (hunk_index, hunk) in hunks.iter().enumerate() {
        let mut positions = BTreeSet::new();
        for position in changeable_positions(hunk) {
            if rng.chance(55) {
                positions.insert(position);
            }
        }
        if !positions.is_empty() {
            selection.insert(hunk_index, positions);
        }
    }

    if selection.is_empty() {
        if let Some((hunk_index, position)) = hunks
            .iter()
            .enumerate()
            .find_map(|(index, hunk)| changeable_positions(hunk).first().map(|p| (index, *p)))
        {
            let mut positions = BTreeSet::new();
            positions.insert(position);
            selection.insert(hunk_index, positions);
        }
    }
    selection
}

/// 一组用例的工作区内容（基线 + 随机改动）。
fn worktree_bytes(lines: &[String]) -> Vec<u8> {
    join_lines(
        &lines
            .iter()
            .map(|line| line.as_bytes().to_vec())
            .collect::<Vec<_>>(),
        true,
    )
}

// ---------------------------------------------------------------- 对拍：暂存

/// 200 组随机：暂存选中的行 / 块，断言索引内容与独立计算器完全一致。
#[test]
fn randomly_selected_lines_and_hunks_stage_exactly_what_was_selected() {
    let fixture = Fixture::with_baseline("staging-random");
    let baseline: Vec<String> = (0..BASE_LINES)
        .map(|index| format!("base-{index:02}"))
        .collect();
    let (base_lines, trailing) = split_lines(&fixture.head_content(FILE));

    for case in 0..RANDOM_CASES {
        fixture.reset();
        let mut rng = Rng::new(0x5EED_0000 + case as u64);

        let mutated = mutate(&mut rng, &baseline);
        write(fixture.dir(), FILE, &worktree_bytes(&mutated));

        let hunks = fixture.unstaged_hunks(FILE);
        assert!(!hunks.is_empty(), "case {case}: 改动应当产生 hunk");
        let selection = random_selection(&mut rng, &hunks);
        let by_hunk = case % 2 == 0;

        let (scope, applied) = if by_hunk {
            (
                hunks_scope(FILE, &selection),
                applied_from_hunks(&hunks, &selection),
            )
        } else {
            (lines_scope(FILE, &selection), selection.clone())
        };
        let expected = join_lines(&expected_lines(&base_lines, &hunks, &applied), trailing);
        let (expected_added, expected_removed) = applied_totals(&hunks, &applied);

        fixture
            .staging()
            .stage(fixture.repo_id, &scope, PatchView::default())
            .unwrap_or_else(|error| panic!("case {case} 暂存失败：{error:?}"));

        assert_eq!(
            fixture.numstat(true, FILE),
            (expected_added, expected_removed),
            "case {case} 已暂存的增删行数与选择不符"
        );
        assert_eq!(
            fixture.index_content(FILE),
            expected,
            "case {case} 暂存后的索引内容不符"
        );
    }
}

/// 200 组随机：取消暂存选中的行 / 块，断言索引内容与独立计算器完全一致。
#[test]
fn randomly_selected_lines_and_hunks_unstage_exactly_what_was_selected() {
    let fixture = Fixture::with_baseline("unstaging-random");
    let baseline: Vec<String> = (0..BASE_LINES)
        .map(|index| format!("base-{index:02}"))
        .collect();
    let (head_lines, trailing) = split_lines(&fixture.head_content(FILE));

    for case in 0..RANDOM_CASES {
        fixture.reset();
        let mut rng = Rng::new(0xBEEF_0000 + case as u64);

        let mutated = mutate(&mut rng, &baseline);
        write(fixture.dir(), FILE, &worktree_bytes(&mutated));
        // 全部改动先进入索引：此时"已暂存"侧就是完整的新内容
        support::git_ok(fixture.dir(), &["add", "--", FILE]);

        let hunks = fixture.staged_hunks(FILE);
        assert!(!hunks.is_empty(), "case {case}: 已暂存侧应当有 hunk");
        let selection = random_selection(&mut rng, &hunks);
        let by_hunk = case % 2 == 0;

        // 取消暂存 = 把选中的改动从索引里撤掉 → 留在索引里的是"未选中"的那些
        // （块级按整块判定，行级按行判定，两者的期望值本来就不同）
        let (scope, staying) = if by_hunk {
            (
                hunks_scope(FILE, &selection),
                complement_hunks(&hunks, &selection),
            )
        } else {
            (
                lines_scope(FILE, &selection),
                complement_lines(&hunks, &selection),
            )
        };
        let expected = join_lines(&expected_lines(&head_lines, &hunks, &staying), trailing);
        fixture
            .staging()
            .unstage(fixture.repo_id, &scope, PatchView::default())
            .unwrap_or_else(|error| panic!("case {case} 取消暂存失败：{error:?}"));

        assert_eq!(
            fixture.index_content(FILE),
            expected,
            "case {case} 取消暂存后的索引内容不符"
        );
    }
}

/// `applied` 集合对应的增删行数。
fn applied_totals(hunks: &[DiffHunk], applied: &Selection) -> (u64, u64) {
    let mut added = 0_u64;
    let mut removed = 0_u64;
    for (hunk_index, hunk) in hunks.iter().enumerate() {
        let selected = applied.get(&hunk_index);
        for (position, line) in hunk.lines.iter().enumerate() {
            if !selected.is_some_and(|set| set.contains(&position)) {
                continue;
            }
            match line.kind {
                DiffLineKind::Added => added += 1,
                DiffLineKind::Removed => removed += 1,
                _ => {}
            }
        }
    }
    (added, removed)
}

// ---------------------------------------------------------------- 定向场景

#[test]
fn an_intent_to_add_file_can_be_staged_partially() {
    let fixture = Fixture::new("staging-new-file", b"base\n");
    // `git add -N` 让未跟踪文件获得工作区 diff（用户在终端里很常见的动作）
    write(fixture.dir(), "fresh.txt", b"one\ntwo\nthree\n");
    support::git_ok(fixture.dir(), &["add", "-N", "--", "fresh.txt"]);

    let hunks = fixture.unstaged_hunks("fresh.txt");
    assert_eq!(hunks.len(), 1, "全新文件应当是一个 hunk");

    // 只暂存第 1 行与第 3 行
    let mut positions = BTreeSet::new();
    positions.insert(0_usize);
    positions.insert(2_usize);
    let mut selection = Selection::new();
    selection.insert(0_usize, positions);

    fixture
        .staging()
        .stage(
            fixture.repo_id,
            &lines_scope("fresh.txt", &selection),
            PatchView::default(),
        )
        .expect("新增文件的部分暂存");

    assert_eq!(fixture.index_content("fresh.txt"), b"one\nthree\n");
}

#[test]
fn a_fully_untracked_file_has_no_patch_and_is_rejected_clearly() {
    let fixture = Fixture::new("staging-untracked", b"base\n");
    write(fixture.dir(), "ghost.txt", b"alpha\nbeta\n");

    let mut positions = BTreeSet::new();
    positions.insert(0_usize);
    let mut selection = Selection::new();
    selection.insert(0_usize, positions);

    let error = fixture
        .staging()
        .stage(
            fixture.repo_id,
            &lines_scope("ghost.txt", &selection),
            PatchView::default(),
        )
        .expect_err("未跟踪文件没有补丁，不该被静默跳过");

    assert_eq!(error.code, ErrorCode::Validation);
    assert_eq!(error.hint.as_deref(), Some("ghost.txt"));
}

#[test]
fn partially_deleting_a_file_keeps_it_in_the_index_with_the_rest_of_its_lines() {
    let fixture = Fixture::new("staging-delete", b"a\nb\nc\nd\n");
    std::fs::remove_file(fixture.dir().join(FILE)).expect("删除文件");

    let hunks = fixture.unstaged_hunks(FILE);
    assert_eq!(hunks.len(), 1);

    // 只确认前两行被删除：文件必须留在索引里（只是少了前两行）
    let mut positions = BTreeSet::new();
    positions.insert(0_usize);
    positions.insert(1_usize);
    let mut selection = Selection::new();
    selection.insert(0_usize, positions);

    fixture
        .staging()
        .stage(
            fixture.repo_id,
            &lines_scope(FILE, &selection),
            PatchView::default(),
        )
        .expect("删除文件的部分暂存");

    assert!(fixture.index_has(FILE), "文件不该从索引里消失");
    assert_eq!(fixture.index_content(FILE), b"c\nd\n");
}

#[test]
fn deleting_every_line_still_removes_the_entry_from_the_index() {
    let fixture = Fixture::new("staging-delete-all", b"a\nb\n");
    std::fs::remove_file(fixture.dir().join(FILE)).expect("删除文件");

    let hunks = fixture.unstaged_hunks(FILE);
    let selection: Selection = hunks
        .iter()
        .enumerate()
        .map(|(index, hunk)| (index, changeable_positions(hunk).into_iter().collect()))
        .collect();

    fixture
        .staging()
        .stage(
            fixture.repo_id,
            &lines_scope(FILE, &selection),
            PatchView::default(),
        )
        .expect("整文件删除");

    assert!(!fixture.index_has(FILE), "整文件删除后索引里不该还有该条目");
}

#[test]
fn a_rename_with_content_changes_keeps_the_rename_when_partially_staged() {
    let fixture = Fixture::new("staging-rename", b"first\nsecond\nthird\n");
    support::git_ok(fixture.dir(), &["mv", FILE, "renamed.txt"]);
    write(fixture.dir(), "renamed.txt", b"first\nSECOND\nthird\n");

    let hunks = fixture.unstaged_hunks("renamed.txt");
    assert_eq!(hunks.len(), 1, "重命名 + 内容变更应当只有一个文件段");

    // 只暂存内容变更中的一部分（重命名是文件级属性，会一起生效）
    let mut positions = BTreeSet::new();
    positions.insert(0_usize);
    let mut selection = Selection::new();
    selection.insert(0_usize, positions);

    fixture
        .staging()
        .stage(
            fixture.repo_id,
            &lines_scope("renamed.txt", &selection),
            PatchView::default(),
        )
        .expect("重命名文件的部分暂存");

    let listed = String::from_utf8_lossy(&git_bytes(fixture.dir(), &["ls-files"])).into_owned();
    assert!(listed.contains("renamed.txt"), "索引里应有新路径：{listed}");
    assert!(!listed.contains(FILE), "索引里不该还有旧路径：{listed}");
}

#[test]
fn crlf_worktree_content_is_staged_without_line_ending_conversion() {
    let fixture = Fixture::new("staging-crlf", b"one\ntwo\nthree\n");
    // 打开 autocrlf 后，工作区的 CRLF 会被规范化成 LF 再与索引比较
    support::git_ok(fixture.dir(), &["config", "core.autocrlf", "true"]);
    write(fixture.dir(), FILE, b"one\r\nTWO\r\nthree\r\n");

    let hunks = fixture.unstaged_hunks(FILE);
    assert_eq!(hunks.len(), 1, "换行风格差异不应该被算成多出来的 hunk");

    // 整块暂存（CRLF 的验证点是"补丁不携带回车"，与选中哪些行无关）
    fixture
        .staging()
        .stage(
            fixture.repo_id,
            &StageScope::hunks(RepoPath::from(FILE), vec![0]),
            PatchView::default(),
        )
        .expect("CRLF 工作区的整块暂存");

    let content = fixture.index_content(FILE);
    assert_eq!(
        content, b"one\nTWO\nthree\n",
        "索引里永远是规范化后的 LF（blob 语义）"
    );
}

#[test]
fn a_file_without_a_trailing_newline_keeps_that_state_after_partial_staging() {
    let fixture = Fixture::new("staging-no-eol", b"alpha\nbeta\ngamma");
    write(fixture.dir(), FILE, b"alpha\nBETA\ngamma");

    let hunks = fixture.unstaged_hunks(FILE);
    assert_eq!(hunks.len(), 1);

    // 只暂存被改写的那一行（`+BETA` 是 hunk 里的第 3 行；它前面的删除行转成上下文，
    // 末尾的无换行状态必须原样保留）
    let mut positions = BTreeSet::new();
    positions.insert(2_usize);
    let mut selection = Selection::new();
    selection.insert(0_usize, positions);

    fixture
        .staging()
        .stage(
            fixture.repo_id,
            &lines_scope(FILE, &selection),
            PatchView::default(),
        )
        .expect("无末尾换行文件的部分暂存");

    assert_eq!(
        fixture.index_content(FILE),
        b"alpha\nbeta\nBETA\ngamma",
        "末尾不该凭空多出或丢掉换行"
    );
}

#[test]
fn binary_files_are_rejected_for_line_level_staging() {
    let fixture = Fixture::new("staging-binary", &[0u8, 159, 146, 150, 0, 1, 2, 3]);
    write(fixture.dir(), FILE, &[0u8, 1, 2, 3, 4, 5, 6, 7]);

    let hunks = fixture.unstaged_hunks(FILE);
    assert!(hunks.is_empty(), "二进制文件不该有 hunk");

    let scope = StageScope::hunks(RepoPath::from(FILE), vec![0]);
    let error = fixture
        .staging()
        .stage(fixture.repo_id, &scope, PatchView::default())
        .expect_err("二进制文件不支持行级");

    assert_eq!(error.code, ErrorCode::Validation);
}

#[test]
fn an_out_of_range_selection_is_rejected_without_touching_the_index() {
    let fixture = Fixture::new("staging-range", b"one\ntwo\n");
    write(fixture.dir(), FILE, b"ONE\ntwo\n");
    let before = fixture.index_content(FILE);

    let scope = StageScope::hunks(RepoPath::from(FILE), vec![9]);
    let error = fixture
        .staging()
        .stage(fixture.repo_id, &scope, PatchView::default())
        .expect_err("越界的 hunk 下标必须被拒绝");

    assert_eq!(error.code, ErrorCode::Validation);
    assert_eq!(
        fixture.index_content(FILE),
        before,
        "失败时索引必须纹丝不动"
    );
}

#[test]
fn an_empty_selection_is_idempotent() {
    let fixture = Fixture::new("staging-empty", b"one\ntwo\n");
    write(fixture.dir(), FILE, b"ONE\ntwo\n");
    let before = fixture.index_content(FILE);

    fixture
        .staging()
        .stage(
            fixture.repo_id,
            &StageScope::hunks(RepoPath::from(FILE), Vec::new()),
            PatchView::default(),
        )
        .expect("空选择不是错误");

    assert_eq!(fixture.index_content(FILE), before);
}

#[test]
fn a_stale_patch_fails_the_dry_run_and_leaves_the_index_untouched() {
    let fixture = Fixture::new("staging-stale", b"one\ntwo\nthree\n");

    // 用服务的一条真实链路拿到补丁，再让索引在"应用之前"发生变化
    write(fixture.dir(), FILE, b"one\nTWO\nthree\n");
    support::git_ok(fixture.dir(), &["add", "--", FILE]);
    let patch = fixture
        .workspace()
        .diff_patch(
            fixture.repo_id,
            DiffSpec::new(DiffTarget::Staged).with_paths(vec![RepoPath::from(FILE)]),
        )
        .expect("staged patch");

    // 外部改动：把工作区再改一版并 add 进索引，旧补丁于是描述了一个已经不存在的索引
    write(fixture.dir(), FILE, b"one\nTHIRD\nthree\n");
    support::git_ok(fixture.dir(), &["add", "--", FILE]);
    let before = fixture.index_content(FILE);

    let spec = ApplyPatchSpec::stage(patch);
    let error = fixture
        .engines
        .write()
        .apply_patch(&fixture.repo(), &spec.clone().checked())
        .expect_err("过期的补丁必须在 check 阶段就被拦下");

    assert_eq!(error.code, ErrorCode::PatchApplyFailed);
    assert!(error.detail.is_some(), "必须带上原始 stderr 供排查");
    assert_eq!(
        fixture.index_content(FILE),
        before,
        "dry-run 失败时索引必须原样"
    );
}

#[test]
fn a_rejected_patch_reports_the_raw_stderr_and_is_retryable() {
    let fixture = Fixture::new("staging-rejected", b"one\ntwo\n");
    let bogus = b"diff --git a/base.txt b/base.txt\n--- a/base.txt\n+++ b/base.txt\n@@ -1,1 +1,1 @@\n-nonexistent line\n+something else\n".to_vec();

    let error = fixture
        .engines
        .write()
        .apply_patch(&fixture.repo(), &ApplyPatchSpec::stage(bogus))
        .expect_err("无法应用的补丁必须失败");

    assert_eq!(error.code, ErrorCode::PatchApplyFailed);
    assert!(
        error.detail.as_deref().is_some_and(|text| !text.is_empty()),
        "detail 里必须是 git 的原始解释"
    );
    assert!(error.retryable, "过期是可以通过刷新解决的，应当可重试");
}

#[test]
fn staging_never_touches_the_worktree() {
    let fixture = Fixture::new("staging-isolation", b"one\ntwo\nthree\n");
    let worktree = b"one\nTWO\nthree\n".to_vec();
    write(fixture.dir(), FILE, &worktree);

    let hunks = fixture.unstaged_hunks(FILE);
    let selection = random_selection(&mut Rng::new(17), &hunks);
    fixture
        .staging()
        .stage(
            fixture.repo_id,
            &lines_scope(FILE, &selection),
            PatchView::default(),
        )
        .expect("部分暂存");

    assert_eq!(
        std::fs::read(fixture.dir().join(FILE)).expect("读工作区"),
        worktree,
        "部分暂存只能改索引，工作区必须原样"
    );
}

#[test]
fn partial_discard_restores_only_the_selected_lines() {
    let fixture = Fixture::new("staging-discard", b"one\ntwo\nthree\n");
    write(fixture.dir(), FILE, b"one\nTWO\nthree\n");

    let before_index = fixture.index_content(FILE);
    // 补丁体是：` one`(0)、`-two`(1)、`+TWO`(2)、` three`(3)。
    // 只选中 `+TWO` 丢弃 → 撤销"把 TWO 加进工作区"这个改动；未选中的 `-two`
    // 意味着"two 的删除不动" → 工作区最终是 `one\three`（部分丢弃的语义就是如此）。
    let mut positions = BTreeSet::new();
    positions.insert(2_usize);
    let mut selection = Selection::new();
    selection.insert(0_usize, positions);

    fixture
        .staging()
        .discard(
            fixture.repo_id,
            &lines_scope(FILE, &selection),
            PatchView::default(),
        )
        .expect("按行丢弃");

    assert_eq!(
        fixture.index_content(FILE),
        before_index,
        "丢弃不能改索引（已暂存的内容要保留）"
    );
    assert_eq!(
        std::fs::read(fixture.dir().join(FILE)).expect("读工作区"),
        b"one\nthree\n",
        "只有被选中的那一行被撤销，未选中的改动必须保留"
    );
}

#[test]
fn a_different_context_setting_shifts_hunk_indices_and_is_caught() {
    let fixture = Fixture::new("staging-context", b"one\ntwo\nthree\nfour\nfive\n");
    write(fixture.dir(), FILE, b"one\nTWO\nthree\nFOUR\nfive\n");

    // 用 -U0 渲染（两个独立的小块），却用默认 -U3 去暂存第 2 块：
    // 后端按调用方给的查看参数重新生成补丁，下标因此对不上 —— 必须被拦下，
    // 而不是"照着错位的下标暂存出别的内容"。
    let narrow = fixture
        .workspace()
        .diff(
            fixture.repo_id,
            DiffSpec::new(DiffTarget::Unstaged)
                .with_paths(vec![RepoPath::from(FILE)])
                .with_context_lines(0),
        )
        .expect("diff with -U0");
    let narrow_hunks = &narrow.files[0].hunks;
    assert_eq!(narrow_hunks.len(), 2, "两处相隔较远的改动在 -U0 下是两个块");

    let index_content_before = fixture.index_content(FILE);
    let scope = StageScope::hunks(RepoPath::from(FILE), vec![1]);
    let error = fixture
        .staging()
        .stage(fixture.repo_id, &scope, PatchView::default())
        .expect_err("错位的下标必须被拦下，而不是暂存出别的内容");

    assert_eq!(error.code, ErrorCode::Validation);
    assert_eq!(
        fixture.index_content(FILE),
        index_content_before,
        "被拦下时索引必须原样"
    );
}
