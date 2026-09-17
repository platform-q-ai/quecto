//! The reload gate's state machine (ADR-0002 AC6/AC7), formerly the
//! `runtime_reload.feature` scenarios (#2017): a unit of infrastructure,
//! not an entry-point behaviour.
use super::*;

fn file_with(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

/// Rewrite `path`, guaranteeing the mtime advances even on coarse-timestamp
/// filesystems (the same-mtime same-length rewrite is the tolerated edge).
fn rewrite_with_mtime_advance(path: &std::path::Path, content: &str) -> SystemTime {
    let before = fs::metadata(path).and_then(|m| m.modified()).unwrap();
    loop {
        fs::write(path, content).unwrap();
        let after = fs::metadata(path).and_then(|m| m.modified()).unwrap();
        if after != before {
            return after;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
}

#[test]
fn unseeded_source_reports_changed_then_stat_only_unchanged() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"first").unwrap();
    let mut source = ReloadSource::new(tmp.path());
    assert_eq!(source.changed(), SourceChange::Changed);
    let observed = source.last_mtime();
    assert!(observed.is_some());
    assert_eq!(source.changed(), SourceChange::UnchangedNoRead);
    assert_eq!(source.last_mtime(), observed);
}

#[test]
fn seeded_source_with_no_edit_reports_unchanged_without_a_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = ReloadSource::new(file_with(&dir, "source.txt", "v1"));
    source.seed();
    assert_eq!(source.changed(), SourceChange::UnchangedNoRead);
}

#[test]
fn seeded_source_detects_a_content_change_once_mtime_moves_and_hash_differs() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_with(&dir, "source.txt", "v1");
    let mut source = ReloadSource::new(&path);
    source.seed();
    rewrite_with_mtime_advance(&path, "v2");
    assert_eq!(source.changed(), SourceChange::Changed);
}

/// AC6b: a touch-only rewrite reports unchanged AND advances the mtime
/// cache, so the following probe is a stat-only no-op.
#[test]
fn touched_but_identical_file_reports_unchanged_and_advances_the_mtime_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_with(&dir, "source.txt", "v1");
    let mut source = ReloadSource::new(&path);
    source.seed();
    let before = source.last_mtime().unwrap();
    let touched = rewrite_with_mtime_advance(&path, "v1");
    assert_eq!(source.changed(), SourceChange::Unchanged);
    assert_ne!(source.last_mtime().unwrap(), before);
    assert_eq!(source.last_mtime().unwrap(), touched);
    assert_eq!(source.changed(), SourceChange::UnchangedNoRead);
}

/// AC7: a missing file is fail-safe — reported, cache untouched, no panic.
#[test]
fn missing_file_reports_missing_or_unreadable_and_keeps_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_with(&dir, "source.txt", "v1");
    let mut source = ReloadSource::new(&path);
    source.seed();
    let seeded = source.last_mtime();
    assert!(seeded.is_some());
    std::fs::remove_file(&path).unwrap();
    assert_eq!(source.changed(), SourceChange::MissingOrUnreadable);
    assert_eq!(source.last_mtime(), seeded);
}

#[test]
fn gate_with_no_edit_reports_no_change_after_seeding() {
    let dir = tempfile::tempdir().unwrap();
    let mut gate = RuntimeReload::new(vec![ReloadSource::new(file_with(&dir, "a", "v1"))]);
    gate.seed();
    assert!(!gate.sources_changed());
}

#[test]
fn gate_reports_a_change_once_then_not_again_until_the_next_edit() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_with(&dir, "a", "v1");
    let mut gate = RuntimeReload::new(vec![ReloadSource::new(&path)]);
    gate.seed();
    rewrite_with_mtime_advance(&path, "broken");
    assert!(gate.sources_changed());
    // The fingerprint advanced with the probe: a rebuild failure of the
    // edited file is not retried every turn until the file changes again.
    assert!(!gate.sources_changed());
    rewrite_with_mtime_advance(&path, "v2");
    assert!(
        gate.sources_changed(),
        "a later fix is picked up (recovery)"
    );
}

#[test]
fn gate_watching_two_sources_reports_a_change_when_either_is_edited() {
    let dir = tempfile::tempdir().unwrap();
    let a = file_with(&dir, "a.json", "a1");
    let b = file_with(&dir, "b.json", "b1");
    let mut gate = RuntimeReload::new(vec![ReloadSource::new(&a), ReloadSource::new(&b)]);
    gate.seed();
    assert!(!gate.sources_changed());
    rewrite_with_mtime_advance(&b, "b2");
    assert!(gate.sources_changed());
    assert!(!gate.sources_changed());
    rewrite_with_mtime_advance(&a, "a2");
    assert!(gate.sources_changed());
}

/// Re-seeding after an edit (what a rebuild does before reading) consumes
/// the pending change without reporting it.
#[test]
fn reseeding_after_an_edit_consumes_the_pending_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_with(&dir, "a", "v1");
    let mut gate = RuntimeReload::new(vec![ReloadSource::new(&path)]);
    gate.seed();
    rewrite_with_mtime_advance(&path, "v2");
    gate.seed();
    assert!(!gate.sources_changed());
}

#[test]
fn missing_source_is_fail_safe_unchanged_for_the_gate() {
    let missing = tempfile::tempdir().unwrap().path().join("missing.json");
    let mut gate = RuntimeReload::new(vec![ReloadSource::new(missing)]);
    gate.seed();
    assert!(!gate.sources_changed());
}

#[test]
fn hash_changes_with_content() {
    assert_ne!(hash_bytes(b"a"), hash_bytes(b"b"));
}
