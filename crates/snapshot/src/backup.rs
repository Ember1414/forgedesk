//! 未跟踪内容的备份、恢复与校验（T3.8 快照 v2）。
//!
//! # 磁盘布局
//!
//! ```text
//! <backup_root>/<repo_id>/<snapshot_id>/manifest.json        自描述清单
//! <backup_root>/<repo_id>/<snapshot_id>/untracked/<相对路径>  内容本体
//! <backup_root>/<repo_id>/.tmp-<token>/                      复制中途（崩溃残留）
//! ```
//!
//! # 为什么先写临时目录、再改名
//!
//! 最终目录名含快照 id，而 id 要等入库之后才有；更重要的是**原子性**：
//! 复制到一半崩溃时，留下的必须是一个一眼能认出是垃圾的 `.tmp-*` 目录，
//! 而不是一个"看起来完整、其实少了一半文件"的正式备份——后者会在回滚时
//! 悄悄恢复出半份内容。同文件系统内的 `rename` 是原子的，于是正式目录
//! "要么完整、要么不存在"。
//!
//! # 为什么逐字节比对而不是先存哈希
//!
//! 验收要求的是"回滚后逐字节一致"，哈希只是它的代理指标，还会引入新依赖
//! 与"哈希算法换了怎么办"。未跟踪文件通常是几十个中小文件，直接分块比对
//! 既更严格也更省事。
//!
//! # 为什么符号链接不进备份
//!
//! 备份要能"复制回来就是原样"。跟随链接会把仓库外的内容抄进备份目录
//! （既可能极大，也可能把 `~/.ssh` 这种不该进快照的东西带进去）；
//! 不跟随则备份的不是用户看到的东西。两者都错，所以链接被跳过——
//! 未跟踪文件里的符号链接极少见，跳过是代价最小的诚实做法。

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::{BackupEntry, BackupManifest, SnapshotLimits, SnapshotWarning};

/// 备份目录里存放内容本体的子目录名。
pub const UNTRACKED_SUBDIR: &str = "untracked";

/// 备份目录里的自描述清单文件名。
pub const MANIFEST_FILE: &str = "manifest.json";

/// 尚未改名到最终目录的临时目录前缀。
pub const TEMP_PREFIX: &str = ".tmp-";

/// 逐字节比对时一次读多少（未跟踪文件以中小文件为主，64 KiB 已足够摊薄系统调用）。
const COMPARE_CHUNK: usize = 64 * 1024;

/// 一个待备份的文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanFile {
    /// 相对仓库根的正斜杠路径（清单与恢复都用它）。
    pub relative: String,
    /// 源文件（工作区里的绝对路径）。
    pub source: PathBuf,
    /// 字节数。
    pub bytes: u64,
    /// 是否来自 gitignore 覆盖范围。
    pub ignored: bool,
}

/// 备份计划：要复制哪些、跳过哪些、为什么。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackupPlan {
    /// 将被备份的文件（超限时为空）。
    pub files: Vec<PlanFile>,
    /// 没有进备份的相对路径（超限时的全部候选）。
    pub skipped: Vec<String>,
    /// 候选内容的字节总数（超限时它是"如果把它们全备下要多少"）。
    pub total_bytes: u64,
    /// 计划阶段的告警（解析目录失败、超限）。
    pub warnings: Vec<SnapshotWarning>,
}

/// 生成备份计划。
///
/// 超限的处理是**整体跳过 + 明确告警**，不是"备到上限为止"：半份备份比没有
/// 备份更危险——用户会以为未跟踪内容都在里面。调用方把告警展示在危险操作的
/// 对话框里，让用户自己决定要不要带着这个代价继续。
pub fn plan(
    workdir: &Path,
    untracked: &[String],
    ignored: &[String],
    limits: &SnapshotLimits,
) -> BackupPlan {
    let mut candidates: Vec<(String, bool)> = Vec::new();
    for path in untracked {
        candidates.push((normalize(path), false));
    }
    if limits.include_ignored {
        for path in ignored {
            candidates.push((normalize(path), true));
        }
    }

    let mut files: Vec<PlanFile> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut detail = String::new();
    for (relative, ignored) in candidates {
        let absolute = workdir.join(&relative);
        if let Err(error) = collect(&absolute, &relative, ignored, &mut files) {
            // 单个条目读不了（被占用、权限）不该让整份计划失败：
            // 记下来，其余照常备份，最后如实在告警里说明
            if detail.is_empty() {
                detail = error.to_string();
            }
            failed.push(relative);
        }
    }

    let mut warnings = Vec::new();
    if !failed.is_empty() {
        warnings.push(SnapshotWarning::UntrackedBackupPartial {
            paths: failed.clone(),
            detail,
        });
    }

    let total_bytes = files.iter().map(|file| file.bytes).sum::<u64>();
    if limits.max_snapshot_bytes > 0 && total_bytes > limits.max_snapshot_bytes {
        let skipped = files.iter().map(|file| file.relative.clone()).collect();
        return BackupPlan {
            files: Vec::new(),
            skipped,
            total_bytes,
            warnings: vec![SnapshotWarning::UntrackedBackupSkipped {
                count: files.len(),
                bytes: total_bytes,
                limit: limits.max_snapshot_bytes,
            }],
        };
    }

    BackupPlan {
        files,
        skipped: Vec::new(),
        total_bytes,
        warnings,
    }
}

/// 把计划里的文件复制进 `staging`（临时目录），返回清单与失败项。
///
/// 单个文件复制失败只把它从清单里剔除（并交给调用方告警），
/// 不放弃整份备份：权限问题、被反病毒软件短暂占用都很常见，
/// 为一个文件让这次破坏性操作没有快照是不划算的。
pub fn copy_into(staging: &Path, files: &[PlanFile]) -> (BackupManifest, Vec<String>) {
    let mut entries: Vec<BackupEntry> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    for file in files {
        let destination = staging.join(UNTRACKED_SUBDIR).join(&file.relative);
        match copy_file(&file.source, &destination) {
            Ok(()) => entries.push(BackupEntry {
                path: file.relative.clone(),
                bytes: file.bytes,
                ignored: file.ignored,
            }),
            Err(_) => failed.push(file.relative.clone()),
        }
    }

    let bytes = entries.iter().map(|entry| entry.bytes).sum::<u64>();
    (
        BackupManifest {
            entries,
            bytes,
            // stash / 分支不在备份目录里（它们只是对象库里的提交与引用，一个字节
            // 都不复制），因此这份"目录自描述清单"没有它们——由 `create_locked`
            // 合并进数据库那份清单（见 `StashedRef` 与 `BranchRef` 的文档）
            stash: Vec::new(),
            branches: Vec::new(),
        },
        failed,
    )
}

/// 把清单写进备份目录（自描述副本）。
pub fn write_manifest(directory: &Path, manifest: &BackupManifest) -> io::Result<()> {
    fs::write(directory.join(MANIFEST_FILE), manifest.to_json())
}

/// 一次内容恢复的实情。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreSummary {
    /// 已经与备份一致的文件数（含本来就不需要改写的）。
    pub restored: usize,
    /// 恢复失败的文件（备份缺失、写不进去）。
    pub failed: Vec<String>,
}

/// 把备份内容恢复到工作区。
///
/// 幂等：目标已存在且逐字节相同时不重写（第二次回滚同一快照是安全的 no-op）。
pub fn restore(backup_dir: &Path, workdir: &Path, manifest: &BackupManifest) -> RestoreSummary {
    let mut summary = RestoreSummary::default();

    for entry in &manifest.entries {
        let source = backup_dir.join(UNTRACKED_SUBDIR).join(&entry.path);
        let target = workdir.join(&entry.path);

        if !source.is_file() {
            summary.failed.push(entry.path.clone());
            continue;
        }
        // 已经一致就不动它：少一次写盘，也让"重复回滚"真的没有副作用
        if target.is_file() && files_equal(&source, &target).unwrap_or(false) {
            summary.restored += 1;
            continue;
        }
        match copy_file(&source, &target) {
            Ok(()) => summary.restored += 1,
            Err(_) => summary.failed.push(entry.path.clone()),
        }
    }

    summary
}

/// 校验清单里的文件在工作区里都存在、且与备份逐字节一致；
/// 返回不一致的相对路径（空 = 全部一致）。
///
/// 判据是**与备份本体比对**，不是"文件存在且长度对"——后者会让
/// "恢复写进去一半"这种失败悄悄通过，而那正是要防的东西。
pub fn verify(backup_dir: &Path, workdir: &Path, manifest: &BackupManifest) -> Vec<String> {
    manifest
        .entries
        .iter()
        .filter(|entry| {
            let source = backup_dir.join(UNTRACKED_SUBDIR).join(&entry.path);
            let target = workdir.join(&entry.path);
            if !source.is_file() || !target.is_file() {
                return true;
            }
            !files_equal(&source, &target).unwrap_or(false)
        })
        .map(|entry| entry.path.clone())
        .collect()
}

/// 复制单个文件（顺带建父目录）。
fn copy_file(source: &Path, destination: &Path) -> io::Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination)?;
    Ok(())
}

/// 目录（递归）占用的字节数；不存在返回 0。
pub fn dir_bytes(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0_u64;
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            total = total.saturating_add(dir_bytes(&entry.path()));
        } else if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    total
}

/// 目录下的子目录名（孤儿清理据此与数据库比对）。
pub fn subdirs(path: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// 递归删除目录；不存在也算成功（清理与回退路径反复调用它）。
pub fn remove_dir(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// 两个文件是否逐字节相同（长度不同直接判否）。
pub fn files_equal(left: &Path, right: &Path) -> io::Result<bool> {
    let left_meta = fs::metadata(left)?;
    let right_meta = fs::metadata(right)?;
    if left_meta.len() != right_meta.len() {
        return Ok(false);
    }

    let mut left_file = fs::File::open(left)?;
    let mut right_file = fs::File::open(right)?;
    let mut left_buffer = vec![0_u8; COMPARE_CHUNK];
    let mut right_buffer = vec![0_u8; COMPARE_CHUNK];
    loop {
        let left_read = read_full(&mut left_file, &mut left_buffer)?;
        let right_read = read_full(&mut right_file, &mut right_buffer)?;
        if left_read != right_read {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
        if left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
    }
}

/// 读满缓冲或读到 EOF（`read` 一次不保证读满）。
fn read_full(file: &mut fs::File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let read = file.read(&mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

/// 递归收集条目下的普通文件（目录展开、链接与特殊文件跳过）。
fn collect(
    absolute: &Path,
    relative: &str,
    ignored: bool,
    out: &mut Vec<PlanFile>,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(absolute)?;

    if metadata.is_dir() {
        for entry in fs::read_dir(absolute)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // 未跟踪清单里不会出现 `.git`，这里是防御：仓库元数据绝不进备份
            if name == ".git" {
                continue;
            }
            let child_relative = format!("{relative}/{name}");
            collect(&entry.path(), &child_relative, ignored, out)?;
        }
        return Ok(());
    }

    // 符号链接与设备文件等一律跳过（见文件头）
    if !metadata.is_file() {
        return Ok(());
    }

    out.push(PlanFile {
        relative: relative.to_owned(),
        source: absolute.to_path_buf(),
        bytes: metadata.len(),
        ignored,
    });
    Ok(())
}

/// 归一化 git 输出的路径：去尾斜杠、统一正斜杠（目录条目形如 `build/`）。
pub(crate) fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        copy_into, dir_bytes, files_equal, plan, remove_dir, restore, subdirs, verify,
        MANIFEST_FILE, TEMP_PREFIX,
    };
    use crate::SnapshotLimits;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// 一次性的临时目录（测试互不干扰）。
    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("forgedesk-backup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("创建临时目录失败");
        directory
    }

    fn write_file(root: &Path, relative: &str, content: &[u8]) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("有父目录")).expect("创建父目录失败");
        fs::write(path, content).expect("写文件失败");
    }

    #[test]
    fn planning_expands_directories_and_counts_bytes() {
        let workdir = scratch("plan");
        write_file(&workdir, "scratch/a.txt", b"aaa");
        write_file(&workdir, "build/nested/b.txt", b"bbbbb");

        let plan = plan(
            &workdir,
            &["scratch/a.txt".to_owned(), "build/".to_owned()],
            &[],
            &SnapshotLimits::default(),
        );

        assert_eq!(plan.files.len(), 2, "目录要展开成其中的每个文件");
        assert_eq!(plan.total_bytes, 8);
        assert!(plan.skipped.is_empty());
        assert!(plan.warnings.is_empty());
        let paths: Vec<&str> = plan
            .files
            .iter()
            .map(|file| file.relative.as_str())
            .collect();
        assert!(paths.contains(&"build/nested/b.txt"));
        assert!(paths.contains(&"scratch/a.txt"));

        remove_dir(&workdir).unwrap();
    }

    #[test]
    fn planning_skips_everything_when_the_candidate_exceeds_the_limit() {
        let workdir = scratch("limit");
        write_file(&workdir, "big.bin", &[0_u8; 64]);

        let plan = plan(
            &workdir,
            &["big.bin".to_owned()],
            &[],
            &SnapshotLimits {
                max_snapshot_bytes: 8,
                ..SnapshotLimits::default()
            },
        );

        assert!(plan.files.is_empty(), "超限时整体不备份");
        assert_eq!(plan.skipped, vec!["big.bin".to_owned()]);
        assert_eq!(plan.total_bytes, 64);
        assert_eq!(plan.warnings.len(), 1, "超限必须有一句明确的告警");

        remove_dir(&workdir).unwrap();
    }

    #[test]
    fn ignored_files_are_only_planned_when_asked_for() {
        let workdir = scratch("ignored");
        write_file(&workdir, "kept.txt", b"k");
        write_file(&workdir, "node_modules/dep.js", b"dd");

        let without = plan(
            &workdir,
            &["kept.txt".to_owned()],
            &["node_modules/".to_owned()],
            &SnapshotLimits::default(),
        );
        assert_eq!(without.files.len(), 1);

        let with = plan(
            &workdir,
            &["kept.txt".to_owned()],
            &["node_modules/".to_owned()],
            &SnapshotLimits {
                include_ignored: true,
                ..SnapshotLimits::default()
            },
        );
        assert_eq!(with.files.len(), 2);
        assert!(with.files.iter().any(|file| file.ignored));

        remove_dir(&workdir).unwrap();
    }

    #[test]
    fn copying_then_restoring_returns_the_exact_bytes() {
        let workdir = scratch("restore");
        let backup = scratch("restore-store");
        write_file(&workdir, "notes/a.txt", b"original-a");
        write_file(&workdir, "b.bin", &[1_u8, 2, 3, 4]);

        let plan = plan(
            &workdir,
            &["notes/".to_owned(), "b.bin".to_owned()],
            &[],
            &SnapshotLimits::default(),
        );
        let (manifest, failed) = copy_into(&backup, &plan.files);
        assert!(failed.is_empty());
        assert_eq!(manifest.entries.len(), 2);

        // 模拟破坏性操作：内容被改写、文件被删
        fs::remove_file(workdir.join("notes/a.txt")).unwrap();
        fs::write(workdir.join("b.bin"), b"changed").unwrap();

        let summary = restore(&backup, &workdir, &manifest);
        assert_eq!(summary.restored, 2);
        assert!(summary.failed.is_empty());
        assert_eq!(
            fs::read(workdir.join("notes/a.txt")).unwrap(),
            b"original-a"
        );
        assert_eq!(
            fs::read(workdir.join("b.bin")).unwrap(),
            vec![1_u8, 2, 3, 4]
        );
        assert!(
            verify(&backup, &workdir, &manifest).is_empty(),
            "恢复后校验应通过"
        );

        // 幂等：第二次恢复不改变任何东西、也不报失败
        let again = restore(&backup, &workdir, &manifest);
        assert_eq!(again.restored, 2);
        assert!(again.failed.is_empty());

        remove_dir(&workdir).unwrap();
        remove_dir(&backup).unwrap();
    }

    #[test]
    fn verification_reports_files_that_no_longer_match() {
        let workdir = scratch("verify");
        let backup = scratch("verify-store");
        write_file(&workdir, "a.txt", b"aaaa");
        let plan = plan(
            &workdir,
            &["a.txt".to_owned()],
            &[],
            &SnapshotLimits::default(),
        );
        let (manifest, _) = copy_into(&backup, &plan.files);

        assert!(verify(&backup, &workdir, &manifest).is_empty());
        // 内容被改写（不是被删）：长度相同也算不一致
        fs::write(workdir.join("a.txt"), b"bbbb").unwrap();
        assert_eq!(
            verify(&backup, &workdir, &manifest),
            vec!["a.txt".to_owned()]
        );
        fs::remove_file(workdir.join("a.txt")).unwrap();
        assert_eq!(
            verify(&backup, &workdir, &manifest),
            vec!["a.txt".to_owned()]
        );

        remove_dir(&workdir).unwrap();
        remove_dir(&backup).unwrap();
    }

    #[test]
    fn directory_helpers_count_bytes_and_list_subdirectories() {
        let root = scratch("helpers");
        write_file(&root, "one/a.txt", b"12345");
        write_file(&root, "two/b.txt", b"123");
        assert_eq!(dir_bytes(&root), 8);
        assert_eq!(subdirs(&root), vec!["one".to_owned(), "two".to_owned()]);
        assert_eq!(dir_bytes(&root.join("missing")), 0, "不存在的目录算 0");

        remove_dir(&root).unwrap();
    }

    #[test]
    fn file_comparison_is_content_based() {
        let root = scratch("compare");
        write_file(&root, "a.bin", b"same");
        write_file(&root, "b.bin", b"same");
        write_file(&root, "c.bin", b"diff");
        write_file(&root, "d.bin", b"same-longer");

        assert!(files_equal(&root.join("a.bin"), &root.join("b.bin")).unwrap());
        assert!(!files_equal(&root.join("a.bin"), &root.join("c.bin")).unwrap());
        assert!(!files_equal(&root.join("a.bin"), &root.join("d.bin")).unwrap());

        remove_dir(&root).unwrap();
    }

    #[test]
    fn the_layout_constants_are_stable() {
        // 这些名字出现在磁盘上（用户可能直接去看），改名要有意识地做
        assert_eq!(MANIFEST_FILE, "manifest.json");
        assert_eq!(TEMP_PREFIX, ".tmp-");
        assert_eq!(super::UNTRACKED_SUBDIR, "untracked");
    }
}
