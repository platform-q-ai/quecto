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

/// #2167 review: the old shared directory loses this user's expired files
/// only (a fresh one may belong to a running older build), nothing else in
/// it, and goes once empty.
#[test]
fn the_legacy_sweep_keeps_fresh_files_and_goes_once_empty() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join("legacy");
    std::fs::create_dir(&legacy).unwrap();
    let now = SystemTime::now();
    let old = saved_file(&legacy, "old000", now, 25 * HOUR);
    let fresh = saved_file(&legacy, "fresh0", now, HOUR);
    let other = legacy.join("notes.txt");
    std::fs::write(&other, "keep").unwrap();
    sweep_legacy(&legacy, owner(), now);
    assert!(!old.exists());
    assert!(fresh.exists(), "a running older build may still read it");
    assert!(other.exists());
    std::fs::remove_file(&fresh).unwrap();
    std::fs::remove_file(&other).unwrap();
    sweep_legacy(&legacy, owner(), now);
    assert!(!legacy.exists(), "an emptied legacy directory goes");
    // Gone, or never there: nothing to do, and nothing fails.
    sweep_legacy(&legacy, owner(), now);
}

fn view() -> TailView {
    TailView {
        start_line: 3,
        end_line: 9,
        total: 9,
        by_bytes: true,
        capture_cut: false,
        combined_len: 90,
        long_lines: None,
    }
}

/// #2167 review: a cut output says where the rest was saved, or that it
/// could not be and how to keep it.
#[test]
fn the_truncation_hint_says_where_the_rest_went() {
    assert_eq!(
        truncation_hint(Some("/t/x.log"), &view()),
        "\n[Showing lines 3-9 of 9 (50KB limit). Page through the saved file with read \
         (offset/limit). Full output (90 bytes) saved to: /t/x.log]"
    );
    let unsaved = truncation_hint(None, &view());
    assert!(unsaved.contains("could not be saved"), "{unsaved}");
    assert!(unsaved.contains("output_file"), "{unsaved}");
    assert!(unsaved.contains("Showing lines 3-9 of 9"), "{unsaved}");
}

/// #2196: cut lines are named in the note, saved or not.
#[test]
fn the_truncation_hint_names_cut_lines() {
    let view = TailView {
        long_lines: Some("line 9 is 60000 bytes".into()),
        ..view()
    };
    assert_eq!(
        truncation_hint(Some("/t/x.log"), &view),
        "\n[Showing lines 3-9 of 9 (50KB limit); line 9 is 60000 bytes. `read` the saved file for \
         lines up to 50KB whole, or reformat the output with `jq .` / `fold -w 200`. Full output \
         (90 bytes) saved to: /t/x.log]"
    );
    let unsaved = truncation_hint(None, &view);
    assert!(
        unsaved.contains("; line 9 is 60000 bytes. Reformat the output with `jq .`"),
        "{unsaved}"
    );
    assert!(unsaved.contains("could not be saved"), "{unsaved}");
}

/// #2167 review: the real entry point saves into the per-user, owner-only
/// directory and sweeps the old shared one.
#[tokio::test]
async fn a_save_goes_to_the_private_per_user_directory() {
    use std::os::unix::fs::PermissionsExt;
    // Other tests' saves sweep this directory too, and remove it once it is
    // empty: a foreign file keeps it, and seeding retries a lost race.
    let legacy = std::env::temp_dir().join("quecto-bash-output-lib-tests-legacy");
    let keep = legacy.join(format!("keep-{}.txt", std::process::id()));
    let seeded = (0..20)
        .any(|_| std::fs::create_dir_all(&legacy).is_ok() && std::fs::write(&keep, "keep").is_ok());
    assert!(seeded, "the legacy directory can be seeded");
    let expired = saved_file(&legacy, "exp000", SystemTime::now(), 25 * HOUR);
    let saved = save_to_temp_file("body".into()).await.unwrap();
    let dir = Path::new(&saved).parent().unwrap();
    assert_eq!(
        dir.file_name().unwrap(),
        OsStr::new(&format!("quecto-bash-output-lib-tests-{}", owner()))
    );
    assert_eq!(
        std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(!expired.exists(), "the old shared directory was swept");
    assert!(
        keep.exists(),
        "nothing but this user's saved files is swept"
    );
    std::fs::remove_file(&saved).unwrap();
    let _ = std::fs::remove_file(&keep);
}

/// #2167 review: tightening never follows a symlink.
#[test]
fn a_symlinked_directory_is_never_made_private() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(!make_private(&link));
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

/// #2187 review: this user's expired files are swept from an old shared
/// directory another user owns; the directory itself stays theirs.
#[test]
fn the_legacy_sweep_reaches_own_files_in_another_users_directory() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join("legacy");
    std::fs::create_dir(&legacy).unwrap();
    let now = SystemTime::now();
    let old = saved_file(&legacy, "old001", now, 25 * HOUR);
    // As if another user owned the directory: pruning never requires it
    // here, and removal of the directory needs ownership.
    let stranger = owner().wrapping_add(1);
    assert!(!is_owned_real_directory(&legacy, stranger));
    sweep_legacy(&legacy, owner(), now);
    assert!(!old.exists(), "own expired file swept");
    // A symlinked legacy directory is never entered.
    let target = tmp.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let theirs = saved_file(&target, "old002", now, 25 * HOUR);
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    sweep_legacy(&link, owner(), now);
    assert!(theirs.exists(), "a symlinked directory is not swept");
}

/// #2254 review: read refuses a file over its cap before any offset or
/// limit, so a saved output over it is paged with bash instead.
#[test]
fn the_hint_sends_a_saved_file_over_reads_cap_to_bash() {
    use crate::infrastructure::tools::filesystem::MAX_READ_BYTES;
    let cap = usize::try_from(MAX_READ_BYTES).unwrap();
    let at_cap = TailView {
        combined_len: cap,
        ..view()
    };
    let hint = truncation_hint(Some("/t/x.log"), &at_cap);
    assert!(
        hint.contains(" Page through the saved file with read (offset/limit). Full output"),
        "{hint}"
    );
    let over = TailView {
        combined_len: cap + 1,
        ..view()
    };
    assert_eq!(
        truncation_hint(Some("/t/x.log"), &over),
        format!(
            "\n[Showing lines 3-9 of 9 (50KB limit). It is over read's 10.0MB limit: page \
             through it with bash, e.g. sed -n '1,200p' '/t/x.log' or tail -n 200 '/t/x.log'. \
             Full output ({} bytes) saved to: /t/x.log]",
            cap + 1
        )
    );
    // Cut long lines over the cap: no advice to read the saved file.
    let over_long = TailView {
        long_lines: Some("line 9 is 60000 bytes".into()),
        ..over
    };
    let hint = truncation_hint(Some("/t/x.log"), &over_long);
    assert!(!hint.contains("`read` the saved file"), "{hint}");
    assert!(hint.contains("Reformat the output with `jq .`"), "{hint}");
    assert!(hint.contains("page through it with bash"), "{hint}");
}
