//! 解析器的表驱动测试：真实 git 输出样本 + 随机字节鲁棒性。
//!
//! # fixtures 是怎么来的
//!
//! `tests/fixtures/` 下的每个文件都是**真实 git（2.54.0.windows.1）的输出**，
//! 由临时脚本在系统临时目录里建仓库后原样重定向得到（不经过字符串、不经过编辑器，
//! 因此 NUL、`%x1f`、非 UTF-8 字节都是真的）。提交前设置
//! `GIT_AUTHOR_DATE`/`GIT_COMMITTER_DATE` 与作者信息，因此 oid 是稳定的。
//!
//! | 文件 | 生成命令 |
//! | --- | --- |
//! | `status_v2_initial_no_commit.txt` | `git status --porcelain=v2 -z --branch`（空仓库） |
//! | `status_v2_clean_with_upstream.txt` | 同上（有上游，ahead 1 / behind 1） |
//! | `status_v2_detached_head.txt` | 同上（`checkout --detach`） |
//! | `status_v2_mixed_worktree.txt` | 同上（`A.`、`.D`、`MM`、`?`） |
//! | `status_v2_rename.txt` | 同上（`2 R.` 双路径） |
//! | `status_v2_copy.txt` | 同上（`-c status.renames=copies`，`2 C.`） |
//! | `status_v2_unmerged.txt` | 同上（合并冲突，`u UU`） |
//! | `status_v2_submodule_dirty.txt` | 同上（子模块脏，`S.MU`） |
//! | `status_v2_ignored.txt` | 同上（`--ignored`，`!`） |
//! | `status_v2_paths_special.txt` | 同上（空格 + 中文路径） |
//! | `diff_numstat_empty.txt` | `git diff --numstat -z`（干净工作区） |
//! | `diff_numstat_basic.txt` | 同上（修改 + 删除） |
//! | `diff_numstat_binary.txt` | `git diff --cached --numstat -z`（文本 + 二进制） |
//! | `diff_numstat_rename.txt` | 同上 `-M`（重命名 + 一行修改） |
//! | `diff_numstat_copy.txt` | 同上 `-C --find-copies-harder`（修改 + 复制） |
//! | `diff_numstat_utf8_path.txt` | 同上（中文路径） |
//! | `log_format_linear_z.txt` | `git log --format=<LOG_FORMAT> -z`（3 个线性提交） |
//! | `log_format_linear_no_z.txt` | 同上，不带 `-z`（验证换行结束符） |
//! | `log_format_root_only.txt` | 同上（单个根提交） |
//! | `log_format_merge.txt` | 同上（`--no-ff` 合并，2 个父提交） |
//! | `log_format_refs_and_utf8_subject.txt` | 同上（轻量 + 附注 tag、中文 subject） |
//! | `ls_files_stage_empty.txt` | `git ls-files -u -z`（无冲突） |
//! | `ls_files_stage_unmerged.txt` | 同上（`UU` 三 stage + `DU` 两 stage） |
//!
//! 集成测试是独立 crate，测试代码里允许 panic 式写法（失败即断言失败）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use forgedesk_domain::git::{ChangeKind, EntryKind, SignatureStatus, UnmergedStage};
use forgedesk_git_engine::parsers::{
    parse_diff_numstat, parse_log_format, parse_ls_files_stage, parse_status_porcelain_v2,
};

/// 读取一个 fixture 的原始字节。
fn fixture(name: &str) -> Vec<u8> {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    fs::read(&path).unwrap_or_else(|error| panic!("读取 fixture {name} 失败: {error}"))
}

// ---------------------------------------------------------------- status

#[test]
fn status_initial_repository_has_no_oid_and_is_not_detached() {
    let report = parse_status_porcelain_v2(&fixture("status_v2_initial_no_commit.txt"));

    assert!(report.is_clean());
    assert!(report.branch.is_initial());
    assert_eq!(report.branch.oid, None);
    assert_eq!(report.branch.head.as_deref(), Some("main"));
    assert!(!report.branch.detached);
}

#[test]
fn status_with_upstream_reports_ahead_and_behind_counts() {
    let report = parse_status_porcelain_v2(&fixture("status_v2_clean_with_upstream.txt"));

    assert!(report.is_clean());
    assert_eq!(
        report.branch.oid.as_deref(),
        Some("09d24e2dade648c3c0b7a647a795a27a8ecf7abe")
    );
    assert_eq!(report.branch.head.as_deref(), Some("main"));
    assert_eq!(report.branch.upstream.as_deref(), Some("origin/main"));
    assert_eq!(report.branch.ahead, Some(1));
    assert_eq!(report.branch.behind, Some(1));
    assert!(!report.branch.is_initial());
}

#[test]
fn status_detached_head_has_an_oid_but_no_branch_name() {
    let report = parse_status_porcelain_v2(&fixture("status_v2_detached_head.txt"));

    assert!(report.branch.detached);
    assert_eq!(report.branch.head, None);
    assert!(report.branch.oid.is_some());
    // 游离 HEAD 不是"初始仓库"：两者都没有分支名，但引导完全不同
    assert!(!report.branch.is_initial());
}

#[test]
fn status_mixed_worktree_covers_staged_worktree_and_untracked_entries() {
    let report = parse_status_porcelain_v2(&fixture("status_v2_mixed_worktree.txt"));

    assert_eq!(report.entries.len(), 4);
    assert!(!report.is_clean());

    let by_path = |path: &str| {
        report
            .entries
            .iter()
            .find(|entry| entry.path.to_string() == path)
            .unwrap_or_else(|| panic!("缺少条目 {path}"))
    };

    let added = by_path("added.txt");
    assert_eq!(added.kind, EntryKind::Ordinary);
    assert_eq!(added.index_status, ChangeKind::Added);
    assert_eq!(added.worktree_status, ChangeKind::Unmodified);
    // 新增文件的 HEAD 侧模式与 oid 都是零 → None，而不是 0
    assert_eq!(added.mode_head, None);
    assert_eq!(added.oid_head, None);
    assert_eq!(added.mode_index, Some(0o100644));

    let deleted = by_path("gone.txt");
    assert_eq!(deleted.worktree_status, ChangeKind::Deleted);
    assert_eq!(deleted.mode_worktree, None);

    let modified = by_path("keep.txt");
    assert_eq!(modified.index_status, ChangeKind::Modified);
    assert_eq!(modified.worktree_status, ChangeKind::Modified);
    assert_ne!(modified.oid_head, modified.oid_index);

    let untracked = by_path("untracked.txt");
    assert_eq!(untracked.kind, EntryKind::Untracked);
    assert!(untracked.is_untracked_or_ignored());
    assert_eq!(untracked.mode_index, None);
}

#[test]
fn status_rename_record_keeps_both_paths_and_similarity() {
    let report = parse_status_porcelain_v2(&fixture("status_v2_rename.txt"));

    assert_eq!(report.entries.len(), 1);
    let entry = &report.entries[0];
    assert_eq!(entry.kind, EntryKind::RenamedOrCopied);
    assert_eq!(entry.path.to_string(), "renamed.txt");
    assert_eq!(
        entry.original_path.as_ref().map(|path| path.to_string()),
        Some("a.txt".to_owned())
    );
    assert_eq!(entry.index_status, ChangeKind::Renamed);
    assert_eq!(entry.similarity, Some(100));
}

#[test]
fn status_copy_record_is_parsed_with_its_source() {
    let report = parse_status_porcelain_v2(&fixture("status_v2_copy.txt"));

    assert_eq!(report.entries.len(), 2);
    let copy = report
        .entries
        .iter()
        .find(|entry| entry.kind == EntryKind::RenamedOrCopied)
        .expect("缺少复制条目");
    assert_eq!(copy.path.to_string(), "copy.txt");
    assert_eq!(
        copy.original_path.as_ref().map(|path| path.to_string()),
        Some("a.txt".to_owned())
    );
    assert_eq!(copy.index_status, ChangeKind::Copied);
    assert_eq!(copy.similarity, Some(100));
}

#[test]
fn status_unmerged_record_exposes_three_stages_and_conflicts_iterator() {
    let report = parse_status_porcelain_v2(&fixture("status_v2_unmerged.txt"));

    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.conflicts().count(), 1);

    let entry = &report.entries[0];
    assert!(entry.is_conflicted());
    assert_eq!(entry.index_status, ChangeKind::Unmerged);
    assert_eq!(entry.worktree_status, ChangeKind::Unmerged);

    let stages = entry.stages.as_ref().expect("冲突条目必须带三个 stage");
    assert_eq!(stages.base.as_ref().unwrap().mode, 0o100644);
    assert_eq!(
        stages.ours.as_ref().unwrap().oid,
        "ba2906d0666cf726c7eaadd2cd3db615dedfdf3a"
    );
    assert_eq!(
        stages.theirs.as_ref().unwrap().oid,
        "e45c9c2666d44e0327c1f9c239a74c508336053e"
    );
    // 冲突条目的 HEAD 侧没有意义，不应伪造
    assert_eq!(entry.oid_head, None);
    assert_eq!(entry.mode_worktree, Some(0o100644));
}

#[test]
fn status_submodule_entry_carries_its_flags_and_gitlink_mode() {
    let report = parse_status_porcelain_v2(&fixture("status_v2_submodule_dirty.txt"));

    assert_eq!(report.entries.len(), 1);
    let entry = &report.entries[0];
    assert_eq!(entry.path.to_string(), "vendor/sub");
    // gitlink 模式是 160000，不是 100644
    assert_eq!(entry.mode_head, Some(0o160000));
    assert!(entry.submodule.is_submodule);
    assert!(entry.submodule.modified_content);
    assert!(entry.submodule.untracked_content);
    assert!(!entry.submodule.commit_changed);
}

#[test]
fn status_ignored_entries_are_parsed_separately_from_untracked() {
    let report = parse_status_porcelain_v2(&fixture("status_v2_ignored.txt"));

    assert_eq!(report.entries.len(), 2);
    assert!(report
        .entries
        .iter()
        .all(|entry| entry.kind == EntryKind::Ignored));
    let paths: Vec<String> = report
        .entries
        .iter()
        .map(|entry| entry.path.to_string())
        .collect();
    assert!(paths.contains(&"build/".to_owned()));
    assert!(paths.contains(&"ignored.txt".to_owned()));
}

#[test]
fn status_paths_with_spaces_and_utf8_survive_the_round_trip() {
    let report = parse_status_porcelain_v2(&fixture("status_v2_paths_special.txt"));

    assert_eq!(report.entries.len(), 2);
    let paths: Vec<String> = report
        .entries
        .iter()
        .map(|entry| entry.path.to_string())
        .collect();
    assert!(paths.contains(&"a b.txt".to_owned()));
    assert!(paths.contains(&"中文 目录/文件.txt".to_owned()));
    assert!(report.entries.iter().all(|entry| entry.path.is_utf8()));
}

// ---------------------------------------------------------------- numstat

#[test]
fn numstat_empty_output_yields_no_stats() {
    assert!(parse_diff_numstat(&fixture("diff_numstat_empty.txt")).is_empty());
}

#[test]
fn numstat_basic_output_reports_line_counts() {
    let stats = parse_diff_numstat(&fixture("diff_numstat_basic.txt"));

    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].path.to_string(), "a.txt");
    assert_eq!(stats[0].additions, Some(2));
    assert_eq!(stats[0].deletions, Some(1));
    assert!(!stats[0].binary);
    assert_eq!(stats[1].path.to_string(), "b.txt");
    assert_eq!(stats[1].additions, Some(0));
    assert_eq!(stats[1].deletions, Some(1));
}

#[test]
fn numstat_binary_output_is_flagged_without_fake_counts() {
    let stats = parse_diff_numstat(&fixture("diff_numstat_binary.txt"));

    assert_eq!(stats.len(), 2);
    let binary = stats
        .iter()
        .find(|stat| stat.binary)
        .expect("缺少二进制条目");
    assert_eq!(binary.path.to_string(), "bin.dat");
    assert_eq!(binary.additions, None);
    assert_eq!(binary.changed_lines(), None);
    // 同一份输出里的文本条目不应被误标为二进制
    assert!(stats
        .iter()
        .any(|stat| !stat.binary && stat.additions == Some(1)));
}

#[test]
fn numstat_rename_output_pairs_source_and_target_paths() {
    let stats = parse_diff_numstat(&fixture("diff_numstat_rename.txt"));

    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].path.to_string(), "new name.txt");
    assert_eq!(
        stats[0].original_path.as_ref().map(|path| path.to_string()),
        Some("old name.txt".to_owned())
    );
    assert!(stats[0].is_rename_or_copy());
    assert_eq!(stats[0].additions, Some(1));
}

#[test]
fn numstat_copy_output_is_parsed_alongside_the_source_modification() {
    let stats = parse_diff_numstat(&fixture("diff_numstat_copy.txt"));

    assert_eq!(stats.len(), 2);
    let copy = stats
        .iter()
        .find(|stat| stat.is_rename_or_copy())
        .expect("缺少复制条目");
    assert_eq!(copy.path.to_string(), "copy.txt");
    assert_eq!(
        copy.original_path.as_ref().map(|path| path.to_string()),
        Some("a.txt".to_owned())
    );
    // 纯复制是 0 0，不是二进制
    assert_eq!(copy.changed_lines(), Some(0));
    assert!(!copy.binary);
}

#[test]
fn numstat_utf8_paths_are_read_as_utf8() {
    let stats = parse_diff_numstat(&fixture("diff_numstat_utf8_path.txt"));

    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].path.to_string(), "中文 目录/文件.txt");
    assert!(stats[0].path.is_utf8());
}

// ---------------------------------------------------------------- log

#[test]
fn log_linear_history_is_parsed_in_git_order_with_parent_links() {
    let commits = parse_log_format(&fixture("log_format_linear_z.txt"));

    assert_eq!(commits.len(), 3);
    assert_eq!(commits[0].subject, "third commit");
    assert_eq!(commits[1].subject, "second commit");
    assert_eq!(commits[2].subject, "first commit");

    // 顺序是"新 → 旧"（与 git log 一致），父提交指向前一条
    assert_eq!(commits[0].parents, vec![commits[1].oid.clone()]);
    assert_eq!(commits[1].parents, vec![commits[2].oid.clone()]);
    assert!(commits[2].is_root());

    // 只有最新提交带 ref
    assert_eq!(commits[0].refs, vec!["HEAD -> main".to_owned()]);
    assert!(commits[1].refs.is_empty());

    assert_eq!(commits[0].author.name, "Fixture Author");
    assert_eq!(commits[0].author.email, "author@example.com");
    assert_eq!(commits[0].author.time, Some(1_704_164_645));
    assert_eq!(commits[0].signature, SignatureStatus::Unsigned);
    assert_eq!(commits[0].body, None, "列表查询不取正文");
}

#[test]
fn log_without_z_terminator_produces_the_same_commits() {
    let with_z = parse_log_format(&fixture("log_format_linear_z.txt"));
    let without_z = parse_log_format(&fixture("log_format_linear_no_z.txt"));

    assert_eq!(with_z, without_z);
}

#[test]
fn log_root_only_history_has_an_empty_parent_list() {
    let commits = parse_log_format(&fixture("log_format_root_only.txt"));

    assert_eq!(commits.len(), 1);
    assert!(commits[0].is_root());
    assert!(!commits[0].is_merge());
    assert_eq!(commits[0].subject, "root commit");
}

#[test]
fn log_merge_commit_keeps_both_parents_in_order() {
    let commits = parse_log_format(&fixture("log_format_merge.txt"));

    assert_eq!(commits.len(), 4);
    let merge = &commits[0];
    assert_eq!(merge.subject, "merge feature");
    assert!(merge.is_merge());
    assert_eq!(merge.parents.len(), 2);
    // first-parent 是合并时的当前分支（main）
    assert_eq!(merge.parents[0], commits[1].oid);
    assert_eq!(merge.parents[1], commits[2].oid);

    // 被合并的分支名出现在它自己的提交上
    assert_eq!(commits[2].refs, vec!["feature".to_owned()]);
}

#[test]
fn log_refs_include_tags_and_survive_non_ascii_subjects() {
    let commits = parse_log_format(&fixture("log_format_refs_and_utf8_subject.txt"));

    assert_eq!(commits.len(), 2);
    assert_eq!(
        commits[0].refs,
        vec!["HEAD -> main".to_owned(), "side".to_owned()]
    );
    assert_eq!(
        commits[1].refs,
        vec!["tag: v1.0.0".to_owned(), "tag: lightweight".to_owned()]
    );
    assert_eq!(commits[1].subject, "初始提交：支持中文与 emoji 的 subject");
}

// ---------------------------------------------------------------- ls-files -u

#[test]
fn ls_files_stage_empty_output_yields_no_entries() {
    assert!(parse_ls_files_stage(&fixture("ls_files_stage_empty.txt")).is_empty());
}

#[test]
fn ls_files_stage_parses_full_and_partial_conflicts() {
    let entries = parse_ls_files_stage(&fixture("ls_files_stage_unmerged.txt"));

    assert_eq!(entries.len(), 5);

    let c_stages: Vec<UnmergedStage> = entries
        .iter()
        .filter(|entry| entry.path.to_string() == "c.txt")
        .map(|entry| entry.stage)
        .collect();
    assert_eq!(
        c_stages,
        vec![
            UnmergedStage::Base,
            UnmergedStage::Ours,
            UnmergedStage::Theirs
        ]
    );

    // 删除/修改冲突天然只有两个 stage，不能被补成三个
    let d_stages: Vec<UnmergedStage> = entries
        .iter()
        .filter(|entry| entry.path.to_string() == "d.txt")
        .map(|entry| entry.stage)
        .collect();
    assert_eq!(d_stages, vec![UnmergedStage::Base, UnmergedStage::Ours]);

    assert!(entries.iter().all(|entry| entry.mode == 0o100644));
    assert!(entries.iter().all(|entry| entry.oid.len() == 40));
}

// ---------------------------------------------------------------- 鲁棒性

/// 确定性伪随机数发生器（xorshift64*）。
///
/// 为什么不用 `rand`：只需要"可复现的随机字节"，为此引入依赖不划算；
/// 固定种子还让失败用例可以原样重放。
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        usize::try_from(self.next_u64() % bound as u64).unwrap_or(0)
    }
}

/// 把一份输入喂给全部解析器。解析器**永不 panic** 是本测试的唯一断言。
fn feed_every_parser(input: &[u8]) {
    let status = parse_status_porcelain_v2(input);
    let stats = parse_diff_numstat(input);
    let commits = parse_log_format(input);
    let entries = parse_ls_files_stage(input);

    // 弱不变量：每条输出记录至少要消耗一个输入字节。
    // 它抓不住所有错误，但能抓住"死循环式产出"这一类（例如偏移没有推进）。
    assert!(status.entries.len() <= input.len() + 1);
    assert!(stats.len() <= input.len() + 1);
    assert!(commits.len() <= input.len() + 1);
    assert!(entries.len() <= input.len() + 1);
}

#[test]
fn parsers_never_panic_on_random_bytes() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);

    for _ in 0..500 {
        let length = rng.below(512);
        let input: Vec<u8> = (0..length).map(|_| (rng.next_u64() >> 33) as u8).collect();
        feed_every_parser(&input);
    }
}

#[test]
fn parsers_never_panic_on_mutated_fixtures() {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    let names = [
        "status_v2_initial_no_commit.txt",
        "status_v2_clean_with_upstream.txt",
        "status_v2_detached_head.txt",
        "status_v2_mixed_worktree.txt",
        "status_v2_rename.txt",
        "status_v2_copy.txt",
        "status_v2_unmerged.txt",
        "status_v2_submodule_dirty.txt",
        "status_v2_ignored.txt",
        "status_v2_paths_special.txt",
        "diff_numstat_basic.txt",
        "diff_numstat_binary.txt",
        "diff_numstat_rename.txt",
        "diff_numstat_copy.txt",
        "diff_numstat_utf8_path.txt",
        "log_format_linear_z.txt",
        "log_format_merge.txt",
        "log_format_refs_and_utf8_subject.txt",
        "ls_files_stage_unmerged.txt",
    ];

    for name in names {
        let original = fixture(name);

        // 截断：模拟"管道读到一半就断了"
        for cut in 0..original.len().min(64) {
            feed_every_parser(&original[..cut]);
        }

        // 字节翻转：模拟编码错误与 git 版本差异
        for _ in 0..200 {
            let mut mutated = original.clone();
            if mutated.is_empty() {
                continue;
            }
            let index = rng.below(mutated.len());
            mutated[index] = (rng.next_u64() >> 33) as u8;
            feed_every_parser(&mutated);
        }
    }
}

#[test]
fn status_parser_keeps_parsing_after_a_garbage_record() {
    // 一条畸形记录不应吃掉后面的正确记录
    let mut input = b"1 .M broken\0".to_vec();
    input.extend_from_slice(&fixture("status_v2_rename.txt"));

    let report = parse_status_porcelain_v2(&input);

    assert!(report
        .entries
        .iter()
        .any(|entry| entry.path.to_string() == "renamed.txt"));
}
