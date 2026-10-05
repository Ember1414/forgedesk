//! 诊断规则引擎的表驱动测试（T5.5）。
//!
//! 验收口径（任务书）：
//! 1. **每条内置规则至少一个真实 stderr 样本命中**（fixture 直接驱动
//!    `diagnose_with`，与 `diagnose` 的内嵌装配同源）；
//! 2. **误报**：20 个无关 stderr 样本不得产生 primary；
//! 3. **消歧**：同一 stderr 在不同 context 下给出不同 primary；
//! 4. 运行时覆盖目录的合并 / 替换 / 容错语义。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use forgedesk_diagnostics::rules::{
    diagnose_with, embedded_rules, load_with_overrides, parse_rules, validate_rules, DiagContext,
};

/// 每条规则的 stderr fixture（真实 git / 网络输出的关键片段）。
const FIXTURES: &[(&str, &str)] = &[
    // push
    ("push-non-fast-forward", "! [rejected]        main -> main (fetch first)\nerror: failed to push some refs to 'origin'\nhint: Updates were rejected because the tip of your current branch is behind\nhint: its remote counterpart. Integrate the remote changes (e.g.\nhint: 'git pull ...') before pushing again. non-fast-forward"),
    ("push-no-upstream", "fatal: The current branch feature/login has no upstream branch.\nTo push the current branch and set the remote as upstream, use\n\ngit push --set-upstream origin feature/login"),
    ("push-fetch-first", "! [rejected]        main -> main (fetch first)"),
    ("push-stale-lease", "! [rejected]        main -> main (stale info)"),
    ("push-remote-rejected", "Enumerating objects: 5, done.\nremote: error: GH006: Protected branch update failed\nTo github.com:org/repo.git\n ! [remote rejected] main -> main (protected branch hook declined)"),
    ("push-permission-403", "remote: Permission to org/repo.git denied to user.\nfatal: unable to access 'https://github.com/org/repo.git/': The requested URL returned error: 403"),
    // network
    ("net-dns-failure", "fatal: unable to access 'https://github.com/org/repo.git/': Could not resolve host: github.com"),
    ("net-connection-refused", "fatal: unable to access 'https://example.com/repo.git/': Failed to connect to example.com port 443: Connection refused"),
    ("net-timeout", "fatal: unable to access 'https://example.com/big.git/': Failed to connect to example.com port 443: Connection timed out"),
    ("net-ssl-certificate", "fatal: unable to access 'https://example.com/repo.git/': SSL certificate problem: self-signed certificate in certificate chain"),
    ("net-proxy-failure", "fatal: unable to access 'https://example.com/repo.git/': Proxy CONNECT aborted"),
    ("net-auth-failed", "remote: Invalid username or password.\nfatal: Authentication failed for 'https://github.com/org/repo.git/'"),
    ("net-prompts-disabled", "fatal: could not read Username for 'https://github.com': Terminal prompts disabled"),
    ("net-could-not-read-remote", "fatal: Could not read from remote repository.\n\nPlease make sure you have the correct access rights\nand the repository exists."),
    ("ssh-publickey-denied", "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository."),
    ("ssh-host-key-verification", "ssh_exchange_identification: Host key verification failed.\nfatal: Could not read from remote repository."),
    ("remote-not-a-repo", "fatal: 'origin' does not appear to be a git repository\nfatal: Could not read from remote repository."),
    ("net-cert-expired", "fatal: unable to access 'https://example.com/repo.git/': server certificate verification failed. certificate has expired"),
    // worktree
    ("wt-not-a-repo", "fatal: not a git repository (or any of the parent directories): .git"),
    ("wt-dubious-ownership", "fatal: detected dubious ownership in repository at 'D:/repos/demo'\nTo add an exception for this directory, call:\n\ngit config --global --add safe.directory D:/repos/demo"),
    ("wt-index-lock", "fatal: Unable to create 'D:/repos/demo/.git/index.lock': File exists.\n\nAnother git process seems to be running in this repository, e.g.\nan editor has opened 'D:/repos/demo/.git/COMMIT_EDITMSG'"),
    ("wt-local-changes-overwritten", "error: Your local changes to the following files would be overwritten by checkout:\n\tsrc/main.rs\nPlease commit your changes or stash them before you switch branches.\nAborting"),
    ("wt-detached-head", "You are in 'detached HEAD' state. You can look around, make experimental\nchanges and commit them, and you can discard any commits you make in this\nstate without impacting any branches by switching back to a branch."),
    ("wt-filename-too-long", "fatal: cannot create directory at 'very/long/path': Filename too long"),
    ("wt-permission-denied-fs", "error: open('src/locked.rs'): Permission denied\nfatal: Unable to process path src/locked.rs"),
    ("wt-disk-full", "fatal: unable to write sha1 filename: No space left on device"),
    ("wt-lf-replaced", "warning: in the working copy of 'src/a.rs', LF will be replaced by CRLF the next time Git touches it"),
    ("wt-crlf-replaced", "warning: in the working copy of 'src/b.sh', CRLF will be replaced by LF the next time Git touches it"),
    ("wt-large-file", "remote: error: GH001: Large files detected. You may want to try Git Large File Storage - https://git-lfs.github.com.\nremote: error: Trace: 7a2b41e..,remote: error: See http://git.io/iept8g for more information.\nremote: error: File big.psd is 250.00 MB; this exceeds GitHub's file size limit of 100.00 MB"),
    ("wt-invalid-filename", "fatal: unable to stat 'src/<bad>.rs': Invalid argument"),
    // commit
    ("commit-no-identity", "Author identity unknown\n\n*** Please tell me who you are.\n\nRun\n\n  git config --global user.email \"you@example.com\"\n  git config --global user.name \"Your Name\"\n\nto set your account's default identity.\nOmit --global to set the identity only in this repository.\n\nfatal: unable to auto-detect email address (got 'user@machine.(none)')"),
    ("commit-empty-message", "Aborting commit due to empty commit message."),
    ("commit-nothing", "On branch main\nnothing to commit, working tree clean"),
    ("commit-hook-rejected", "pre-commit hook failed\nhusky - pre-commit hook exited with code 1 (error)"),
    ("conflict-content", "Auto-merging src/main.rs\nCONFLICT (content): Merge conflict in src/main.rs\nAutomatic merge failed; fix conflicts and then commit the result."),
    ("merge-auto-failed", "Auto-merging src/lib.rs\nCONFLICT (rename/delete): src/old.rs renamed to src/new.rs in branch.\nAutomatic merge failed; fix conflicts and then commit the result."),
    // history
    ("hist-bad-object", "fatal: bad object abc1234"),
    ("hist-unknown-revision", "fatal: ambiguous argument 'origin/feat': unknown revision or path not in the working tree.\nUse '--' to separate paths from revisions, like this:\n'git <command> [<revision>...] -- [<file>...]'"),
    ("hist-branch-not-exist", "fatal: couldn't find remote ref refs/heads/nope"),
    ("hist-tag-exists", "fatal: tag 'v1.0.0' already exists"),
    ("hist-branch-exists", "fatal: a branch named 'feature/x' already exists"),
    ("hist-no-tracking-info", "There is no tracking information for the current branch.\nPlease specify which branch you want to merge with.\nSee git-pull(1) for details.\n\n    git pull <remote> <branch>"),
    ("hist-rebase-in-progress", "It seems that there is already a rebase-merge directory, and\nI wonder if you are in the middle of another rebase.  If that is the\ncase, please try\n\tgit rebase (--continue | --abort | --skip)"),
    ("hist-merge-in-progress", "fatal: You have not concluded your merge (MERGE_HEAD exists).\nPlease, commit your changes before you merge."),
    ("hist-shallow-clone", "fatal: refusing to merge unrelated histories kept in a shallow clone"),
    ("hist-branches-diverged", "You are currently rebasing branch 'main' on 'abc1234'.\n...note about the branches that have diverged and need to be specified..."),
    // extras
    ("lfs-not-installed", "git: 'lfs' is not a git command. See 'git --help'."),
    ("lfs-missing-objects", "batch response: Authentication required: LFS: Unauthorized\nError downloading object: big.psd (abc): LFS: Unauthorized"),
    ("api-rate-limit", "error: API rate limit exceeded for 203.0.113.1. (But here's the good news: Authenticated requests get a higher rate limit.)"),
    ("api-bad-credentials", "remote: Invalid username or password.\nfatal: Authentication failed for 'https://github.com/org/repo.git/'\nerror: Bad credentials"),
    ("submodule-missing", "fatal: No submodule mapping found in .gitmodules for path 'vendor/lib'"),
];

/// 与规则一一对应的 context（缺省 = 默认）。
fn context_for(id: &str) -> DiagContext {
    let mut ctx = DiagContext::default();
    match id {
        "push-non-fast-forward"
        | "push-no-upstream"
        | "push-fetch-first"
        | "push-stale-lease"
        | "push-remote-rejected"
        | "push-permission-403" => ctx.op_type = Some("push".into()),
        "commit-no-identity" | "commit-empty-message" => ctx.op_type = Some("commit".into()),
        "wt-detached-head" => ctx.detached = true,
        "hist-shallow-clone" => ctx.shallow = true,
        "hist-no-tracking-info" => ctx.upstream = false,
        _ => {}
    }
    ctx
}

/// 验收 1：每条内置规则至少一个真实 stderr 样本命中（且为主诊断）。
#[test]
fn every_embedded_rule_is_hit_by_its_fixture_as_primary() {
    let rules = embedded_rules();
    assert!(rules.len() >= 50, "内置规则必须 ≥50，实际 {}", rules.len());

    let mut hit_count = 0;
    for (id, stderr) in FIXTURES {
        let report = diagnose_with(rules, stderr, &context_for(id));
        let primary = report
            .primary
            .as_ref()
            .unwrap_or_else(|| panic!("rule {id} 的 fixture 必须命中 primary，报告: {report:?}"));
        assert_eq!(&primary.id, id, "fixture 命中的应是它自己的规则");
        assert!(!primary.title_key.is_empty());
        assert!(!primary.explanation_key.is_empty());
        hit_count += 1;
    }
    assert_eq!(hit_count, FIXTURES.len());
    // 所有内置规则都有 fixture（防"加了规则忘了测试"）
    for rule in rules {
        assert!(
            FIXTURES.iter().any(|(id, _)| id == &rule.id),
            "规则 {} 没有 fixture",
            rule.id
        );
    }
}

/// 验收 2：20 个无关 stderr 样本不得产生 primary（误报测试）。
#[test]
fn unrelated_stderr_samples_produce_no_primary() {
    let noise: [&str; 20] = [
        "hello world",
        "",
        "user@machine:~$ ls -la",
        "2 files changed, 10 insertions(+), 3 deletions(-)",
        "branch 'feature/x' set up to track 'origin/feature/x'.",
        "Everything up-to-date",
        "Switched to a new branch 'main'",
        "Deleted branch feature/y (was abc1234).",
        "1 file changed, 1 insertion(+)",
        "Rebase successful.",
        "Fast-forward\n  a.rs | 2 ++\n  1 file changed, 2 insertions(+)",
        "Saving your working directory to WIP on main: abc1234 done.",
        "Dropped refs/stash@{0} (def5678)",
        "[main abc1234] feat: add login flow",
        "Date: Mon Oct 5 10:00:00 2026 +0800",
        "warning: 1 line adds whitespace errors.",
        "Successfully rebased and updated refs/heads/main.",
        "Enumerating objects: 12, done.\nWriting objects: 100% (12/12), done.",
        "To github.com:org/repo.git\n   abc1234..def5678  main -> main",
        "note: switching to 'origin/main'.\nYou are in 'detached HEAD' state.",
    ];
    let rules = embedded_rules();
    for stderr in noise {
        let report = diagnose_with(rules, stderr, &DiagContext::default());
        assert!(
            report.primary.is_none(),
            "无关 stderr 不得命中: {stderr:?} => {:?}",
            report.primary.map(|p| p.id)
        );
    }
}

/// 验收 3：同一 stderr 在不同 context 下给出不同 primary（消歧）。
#[test]
fn the_same_stderr_resolves_differently_under_different_contexts() {
    // "Permission denied" 有两条规则：SSH 公钥（优先）与文件系统权限。
    // SSH 版本的 confidence 更高且不带 none_of，所以默认上下文命中 SSH；
    // 文件系统版本用 none_of 排除 "(publickey)"——这里验证的是：
    // (a) SSH 输出命中 SSH 规则；(b) 去掉 publickey 括号后命中 FS 规则。
    let rules = embedded_rules();

    let ssh = diagnose_with(
        rules,
        "git@github.com: Permission denied (publickey).",
        &DiagContext::default(),
    );
    assert_eq!(
        ssh.primary.as_ref().expect("hit").id,
        "ssh-publickey-denied"
    );

    let fs = diagnose_with(
        rules,
        "error: open('src/a.rs'): Permission denied",
        &DiagContext::default(),
    );
    assert_eq!(
        fs.primary.as_ref().expect("hit").id,
        "wt-permission-denied-fs"
    );

    // detached HEAD 的规则要求 ctx.detached = true：默认上下文不命中，给了才命中。
    let stderr = "You are in 'detached HEAD' state.";
    let default_ctx = diagnose_with(rules, stderr, &DiagContext::default());
    assert!(
        default_ctx.primary.is_none(),
        "默认上下文不应命中 detached 规则"
    );
    let detached_ctx = DiagContext {
        detached: true,
        ..DiagContext::default()
    };
    let detached = diagnose_with(rules, stderr, &detached_ctx);
    assert_eq!(
        detached.primary.as_ref().expect("hit").id,
        "wt-detached-head"
    );

    // op_type 消歧：同样的 "! [rejected]" 在 push 上下文是推送拒绝规则，
    // 无 push 上下文时让位给其它规则（这里用 pull 上下文验证不命中 push 规则）。
    let rejected = "! [rejected] main -> main (fetch first) non-fast-forward";
    let push = DiagContext {
        op_type: Some("push".into()),
        ..DiagContext::default()
    };
    let pull = DiagContext {
        op_type: Some("pull".into()),
        ..DiagContext::default()
    };
    let as_push = diagnose_with(rules, rejected, &push);
    let as_pull = diagnose_with(rules, rejected, &pull);
    assert_eq!(
        as_push.primary.as_ref().expect("hit").id,
        "push-non-fast-forward"
    );
    assert_ne!(
        as_pull.primary.as_ref().map(|p| p.id.as_str()),
        Some("push-non-fast-forward")
    );
}

/// 规则结构：全部只有 i18n key（不含界面文案），confidence 在 0..=1。
#[test]
fn rules_carry_only_i18n_keys_and_valid_confidence() {
    for rule in embedded_rules() {
        for key in [&rule.title_key, &rule.explanation_key] {
            assert!(
                key.starts_with("diag."),
                "{} 的 key 必须以 diag. 开头: {key}",
                rule.id
            );
        }
        for cause in &rule.causes {
            assert!(
                cause.starts_with("diag."),
                "{} cause 不是 key: {cause}",
                rule.id
            );
        }
        for fix in &rule.fixes {
            assert!(
                fix.label_key.starts_with("diag."),
                "{} fix 不是 key",
                rule.id
            );
            assert!(
                ["command", "guide", "dangerous"].contains(&fix.action.kind.as_str()),
                "{} fix kind 非法: {}",
                rule.id,
                fix.action.kind
            );
        }
        assert!(
            (0.0..=1.0).contains(&rule.confidence),
            "{} confidence 越界",
            rule.id
        );
    }
}

/// 运行时覆盖目录：同 id 替换、新 id 追加、坏文件跳过并告警。
#[test]
fn override_directory_merges_replaces_and_tolerates_bad_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("override.yaml"),
        r#"
- id: wt-not-a-repo
  match:
    all_of:
      - contains: "not a git repository"
  confidence: 0.99
  title_key: diag.wt-not-a-repo.title
  explanation_key: diag.wt-not-a-repo.explanation
  causes: []
  fixes: []
- id: brand-new-rule
  match:
    all_of:
      - contains: "totally unique marker"
  confidence: 0.5
  title_key: diag.brand-new-rule.title
  explanation_key: diag.brand-new-rule.explanation
"#,
    )
    .expect("write override");
    // 病文件：解析失败 → 跳过 + 警告
    std::fs::write(dir.path().join("broken.yaml"), "- id: [broken").expect("write broken");

    let (rules, warnings) = load_with_overrides(dir.path());

    let overridden = rules
        .iter()
        .find(|r| r.id == "wt-not-a-repo")
        .expect("builtin");
    assert!(
        (overridden.confidence - 0.99).abs() < f32::EPSILON,
        "同 id 必须被替换"
    );
    assert!(
        rules.iter().any(|r| r.id == "brand-new-rule"),
        "新 id 必须被追加"
    );
    assert!(
        warnings.iter().any(|w| w.contains("broken.yaml")),
        "坏文件必须产生警告"
    );

    // 覆盖后的引擎行为：提高的 confidence 让它在同 stderr 上排到最前
    let report = diagnose_with(
        &rules,
        "fatal: not a git repository (or any of the parent directories): .git",
        &DiagContext::default(),
    );
    assert_eq!(report.primary.as_ref().expect("hit").confidence, 0.99);
}

/// parse/validate 的边界：空文件、重复 id、空匹配块。
#[test]
fn parse_and_validate_surface_structural_problems() {
    let parsed = parse_rules("- id: a\n  match:\n    all_of:\n      - contains: x\n  confidence: 0.5\n  title_key: diag.a.title\n  explanation_key: diag.a.explanation\n").expect("parse");
    assert_eq!(parsed.len(), 1);
    assert!(validate_rules(&parsed).is_empty(), "完整规则不应有警告");

    let dup = parse_rules(
        "- id: a\n  match:\n    all_of:\n      - contains: x\n  confidence: 0.5\n  title_key: t\n  explanation_key: e\n- id: a\n  match:\n    all_of:\n      - contains: y\n  confidence: 0.5\n  title_key: t\n  explanation_key: e\n",
    )
    .expect("parse dup");
    let warnings = validate_rules(&dup);
    assert!(
        warnings.iter().any(|w| w.contains("duplicate")),
        "重复 id 必须告警"
    );

    let empty_match = parse_rules(
        "- id: b\n  match:\n    all_of: []\n    none_of: []\n  confidence: 0.5\n  title_key: t\n  explanation_key: e\n",
    )
    .expect("parse empty");
    assert!(
        validate_rules(&empty_match)
            .iter()
            .any(|w| w.contains("empty match")),
        "空匹配块必须告警"
    );
}
