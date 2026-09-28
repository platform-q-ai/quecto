use super::*;
use crate::domain::crash_record::PanicReport;
use std::os::unix::fs::PermissionsExt;

/// A record as this process's hook writes it: its own pid.
fn record(message: &str) -> CrashRecord {
    CrashRecord::new(
        PanicReport::new(message, Some("src/edit.rs:10:5")),
        std::process::id(),
        7,
    )
    .running(vec!["edit".into()])
}

/// Where a base's crash records live (#2192 review L-a): their own
/// directory, apart from every session's event log.
fn crash_dir(base: &Path) -> std::path::PathBuf {
    base.join("audit").join("crash")
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The fatal record's name of `key`: its digest's (#2192 review).
fn fatal_name(key: &str) -> String {
    AuditLog::crash_record_name(key)
}

fn armed(base: &Path, key: &str) -> Armed {
    Armed::new(base, Some(key), None)
}

fn read(base: &Path, key: &str) -> Option<CrashRecord> {
    SessionRecords::new(key).read(&RecordDir::existing(base).ok()?, Some(std::process::id()))
}

#[test]
fn a_fatal_record_is_written_whole_to_a_private_file() {
    let base = tempfile::tempdir().unwrap();
    armed(base.path(), "cli:abc").record_fatal(&record("boom"), "panic", 3);
    assert_eq!(
        read(base.path(), "cli:abc"),
        Some(record("boom").for_session("cli:abc"))
    );
    let crash = crash_dir(base.path());
    assert_eq!(
        names_in(&crash),
        [fatal_name("cli:abc")],
        "no temporary left"
    );
    assert_eq!(mode(&crash.join(fatal_name("cli:abc"))), 0o600);
    assert_eq!(mode(&crash), 0o700);
    assert_eq!(mode(&base.path().join("audit")), 0o700);
}

#[test]
fn a_provisional_record_is_the_calls_own_file_and_is_withdrawn_by_name() {
    let base = tempfile::tempdir().unwrap();
    let target = armed(base.path(), "cli:abc");
    target.record_provisional(1, &record("one").provisional());
    target.record_provisional(2, &record("two").provisional());
    let (pid, fatal) = (std::process::id(), fatal_name("cli:abc"));
    assert_eq!(
        names_in(&crash_dir(base.path())),
        [
            format!("{fatal}.provisional.{pid}.1"),
            format!("{fatal}.provisional.{pid}.2")
        ]
    );
    target.withdraw(1, Some("cli:abc"));
    assert_eq!(
        read(base.path(), "cli:abc").map(|r| r.panic.message),
        Some("two".to_string()),
        "another scope's record is untouched"
    );
    target.withdraw(2, None);
    assert_eq!(read(base.path(), "cli:abc"), None);
}

#[test]
fn a_provisional_record_is_withdrawn_from_the_session_it_was_written_under() {
    let base = tempfile::tempdir().unwrap();
    let target = armed(base.path(), "cli:a");
    let under = target.record_provisional(42, &record("x").provisional());
    assert_eq!(under.as_deref(), Some("cli:a"));
    target.follow(Some("cli:b"));
    target.withdraw(42, under.as_deref());
    assert_eq!(
        read(base.path(), "cli:a"),
        None,
        "withdrawn where it was written"
    );
    // Unnoted (the write's session unknown): the current session's is removed.
    let under = target.record_provisional(43, &record("y").provisional());
    assert_eq!(under.as_deref(), Some("cli:b"));
    target.withdraw(43, None);
    assert_eq!(read(base.path(), "cli:b"), None);
}

#[test]
fn a_disarmed_target_writes_no_provisional_record_and_names_no_session() {
    let base = tempfile::tempdir().unwrap();
    let target = Armed::new(base.path(), None, None);
    assert_eq!(
        target.record_provisional(1, &record("x").provisional()),
        None
    );
    let target = armed(base.path(), "cli:a");
    target.follow(Some(""));
    assert_eq!(
        target.record_provisional(1, &record("x").provisional()),
        None
    );
}

#[test]
fn the_fatal_record_is_preferred_and_never_withdrawn() {
    let base = tempfile::tempdir().unwrap();
    let target = armed(base.path(), "cli:abc");
    target.record_provisional(1, &record("provisional").provisional());
    target.record_fatal(&record("fatal"), "panic", 0);
    assert_eq!(
        read(base.path(), "cli:abc").map(|r| r.panic.message),
        Some("fatal".to_string())
    );
    for scope in [1, 2, 3] {
        target.withdraw(scope, Some("cli:abc"));
    }
    assert_eq!(
        read(base.path(), "cli:abc"),
        Some(record("fatal").for_session("cli:abc"))
    );
}

/// M1: nothing a concurrent call does to its own provisional record can
/// drop or replace the fatal record.
#[test]
fn a_fatal_panic_while_other_calls_write_and_withdraw_still_leaves_the_fatal_record() {
    let base = tempfile::tempdir().unwrap();
    let target = std::sync::Arc::new(armed(base.path(), "cli:abc"));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let churn: Vec<_> = (0..4)
        .map(|scope| {
            let (target, stop) = (target.clone(), stop.clone());
            std::thread::spawn(move || {
                let provisional = record("churn").provisional();
                while !stop.load(Ordering::Relaxed) {
                    let under = target.record_provisional(scope, &provisional);
                    target.withdraw(scope, under.as_deref());
                }
            })
        })
        .collect();
    for _ in 0..50 {
        target.record_fatal(&record("fatal"), "panic", 0);
        assert_eq!(
            read(base.path(), "cli:abc").map(|r| (r.panic.message, r.provisional)),
            Some(("fatal".to_string(), false))
        );
    }
    stop.store(true, Ordering::Relaxed);
    churn.into_iter().for_each(|thread| thread.join().unwrap());
    assert_eq!(
        read(base.path(), "cli:abc"),
        Some(record("fatal").for_session("cli:abc"))
    );
}

#[test]
fn following_a_session_clears_what_an_earlier_run_left_and_records_there() {
    let base = tempfile::tempdir().unwrap();
    let earlier = armed(base.path(), "cli:next");
    earlier.record_fatal(&record("old"), "panic", 0);
    earlier.record_provisional(9, &record("old").provisional());
    let target = armed(base.path(), "cli:first");
    target.follow(Some("cli:next"));
    assert_eq!(
        read(base.path(), "cli:next"),
        None,
        "the stale records are gone"
    );
    target.record_fatal(&record("new"), "panic", 0);
    assert_eq!(
        read(base.path(), "cli:next"),
        Some(record("new").for_session("cli:next"))
    );
    assert_eq!(read(base.path(), "cli:first"), None);
    // A session that leaves nothing behind: disarmed.
    target.follow(None);
    target.record_fatal(&record("ephemeral"), "panic", 0);
    assert_eq!(
        read(base.path(), "cli:next"),
        Some(record("new").for_session("cli:next"))
    );
}

#[test]
fn a_disarmed_target_creates_nothing() {
    let base = tempfile::tempdir().unwrap();
    let target = Armed::new(base.path(), None, None);
    target.record_fatal(&record("x"), "panic", 0);
    target.record_provisional(1, &record("x").provisional());
    assert!(!base.path().join("audit").exists());
}

#[test]
fn a_missing_malformed_or_oversized_record_reads_as_none() {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    let path = crash_dir(base.path()).join(fatal_name("cli:abc"));
    assert_eq!(read(base.path(), "cli:abc"), None);
    std::fs::write(&path, "not json").unwrap();
    assert_eq!(read(base.path(), "cli:abc"), None);
    let mut padded = serde_json::to_string(&record("x")).unwrap();
    padded.push_str(&" ".repeat(MAX_RECORD_BYTES as usize));
    std::fs::write(&path, padded).unwrap();
    assert_eq!(read(base.path(), "cli:abc"), None, "padded past the bound");
    drop(dir);
}

#[test]
fn a_record_read_back_is_bounded_again_whoever_wrote_it() {
    let base = tempfile::tempdir().unwrap();
    RecordDir::create(base.path()).unwrap();
    let forged = serde_json::json!({
        "message": "m".repeat(5000),
        "location": "l".repeat(5000),
        "running": (0..40).map(|i| format!("t{i}")).collect::<Vec<_>>(),
        "pid": std::process::id(),
        "unix_ms": 1,
        "session": "cli:abc",
    });
    std::fs::write(
        crash_dir(base.path()).join(fatal_name("cli:abc")),
        forged.to_string(),
    )
    .unwrap();
    let read = read(base.path(), "cli:abc").unwrap();
    assert_eq!(read.panic.message.len(), 2048);
    assert_eq!(read.panic.location.unwrap().len(), 2048);
    assert_eq!(read.running.len(), 16);
}

#[test]
fn a_fifo_planted_as_the_record_is_not_waited_on() {
    let base = tempfile::tempdir().unwrap();
    RecordDir::create(base.path()).unwrap();
    let path = std::ffi::CString::new(
        crash_dir(base.path())
            .join(fatal_name("cli:abc"))
            .into_os_string()
            .into_encoded_bytes(),
    )
    .unwrap();
    // SAFETY: `path` is NUL-terminated.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let started = std::time::Instant::now();
    assert_eq!(read(base.path(), "cli:abc"), None);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn no_link_is_followed() {
    let base = tempfile::tempdir().unwrap();
    RecordDir::create(base.path()).unwrap();
    let elsewhere = base.path().join("elsewhere");
    let planted = serde_json::to_string(&record("planted")).unwrap();
    std::fs::write(&elsewhere, &planted).unwrap();
    let path = crash_dir(base.path()).join(fatal_name("cli:abc"));
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    assert_eq!(
        read(base.path(), "cli:abc"),
        None,
        "a planted link is not read"
    );
    armed(base.path(), "cli:other").record_fatal(&record("x"), "panic", 0);
    let target = Armed::new(base.path(), None, None);
    target.follow(Some("cli:abc"));
    target.record_fatal(&record("mine"), "panic", 0);
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), planted);
    assert_eq!(
        read(base.path(), "cli:abc"),
        Some(record("mine").for_session("cli:abc"))
    );

    let linked_base = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(base.path().join("audit"), linked_base.path().join("audit"))
        .unwrap();
    assert!(RecordDir::existing(linked_base.path()).is_err());
    assert!(RecordDir::create(linked_base.path()).is_err());

    // The crash directory itself planted as a link is refused too.
    let planted_base = tempfile::tempdir().unwrap();
    std::fs::create_dir(planted_base.path().join("audit")).unwrap();
    std::os::unix::fs::symlink(crash_dir(base.path()), crash_dir(planted_base.path())).unwrap();
    assert!(RecordDir::existing(planted_base.path()).is_err());
    assert!(RecordDir::create(planted_base.path()).is_err());
}

#[test]
fn the_fatal_event_log_line_is_filed_under_its_turn() {
    let base = tempfile::tempdir().unwrap();
    let key = "cli:logged";
    let log = AuditLog::open_sync(base.path(), key)
        .unwrap()
        .with_parent(Some("parent".into()));
    let target = Armed::new(base.path(), Some(key), log.crash_line());
    target.record_fatal(&record("boom").in_call("edit"), "panic", 4);
    let lines = std::fs::read_to_string(AuditLog::file_path(base.path(), key)).unwrap();
    let event: serde_json::Value = serde_json::from_str(lines.trim_end()).unwrap();
    assert_eq!(event["event"], "error");
    assert_eq!(event["source"], "panic");
    assert_eq!(event["turn"], 4);
    assert_eq!(event["tool"], "edit");
    assert_eq!(event["message"], "boom");
    assert_eq!(event["location"], "src/edit.rs:10:5");
    assert_eq!(event["session"], key);
    assert_eq!(event["parent"], "parent");
}

#[test]
#[should_panic(expected = "a crash record names its session")]
fn records_need_a_session() {
    let _ = SessionRecords::new("");
}

/// Info (#2192 review): the records a reader finds are listed from the
/// directory it holds, not from whatever is at its path now.
#[test]
fn records_are_listed_through_the_held_directory_not_its_path() {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    let records = SessionRecords::new("cli:a");
    records
        .write_provisional(&dir, 1, &record("held").provisional())
        .unwrap();
    let crash = crash_dir(base.path());
    std::fs::rename(&crash, base.path().join("moved")).unwrap();
    std::fs::create_dir(&crash).unwrap();
    let planted =
        serde_json::to_string(&record("planted").provisional().for_session("cli:a")).unwrap();
    let (pid, fatal) = (std::process::id(), fatal_name("cli:a"));
    std::fs::write(crash.join(format!("{fatal}.provisional.{pid}.2")), planted).unwrap();
    assert_eq!(
        records
            .read(&dir, Some(pid))
            .map(|r| r.panic.message)
            .as_deref(),
        Some("held")
    );
}

#[test]
fn a_listing_examines_a_bounded_number_of_entries() {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    for n in 0..20 {
        std::fs::write(crash_dir(base.path()).join(format!("p.{n}")), "").unwrap();
    }
    let all = dir.names_with_prefix_within("p.", 100);
    assert_eq!((all.names.len(), all.complete), (20, true));
    // `.` and `..` are entries too: at most the five examined match, and
    // the listing says it stopped short (#2192 review: never silently).
    let bounded = dir.names_with_prefix_within("p.", 5);
    assert!((3..=5).contains(&bounded.names.len()), "{bounded:?}");
    assert!(!bounded.complete, "a capped listing is marked incomplete");
    let none = dir.names_with_prefix_within("q.", 100);
    assert_eq!((none.names, none.complete), (Vec::<String>::new(), true));
    let names = dir.names_with_prefix_within("p.", 100).names;
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "sorted");
}

/// #2192 review L-a: a base with many sessions' event logs does not crowd
/// a session's crash records out of the listing, nor out of its clear.
#[test]
fn many_event_logs_do_not_hide_a_sessions_records() {
    let base = tempfile::tempdir().unwrap();
    let target = armed(base.path(), "cli:abc");
    let audit = base.path().join("audit");
    for n in 0..(MAX_LISTED + 100) {
        std::fs::write(audit.join(format!("cli_{n:05}.jsonl")), "").unwrap();
    }
    target.record_provisional(1, &record("seen").provisional());
    assert_eq!(
        read(base.path(), "cli:abc")
            .map(|r| r.panic.message)
            .as_deref(),
        Some("seen")
    );
    SessionRecords::new("cli:abc").clear(&RecordDir::existing(base.path()).unwrap());
    assert_eq!(read(base.path(), "cli:abc"), None, "cleared too");
    assert!(names_in(&crash_dir(base.path())).is_empty());
}

/// #2192 review L-b: a record another process planted — even a newer one —
/// does not stand in for the one the expected process wrote.
#[test]
fn a_reader_takes_only_the_expected_process_records() {
    let base = tempfile::tempdir().unwrap();
    let target = armed(base.path(), "cli:abc");
    target.record_provisional(1, &record("real").provisional());
    let crash = crash_dir(base.path());
    let fatal = fatal_name("cli:abc");
    let mut forged = record("forged").provisional().for_session("cli:abc");
    forged.pid = 999_999;
    forged.unix_ms = u64::MAX;
    let text = serde_json::to_string(&forged).unwrap();
    std::fs::write(crash.join(format!("{fatal}.provisional.999999.1")), &text).unwrap();
    // Named for this process, but saying another pid: not this process's.
    let me = std::process::id();
    std::fs::write(crash.join(format!("{fatal}.provisional.{me}.9")), &text).unwrap();
    // Saying this pid, but under another process's name: not read for it.
    let mut misfiled = record("misfiled").provisional().for_session("cli:abc");
    misfiled.unix_ms = u64::MAX;
    let misfiled = serde_json::to_string(&misfiled).unwrap();
    std::fs::write(
        crash.join(format!("{fatal}.provisional.999998.1")),
        misfiled,
    )
    .unwrap();
    assert_eq!(
        read(base.path(), "cli:abc")
            .map(|r| r.panic.message)
            .as_deref(),
        Some("real")
    );
    let dir = RecordDir::existing(base.path()).unwrap();
    let records = SessionRecords::new("cli:abc");
    assert_eq!(
        records
            .read(&dir, Some(999_999))
            .map(|r| r.panic.message)
            .as_deref(),
        Some("forged"),
        "the other process's own record is its"
    );
    assert_eq!(records.read(&dir, Some(1)), None, "nothing of pid 1");
    // A fatal record of another process is not the expected one's.
    std::fs::write(crash.join(fatal_name("cli:abc")), &text).unwrap();
    assert_eq!(
        read(base.path(), "cli:abc")
            .map(|r| r.panic.message)
            .as_deref(),
        Some("real")
    );
    // With no process expected, the newest of any is read (unverified).
    assert_eq!(
        records.read(&dir, None).map(|r| r.panic.message).as_deref(),
        Some("forged")
    );
}

/// #2192 review L-c: a co-tenant that pre-creates the temporary name a
/// write would take does not stop the record from being written.
#[test]
fn a_pre_created_temporary_name_does_not_stop_a_write() {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    let (crash, fatal) = (crash_dir(base.path()), fatal_name("cli:abc"));
    std::fs::write(
        crash.join(format!(".{fatal}.0000000000000001.tmp")),
        "squatted",
    )
    .unwrap();
    let mut suffixes = [1u64, 1, 2].into_iter();
    dir.write_with(
        &fatal,
        &record("written").for_session("cli:abc"),
        &mut || suffixes.next().unwrap_or(3),
    )
    .unwrap();
    assert_eq!(
        read(base.path(), "cli:abc"),
        Some(record("written").for_session("cli:abc"))
    );
    assert_eq!(
        std::fs::read_to_string(crash.join(format!(".{fatal}.0000000000000001.tmp"))).unwrap(),
        "squatted",
        "the squatted name is left alone"
    );
    // Every attempt squatted: the write fails, and says so.
    let error = dir
        .write_with(&fatal, &record("never"), &mut || 1)
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    // The production names are not predictable: two writes, two suffixes,
    // and a squat on any fixed name (here the zero one, and the old
    // `<pid>.<serial>` scheme's first) does not stop a fatal record.
    assert_ne!(unpredictable(), unpredictable());
    let pid = std::process::id();
    for squatted in [
        format!(".{fatal}.0000000000000000.tmp"),
        format!(".{fatal}.{pid}.0.tmp"),
    ] {
        std::fs::write(crash.join(squatted), "squatted").unwrap();
    }
    armed(base.path(), "cli:abc").record_fatal(&record("despite"), "panic", 0);
    assert_eq!(
        read(base.path(), "cli:abc"),
        Some(record("despite").for_session("cli:abc"))
    );
}

/// #2192 review round 5 (L3): every entry read counts toward the bound,
/// names that are not UTF-8 included — a co-tenant cannot make a listing
/// read without end by filling the directory with them.
#[test]
fn entries_that_are_not_utf8_count_toward_the_bound() {
    use std::os::unix::ffi::OsStrExt;
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    let crash = crash_dir(base.path());
    for n in 0..20u8 {
        let bytes = [b'p', b'.', 0xff, b'a' + n];
        std::fs::write(crash.join(std::ffi::OsStr::from_bytes(&bytes)), "").unwrap();
    }
    std::fs::write(crash.join("p.real"), "").unwrap();
    // 23 entries (`.`, `..`, 21 files): five examined cannot reach the end.
    let bounded = dir.names_with_prefix_within("p.", 5);
    assert!(!bounded.complete, "{bounded:?}");
    let all = dir.names_with_prefix_within("p.", 23);
    assert_eq!(
        (all.names, all.complete),
        (vec!["p.real".to_string()], true)
    );
}

/// #2192 review round 5 (L3): a directory holding exactly as many entries
/// as the bound was listed to its end, and says so.
#[test]
fn a_listing_of_exactly_the_bound_is_complete() {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    for n in 0..20 {
        std::fs::write(crash_dir(base.path()).join(format!("p.{n}")), "").unwrap();
    }
    // `.`, `..` and twenty files.
    let exact = dir.names_with_prefix_within("p.", 22);
    assert_eq!((exact.names.len(), exact.complete), (20, true));
    let short = dir.names_with_prefix_within("p.", 21);
    assert!(!short.complete, "one entry was never read");
}

/// #2192 review round 5 (nit): a directory planted under the fatal
/// record's name does not stop the fatal record. An empty one is removed
/// (by the write, and by a new run's clear); a non-empty one leaves the
/// record under the fallback name a reader of that process finds.
#[test]
fn a_directory_planted_as_the_fatal_record_does_not_stop_it() {
    let base = tempfile::tempdir().unwrap();
    let target = armed(base.path(), "cli:abc");
    let fatal = crash_dir(base.path()).join(fatal_name("cli:abc"));
    std::fs::create_dir(&fatal).unwrap();
    target.record_fatal(&record("over an empty directory"), "panic", 0);
    assert_eq!(
        read(base.path(), "cli:abc")
            .map(|r| r.panic.message)
            .as_deref(),
        Some("over an empty directory")
    );
    assert!(fatal.is_file());
    std::fs::remove_file(&fatal).unwrap();
    std::fs::create_dir(&fatal).unwrap();
    std::fs::write(fatal.join("kept"), "").unwrap();
    target.record_fatal(&record("beside a full directory"), "panic", 0);
    let read_back = read(base.path(), "cli:abc").unwrap();
    assert_eq!(read_back.panic.message, "beside a full directory");
    assert!(!read_back.provisional, "still the fatal record");
    // A new run's clear removes the fallback, and an empty planted directory.
    std::fs::remove_file(fatal.join("kept")).unwrap();
    SessionRecords::new("cli:abc").clear(&RecordDir::existing(base.path()).unwrap());
    assert_eq!(read(base.path(), "cli:abc"), None);
    assert!(names_in(&crash_dir(base.path())).is_empty());
}

/// #2192 review (F3): names that only start like a process's provisional
/// records — 64 and more of them, sorting first — do not shadow the real
/// one: only `<prefix><pid>.<scope>` in decimal digits is read.
#[test]
fn prefix_squatters_do_not_shadow_a_provisional_record() {
    let base = tempfile::tempdir().unwrap();
    let target = armed(base.path(), "cli:abc");
    target.record_provisional(9, &record("real").provisional());
    let crash = crash_dir(base.path());
    let (me, fatal) = (std::process::id(), fatal_name("cli:abc"));
    let mut squatter = record("squatter").provisional();
    squatter.unix_ms = u64::MAX;
    let text = serde_json::to_string(&squatter).unwrap();
    for n in 0..(MAX_PROVISIONAL_READ + 10) {
        for name in [
            format!("{fatal}.provisional.{me}.0{n}x"),
            format!("{fatal}.provisional.{me}.-{n}"),
            format!("{fatal}.provisional.{me}.0.{n}"),
        ] {
            std::fs::write(crash.join(name), &text).unwrap();
        }
    }
    assert_eq!(
        read(base.path(), "cli:abc")
            .map(|r| r.panic.message)
            .as_deref(),
        Some("real")
    );
    let dir = RecordDir::existing(base.path()).unwrap();
    assert_eq!(
        SessionRecords::new("cli:abc")
            .read(&dir, None)
            .map(|r| r.panic.message)
            .as_deref(),
        Some("real"),
        "with no pid expected, too"
    );
}

#[test]
fn a_provisional_name_is_read_only_in_its_exact_form() {
    assert_eq!(provisional_ids("12", Some(7)), Some((7, 12)));
    assert_eq!(provisional_ids("7.12", None), Some((7, 12)));
    for (rest, writer) in [
        ("", Some(7)),
        ("1x", Some(7)),
        ("+1", Some(7)),
        ("1.2", Some(7)),
        ("7", None),
        ("7.", None),
        (".12", None),
        ("7.1.2", None),
    ] {
        assert_eq!(provisional_ids(rest, writer), None, "{rest:?} {writer:?}");
    }
}

/// #2192 review (F5): what an entry is decides how it is removed — never
/// the platform's error for unlinking a directory.
#[test]
fn a_directory_is_removed_as_one_and_anything_else_as_a_file() {
    assert_eq!(unlink_flags(libc::S_IFDIR | 0o700), libc::AT_REMOVEDIR);
    for mode in [libc::S_IFREG, libc::S_IFLNK, libc::S_IFIFO, libc::S_IFSOCK] {
        assert_eq!(unlink_flags(mode | 0o600), 0, "{mode:o}");
    }
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    let crash = crash_dir(base.path());
    std::fs::create_dir(crash.join("empty")).unwrap();
    std::fs::write(crash.join("file"), "").unwrap();
    std::os::unix::fs::symlink(crash.join("empty"), crash.join("link")).unwrap();
    assert!(dir.is_directory("empty"));
    assert!(!dir.is_directory("link"), "a link is not followed");
    for name in ["empty", "file", "link", "missing"] {
        dir.remove(name).unwrap();
    }
    assert!(crash.join("empty").symlink_metadata().is_err());
    assert!(names_in(&crash).is_empty());
}

/// The `error` events of `key`'s event log under `base`.
fn error_events(base: &Path, key: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(AuditLog::file_path(base, key))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event["event"] == "error")
        .collect()
}

/// #2192 review (F2): a session switch moves the crash line with the crash
/// record: when the event log was the departing session's own, the
/// arriving session's log (same parent) is opened and answered, and the
/// fatal event lands there.
#[test]
fn the_event_log_and_its_crash_line_follow_a_session_switch() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:first")
        .unwrap()
        .with_parent(Some("the-parent".into()));
    let target = Armed::new(base.path(), Some("cli:first"), log.crash_line());
    let followed = target.follow(Some("cli:second")).expect("the log follows");
    target.record_fatal(&record("after the switch"), "panic", 2);
    assert!(error_events(base.path(), "cli:first").is_empty());
    let events = error_events(base.path(), "cli:second");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["session"], "cli:second");
    assert_eq!(events[0]["parent"], "the-parent");
    assert_eq!(
        read(base.path(), "cli:second").unwrap().panic.message,
        "after the switch"
    );
    drop(followed);
    // A switch to a session that leaves nothing behind opens no log.
    assert!(target.follow(None).is_none());
}

#[test]
fn a_log_of_its_own_key_does_not_follow_the_session() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "unkeyed-1-2").unwrap();
    let target = Armed::new(base.path(), Some("cli:first"), log.crash_line());
    assert!(target.follow(Some("cli:second")).is_none());
    target.record_fatal(&record("kept"), "panic", 0);
    assert_eq!(error_events(base.path(), "unkeyed-1-2").len(), 1);
    assert!(error_events(base.path(), "cli:second").is_empty());
}
