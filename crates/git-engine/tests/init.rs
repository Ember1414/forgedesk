//! `git init` 的目标目录语义（2026-10-08 修复的回归护栏）。
//!
//! # 契约
//!
//! "初始化到指定目录"必须**自己把目录建出来**（含缺失的父目录）。
//!
//! # 为什么这条契约曾经只是文档
//!
//! `init` 是以目标路径作为**工作目录**执行的（`run_write_at`，命令里不带路径），
//! 而进程层对不存在的工作目录直接拒绝（"找不到目标"），于是"目录不存在"这条
//! 完全正常的路径在界面上表现为"初始化失败"。用户的实际操作路径就是它：
//! 新建一个工程目录 → 直接在里面初始化仓库。
//!
//! 集成测试是独立 crate，测试代码里允许 panic 式写法（失败即断言失败）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use forgedesk_domain::git::InitSpec;
use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engine::{CliGitEngine, GitEngine};

use support::TempDir;

fn engine() -> CliGitEngine {
    CliGitEngine::new().expect("创建 CLI 引擎失败")
}

#[test]
fn creates_the_target_directory_when_it_does_not_exist() {
    let dir = TempDir::new("init-missing-target");
    let target = dir.path().join("brand-new");
    assert!(!target.exists(), "前置条件：目标目录还不存在");

    let info = engine()
        .init(&target, InitSpec::default())
        .expect("目标目录不存在时 init 必须自己创建它");

    assert!(target.is_dir(), "目标目录应当被创建出来");
    assert!(target.join(".git").is_dir(), "应当是一个真实的 git 仓库");
    assert!(info.is_empty, "刚初始化的仓库是空的");
    assert!(!info.is_bare);
    // workdir 必须指向刚创建的那个目录，而不是它的父目录
    let workdir = info.workdir.as_deref().expect("非裸仓库必须有工作区");
    assert_eq!(
        workdir.canonicalize().expect("canonicalize 工作区"),
        target.canonicalize().expect("canonicalize 目标目录")
    );
}

#[test]
fn creates_missing_parent_directories_too() {
    // "在 D:\code\新工程 里初始化，而 D:\code 还不存在" 是真实用法
    let dir = TempDir::new("init-missing-parents");
    let target = dir.path().join("code").join("projects").join("fresh");

    engine()
        .init(&target, InitSpec::default())
        .expect("缺失的父目录也要一并创建");

    assert!(target.join(".git").is_dir());
}

#[test]
fn an_existing_directory_is_reused_rather_than_rejected() {
    let dir = TempDir::new("init-existing");
    let target = dir.path().join("already-here");
    std::fs::create_dir_all(&target).expect("准备已存在的目录");
    std::fs::write(target.join("README.md"), b"hello\n").expect("放一个已有文件");

    engine()
        .init(&target, InitSpec::default())
        .expect("对已存在的目录初始化应当成功（git init 是幂等的）");

    assert!(target.join(".git").is_dir());
    assert!(
        target.join("README.md").is_file(),
        "初始化不能动用户已有的文件"
    );
}

#[test]
fn a_file_at_the_target_path_reports_a_validation_error() {
    let dir = TempDir::new("init-path-is-file");
    let target = dir.path().join("not-a-directory");
    std::fs::write(&target, b"i am a file\n").expect("准备一个同名文件");

    let error = engine()
        .init(&target, InitSpec::default())
        .expect_err("目标路径是文件时必须失败");

    // 必须是"可操作的输入错误"，而不是内部错误或超时——
    // 用户要做的是换一个路径，而不是重试或报 bug
    assert_eq!(error.code, ErrorCode::Validation);
    assert!(
        !error.retryable,
        "重试同一个路径不会有别的结果，不该给出重试按钮"
    );
    assert!(
        error
            .message
            .contains("could not create the target directory"),
        "错误信息要指向真正失败的动作，实际是：{}",
        error.message
    );
}
