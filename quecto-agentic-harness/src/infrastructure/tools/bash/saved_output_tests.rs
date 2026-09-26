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

    // Room is left for the save that follows: it then holds exactly 200.
    assert_eq!(removed, 6);
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 199);
    for (i, file) in files.iter().enumerate() {
        assert_eq!(file.exists(), i >= 6, "file {i}");
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
    assert!(!oldest.exists(), "the oldest files make room");
    assert!(!middle.exists());
    assert!(newest.exists(), "the newest earlier file survives");
    assert_eq!(
        std::fs::read_dir(tmp.path()).unwrap().count(),
        2,
        "a save leaves at most max_files (#2167 review)"
    );
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

#[test]
fn only_a_real_directory_the_owner_owns_qualifies() {
    let tmp = TempDir::new().unwrap();
    let holder = TempDir::new().unwrap();
    let link = holder.path().join("linked");
    std::os::unix::fs::symlink(tmp.path(), &link).unwrap();

    assert!(is_owned_real_directory(tmp.path(), owner()));
    assert!(!is_owned_real_directory(
        tmp.path(),
        owner().wrapping_add(1)
    ));
    assert!(!is_owned_real_directory(&link, owner()));
}

#[test]
fn only_files_the_owner_owns_are_candidates() {
    let tmp = TempDir::new().unwrap();
    let now = SystemTime::now();
    let old = saved_file(tmp.path(), "old000", now, 48 * HOUR);

    let mine = saved_output_files(tmp.path(), owner()).unwrap();
    assert_eq!(
        mine,
        vec![(std::fs::metadata(&old).unwrap().modified().unwrap(), old)]
    );
    let theirs = saved_output_files(tmp.path(), owner().wrapping_add(1)).unwrap();
    assert!(
        theirs.is_empty(),
        "another user's file is never a candidate"
    );
}

/// #2167 review: output is never saved through a symlinked directory or
/// into another user's directory — the temp directory is shared.
#[test]
fn output_is_saved_only_into_a_real_directory_the_user_owns() {
    let tmp = tempfile::TempDir::new().unwrap();
    let target = tmp.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(save_output_in(&link, "secret", small_policy(200)).is_none());
    assert_eq!(
        std::fs::read_dir(&target).unwrap().count(),
        0,
        "nothing written"
    );
    let stranger = owner().wrapping_add(1);
    assert!(save_output_in_as(tmp.path(), "secret", small_policy(200), stranger).is_none());
    assert!(save_output_in(tmp.path(), "fine", small_policy(200)).is_some());
}

/// #2167 review: a new saved-output directory is private to its owner.
#[test]
fn a_new_saved_output_directory_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("a").join("saved");
    create_private_dir(&dir).unwrap();
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

/// #2167 review: expiry and the cap together remove whichever is more, not
/// their sum.
#[test]
fn expiry_and_the_cap_remove_the_larger_count() {
    let tmp = TempDir::new().unwrap();
    let now = SystemTime::now();
    for i in 0..3 {
        saved_file(tmp.path(), &format!("old{i:03}"), now, 25 * HOUR);
    }
    for i in 0..7 {
        saved_file(
            tmp.path(),
            &format!("new{i:03}"),
            now,
            Duration::from_secs(60 + i),
        );
    }
    // 10 files, 3 expired; a cap of 6 leaves room for 5, so 5 go.
    assert_eq!(
        prune_saved_outputs(tmp.path(), owner(), now, small_policy(6)),
        5
    );
}

/// #2167 review: a file exactly at the age limit is kept.
#[test]
fn a_file_exactly_at_the_age_limit_is_kept() {
    let tmp = TempDir::new().unwrap();
    let now = SystemTime::now();
    let policy = small_policy(200);
    let edge = saved_file(tmp.path(), "edge00", now, policy.max_age);
    assert_eq!(prune_saved_outputs(tmp.path(), owner(), now, policy), 0);
    assert!(edge.exists());
}

/// #2167 review: an owned directory others can write into (as an older
/// build may have left it) is made owner-only before a save.
#[test]
fn a_loose_owned_directory_is_made_private_before_a_save() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("loose");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(save_output_in(&dir, "body", small_policy(200)).is_some());
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

/// #2167 review: the old shared directory is swept of this user's files,
/// and nothing else in it is touched.
#[test]
fn the_legacy_sweep_removes_only_this_users_saved_files() {
    let tmp = TempDir::new().unwrap();
    let now = SystemTime::now();
    let mine = saved_file(tmp.path(), "mine00", now, HOUR);
    let other = tmp.path().join("notes.txt");
    std::fs::write(&other, "keep").unwrap();
    assert_eq!(prune_saved_outputs(tmp.path(), owner(), now, SWEEP_ALL), 1);
    assert!(!mine.exists());
    assert!(other.exists());
}
