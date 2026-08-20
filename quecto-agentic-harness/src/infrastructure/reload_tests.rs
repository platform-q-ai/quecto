use super::*;

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
fn seeded_source_detects_content_change_and_runtime_reload_keeps_last_good_on_failure() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"v1").unwrap();
    let mut reload = RuntimeReload::new(vec![ReloadSource::new(tmp.path())]);
    reload.seed("initial".to_string());
    assert_eq!(
        reload.poll(|| Ok("should not run".to_string())),
        ReloadResult::Unchanged
    );
    std::thread::sleep(std::time::Duration::from_millis(5));
    std::fs::write(tmp.path(), b"v2-longer").unwrap();
    assert_eq!(
        reload.poll(|| Err("bad config".into())),
        ReloadResult::Unchanged
    );
    assert_eq!(reload.last_good().map(String::as_str), Some("initial"));
    assert_eq!(
        reload.poll_forced(|| Ok("forced".to_string())),
        ReloadResult::Reloaded("forced".to_string())
    );
    assert_eq!(reload.last_good().map(String::as_str), Some("forced"));
}

#[test]
fn record_reloaded_updates_last_good_for_multiple_value_types() {
    let mut numbers = RuntimeReload::<i32>::new(vec![]);
    assert_eq!(numbers.record_reloaded(7), ReloadResult::Reloaded(7));
    assert_eq!(numbers.last_good(), Some(&7));

    let mut unsigned = RuntimeReload::<u32>::new(vec![]);
    assert_eq!(unsigned.record_reloaded(9), ReloadResult::Reloaded(9));
    assert_eq!(unsigned.last_good(), Some(&9));
}

#[test]
fn missing_source_is_fail_safe_unchanged_for_runtime_reload() {
    let missing = tempfile::tempdir().unwrap().path().join("missing.json");
    let mut source = ReloadSource::new(&missing);
    assert_eq!(source.changed(), SourceChange::MissingOrUnreadable);
    let mut reload = RuntimeReload::new(vec![ReloadSource::new(missing)]);
    reload.seed(7);
    assert!(!reload.sources_changed());
    assert_eq!(reload.poll(|| Ok(8)), ReloadResult::Unchanged);
    assert_eq!(reload.last_good(), Some(&7));
}

#[test]
fn forced_reload_result_preserves_error_without_replacing_last_good() {
    let mut reload: RuntimeReload<u32> = RuntimeReload::new(vec![]);
    reload.seed(3);
    let err = reload
        .poll_forced_result(|| Err("syntax error".into()))
        .unwrap_err();
    assert_eq!(err, "syntax error");
    assert_eq!(reload.last_good(), Some(&3));
}

#[test]
fn hash_changes_with_content() {
    assert_ne!(hash_bytes(b"a"), hash_bytes(b"b"));
}
