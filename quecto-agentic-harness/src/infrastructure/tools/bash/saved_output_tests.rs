//! #2167: the saved-output directory limits itself. Every test prunes a
//! private temporary directory, never the real shared one.
#![cfg(unix)]
use super::saved_output::*;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

const HOUR: Duration = Duration::from_secs(3600);

fn small_policy(max_files: usize) -> PrunePolicy {
    PrunePolicy {
        max_age: Duration::from_secs(24 * 3600),
        max_files,
    }
}

fn owner() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// A saved-output file named as the tool names them, last modified `age` ago.
fn saved_file(dir: &Path, random: &str, now: SystemTime, age: Duration) -> PathBuf {
    let path = dir.join(format!("bash-output-{random}.log"));
    std::fs::write(&path, random).unwrap();
    set_age(&path, now, age);
    path
}

fn set_age(path: &Path, now: SystemTime, age: Duration) {
    let mtime = filetime::FileTime::from_system_time(now - age);
    filetime::set_symlink_file_times(path, mtime, mtime).unwrap();
}

#[test]
fn the_name_matcher_accepts_only_the_exact_tool_pattern() {
    assert!(is_saved_output_name(OsStr::new("bash-output-aB3xY9.log")));
    assert!(is_saved_output_name(OsStr::new("bash-output-000000.log")));
    for rejected in [
        "bash-output-aB3xY.log",
        "bash-output-aB3xY9Z.log",
        "bash-output-aB3-Y9.log",
        "bash-output-aB3.Y9.log",
        "bash-output-aB3xY9.log.bak",
        "bash-output-aB3xY9.txt",
        "xbash-output-aB3xY9.log",
        "other-aB3xY9.log",
        "bash-output-ééé.log",
        ".log",
        "",
    ] {
        assert!(
            !is_saved_output_name(OsStr::new(rejected)),
            "{rejected} must not match"
        );
    }
}

#[test]
fn a_non_utf8_name_is_not_a_saved_output() {
    use std::os::unix::ffi::OsStrExt;
    assert!(!is_saved_output_name(OsStr::from_bytes(
        b"bash-output-ab\xffdef.log"
    )));
}

#[test]
fn files_older_than_the_age_limit_are_removed() {
    let tmp = TempDir::new().unwrap();
    let now = SystemTime::now();
    let old = saved_file(tmp.path(), "old000", now, 25 * HOUR);
    let fresh = saved_file(tmp.path(), "fresh0", now, HOUR);

    let removed = prune_saved_outputs(tmp.path(), owner(), now, small_policy(200));

    assert_eq!(removed, 1);
    assert!(!old.exists(), "a 25-hour-old file is pruned");
    assert!(fresh.exists(), "a 1-hour-old file is kept");
}

#[test]
fn a_file_dated_in_the_future_is_kept() {
    let tmp = TempDir::new().unwrap();
    let now = SystemTime::now();
    let future = saved_file(tmp.path(), "future", now, Duration::ZERO);
    let mtime = filetime::FileTime::from_system_time(now + HOUR);
    filetime::set_file_mtime(&future, mtime).unwrap();

    assert_eq!(
        prune_saved_outputs(tmp.path(), owner(), now, small_policy(200)),
        0
    );
    assert!(future.exists());
}

#[test]
fn the_count_cap_removes_the_oldest_files_first() {
    let tmp = TempDir::new().unwrap();
    let now = SystemTime::now();
    let policy = SAVED_OUTPUT_POLICY;
    let files: Vec<PathBuf> = (0..policy.max_files + 5)
        .map(|i| {
            // File 0 is the oldest; all are within the age limit.
            let age = Duration::from_secs(60 * (policy.max_files + 5 - i) as u64);
            saved_file(tmp.path(), &format!("{i:06}"), now, age)
        })
        .collect();

    let removed = prune_saved_outputs(tmp.path(), owner(), now, policy);

    assert_eq!(removed, 5);
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 200);
    for (i, file) in files.iter().enumerate() {
        assert_eq!(file.exists(), i >= 5, "file {i}");
    }
}

#[test]
fn foreign_names_symlinks_and_directories_are_left_alone() {
    let tmp = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let now = SystemTime::now();
    let dir = tmp.path();

    let foreign = dir.join("notes-aB3xY9.log");
    std::fs::write(&foreign, "keep").unwrap();
    set_age(&foreign, now, 48 * HOUR);

    let target = saved_file(outside.path(), "target", now, 48 * HOUR);
    let link = dir.join("bash-output-link00.log");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    set_age(&link, now, 48 * HOUR);

    let subdir = dir.join("bash-output-subdir.log");
    std::fs::create_dir(&subdir).unwrap();
    let nested = saved_file(&subdir, "nested", now, 48 * HOUR);
    set_age(&subdir, now, 48 * HOUR);

    let removed = prune_saved_outputs(dir, owner(), now, small_policy(0));

    assert_eq!(removed, 0);
    assert!(foreign.exists(), "a file with a foreign name is kept");
    assert!(link.symlink_metadata().is_ok(), "a symlink is kept");
    assert!(target.exists(), "a symlink's target is never touched");
    assert!(subdir.is_dir(), "a directory is kept");
    assert!(nested.exists(), "pruning never recurses");
}

#[test]
fn a_symlinked_directory_is_not_pruned() {
    let real = TempDir::new().unwrap();
    let holder = TempDir::new().unwrap();
    let now = SystemTime::now();
    let old = saved_file(real.path(), "old000", now, 48 * HOUR);
    let link = holder.path().join("quecto-bash-output");
    std::os::unix::fs::symlink(real.path(), &link).unwrap();

    assert_eq!(
        prune_saved_outputs(&link, owner(), now, small_policy(200)),
        0
    );
    assert!(old.exists());
}

#[test]
fn a_directory_owned_by_someone_else_is_not_pruned() {
    let tmp = TempDir::new().unwrap();
    let now = SystemTime::now();
    let old = saved_file(tmp.path(), "old000", now, 48 * HOUR);

    let stranger = owner().wrapping_add(1);
    assert_eq!(
        prune_saved_outputs(tmp.path(), stranger, now, small_policy(200)),
        0
    );
    assert!(old.exists());
}

#[test]
fn a_missing_directory_prunes_nothing() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("absent");
    assert_eq!(
        prune_saved_outputs(&missing, owner(), SystemTime::now(), small_policy(0)),
        0
    );
}

#[test]
fn a_save_into_a_full_directory_keeps_the_new_file() {
    let tmp = TempDir::new().unwrap();
    let now = SystemTime::now();
    let oldest = saved_file(tmp.path(), "first0", now, 3 * HOUR);
    let middle = saved_file(tmp.path(), "second", now, 2 * HOUR);
    let newest = saved_file(tmp.path(), "third0", now, HOUR);

    let saved = save_output_in(tmp.path(), "fresh body", small_policy(2)).unwrap();

    assert_eq!(std::fs::read_to_string(&saved).unwrap(), "fresh body");
    assert!(is_saved_output_name(saved.file_name().unwrap()));
    assert!(!oldest.exists(), "the oldest file makes room");
    assert!(middle.exists());
    assert!(newest.exists(), "the newest earlier file survives");
}

#[test]
fn a_save_into_an_unreadable_directory_still_succeeds() {
    use std::os::unix::fs::PermissionsExt;
    if owner() == 0 {
        // root reads any directory, so the listing cannot be made to fail.
        return;
    }
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("write-only");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o300)).unwrap();

    let saved = save_output_in(&dir, "body", small_policy(0));

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let saved = saved.expect("an unreadable directory never fails the save");
    assert_eq!(std::fs::read_to_string(saved).unwrap(), "body");
}
