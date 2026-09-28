//! A session's records are its own (#2192 review): keys that sanitize to
//! one readable name never share a record, a record is taken only for the
//! session it names, and a clear removes the temporary files of writes a
//! killed process never finished.
use super::*;
use crate::domain::crash_record::PanicReport;

fn record(message: &str) -> CrashRecord {
    CrashRecord::new(PanicReport::new(message, None), std::process::id(), 7)
}

fn crash_dir(base: &Path) -> std::path::PathBuf {
    base.join("audit").join("crash")
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn message(dir: &RecordDir, key: &str, writer: Option<u32>) -> Option<String> {
    SessionRecords::new(key)
        .read(dir, writer)
        .map(|record| record.panic.message)
}

#[test]
fn keys_that_sanitize_alike_keep_apart_records() {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    let (colon, underscore) = (
        SessionRecords::new("telegram:123"),
        SessionRecords::new("telegram_123"),
    );
    colon.write_fatal(&dir, &record("colon's")).unwrap();
    colon
        .write_provisional(&dir, 1, &record("colon's call").provisional())
        .unwrap();
    // A new run of the other session clears only its own records.
    underscore.clear(&dir);
    assert_eq!(
        names_in(&crash_dir(base.path())).len(),
        2,
        "the other session's records stay"
    );
    let me = Some(std::process::id());
    assert_eq!(
        message(&dir, "telegram:123", me).as_deref(),
        Some("colon's")
    );
    for writer in [me, None] {
        assert_eq!(message(&dir, "telegram_123", writer), None, "{writer:?}");
    }
    underscore
        .write_fatal(&dir, &record("underscore's"))
        .unwrap();
    assert_eq!(
        message(&dir, "telegram_123", None).as_deref(),
        Some("underscore's")
    );
    assert_eq!(
        message(&dir, "telegram:123", None).as_deref(),
        Some("colon's")
    );
}

/// A record under a session's name that names another session — or none
/// — is not that session's, for a reader expecting a pid or not.
#[test]
fn a_record_naming_another_session_is_rejected() {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    let crash = crash_dir(base.path());
    let records = SessionRecords::new("telegram:123");
    let me = std::process::id();
    for (planted, session) in [
        (record("another's").for_session("telegram_123"), "another"),
        (record("nobody's"), "none"),
    ] {
        let text = serde_json::to_string(&planted).unwrap();
        std::fs::write(crash.join(&records.fatal), &text).unwrap();
        std::fs::write(crash.join(records.provisional(3)), &text).unwrap();
        for writer in [Some(me), None] {
            assert_eq!(records.read(&dir, writer), None, "{session} {writer:?}");
        }
    }
}

/// A process killed between creating its temporary file and renaming it
/// leaves the file; the session's next clear removes it — and only the
/// session's own, of the exact temporary shape.
#[test]
fn a_clear_removes_the_sessions_leftover_temporary_files() {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    let crash = crash_dir(base.path());
    let (mine, other) = (
        SessionRecords::new("cli:abc"),
        SessionRecords::new("cli:other"),
    );
    let left = [
        format!(".{}.00000000000000aa.tmp", mine.fatal),
        format!(".{}.0123456789abcdef.tmp", mine.provisional(4)),
    ];
    let kept = [
        format!(".{}.00000000000000aa.tmp", other.fatal),
        format!(".{}.notatemporary.tmp", mine.fatal),
        format!(".{}.00000000000000aa.tmp.keep", mine.fatal),
    ];
    for name in left.iter().chain(&kept) {
        std::fs::write(crash.join(name), "{\"partial").unwrap();
    }
    // A link under a temporary's shape is removed as the link, never followed.
    let target = base.path().join("target");
    std::fs::write(&target, "kept").unwrap();
    let linked = format!(".{}.00000000000000cc.tmp", mine.fatal);
    std::os::unix::fs::symlink(&target, crash.join(&linked)).unwrap();
    mine.clear(&dir);
    let mut expected = kept.to_vec();
    expected.sort();
    assert_eq!(names_in(&crash), expected);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "kept");
}
