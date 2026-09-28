use super::*;
use std::path::Path;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn crash_dir(base: &Path) -> std::path::PathBuf {
    base.join("audit").join("crash")
}

/// A record file named `name`, last modified `days` ago.
fn aged(crash: &Path, name: &str, days: u32) {
    let file = std::fs::File::create(crash.join(name)).unwrap();
    file.set_modified(SystemTime::now() - DAY * days).unwrap();
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn stem(key: &str) -> String {
    super::super::super::filename::digest_session_key(key)
}

/// #2192 review: records of a session never resumed are removed once
/// stale — fatal, provisional and leftover temporary files alike — and
/// fresh ones, and anything not named as a record, are left.
#[test]
fn records_older_than_the_age_are_swept_and_the_rest_left() {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    let crash = crash_dir(base.path());
    let (old, fresh) = (stem("cli:gone"), stem("cli:live"));
    let stale = [
        format!("{old}.crash"),
        format!("{old}.crash.provisional.12.3"),
        format!(".{old}.crash.00000000000000aa.tmp"),
        format!(".{old}.crash.provisional.12.4.00000000000000bb.tmp"),
    ];
    for name in &stale {
        aged(&crash, name, 31);
    }
    let kept = [
        format!("{fresh}.crash"),
        format!("{old}.crash.provisional.12.5"),
        "notes.txt".to_string(),
        "cli_gone.crash".to_string(),
    ];
    aged(&crash, &kept[0], 1);
    aged(&crash, &kept[1], 29);
    aged(&crash, &kept[2], 400);
    aged(&crash, &kept[3], 400);
    let swept = sweep_within(&dir, SystemTime::now(), STALE_RECORD_AGE, MAX_LISTED);
    assert_eq!(
        swept,
        Swept {
            removed: stale.len(),
            complete: true
        }
    );
    let mut left = kept.to_vec();
    left.sort();
    assert_eq!(names_in(&crash), left);
}

/// Nothing but a regular file is swept: a link (never followed — its
/// target stays) and a directory under a record's name are left.
#[test]
fn a_sweep_removes_only_regular_files_and_follows_no_link() {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    let crash = crash_dir(base.path());
    let key = stem("cli:linked");
    let target = base.path().join("target");
    aged(base.path(), "target", 90);
    let link = crash.join(format!("{key}.crash"));
    std::os::unix::fs::symlink(&target, &link).unwrap();
    backdate_link(&link, 90);
    let planted = crash.join(format!("{key}.crash.provisional.1.1"));
    std::fs::create_dir(&planted).unwrap();
    std::fs::File::open(&planted)
        .unwrap()
        .set_modified(SystemTime::now() - DAY * 90)
        .unwrap();
    let swept = sweep_within(&dir, SystemTime::now(), STALE_RECORD_AGE, MAX_LISTED);
    assert_eq!(swept.removed, 0);
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(target.is_file(), "the link's target is untouched");
    assert!(planted.is_dir());
}

/// The link itself, last modified `days` ago (`utimensat`, not following it).
fn backdate_link(link: &Path, days: u32) {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(link.as_os_str().as_bytes()).unwrap();
    let then = SystemTime::now() - DAY * days;
    let secs = then
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let time = libc::timespec {
        tv_sec: secs as libc::time_t,
        tv_nsec: 0,
    };
    let times = [time, time];
    // SAFETY: `path` is NUL-terminated and `times` holds the two entries utimensat reads.
    let status = unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            path.as_ptr(),
            times.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    assert_eq!(status, 0, "{}", std::io::Error::last_os_error());
    let modified = link.symlink_metadata().unwrap().modified().unwrap();
    assert!(modified < SystemTime::now() - DAY * (days - 1));
}

/// A sweep examines a bounded number of entries, and says when it
/// stopped short.
#[test]
fn a_sweep_examines_a_bounded_number_of_entries() {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    let crash = crash_dir(base.path());
    for n in 0..20 {
        aged(&crash, &format!("{}.crash", stem(&format!("cli:{n}"))), 60);
    }
    let swept = sweep_within(&dir, SystemTime::now(), STALE_RECORD_AGE, 8);
    assert!(!swept.complete);
    assert!((6..=8).contains(&swept.removed), "{swept:?}");
    assert_eq!(names_in(&crash).len(), 20 - swept.removed);
}

/// The sweep runs when the target is made at startup: a session never
/// resumed does not keep its records for ever — nor the arming session's
/// own fresh records are touched.
#[test]
fn a_start_sweeps_stale_records() {
    let base = tempfile::tempdir().unwrap();
    RecordDir::create(base.path()).unwrap();
    let crash = crash_dir(base.path());
    let stale = format!("{}.crash", stem("cli:never-resumed"));
    let fresh = format!("{}.crash", stem("cli:recent"));
    aged(&crash, &stale, 45);
    aged(&crash, &fresh, 2);
    let _armed = super::super::Armed::new(base.path(), None, None);
    assert_eq!(names_in(&crash), [fresh]);
}

#[test]
fn the_stale_age_is_thirty_days() {
    assert_eq!(STALE_RECORD_AGE, DAY * 30);
}
