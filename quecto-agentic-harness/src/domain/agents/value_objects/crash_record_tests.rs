use super::*;

#[test]
fn texts_are_cut_on_a_character_boundary() {
    let long = "’".repeat(MAX_CRASH_TEXT_BYTES);
    let report = PanicReport::new(&long, Some(&long));
    for text in [&report.message, report.location.as_ref().unwrap()] {
        assert!(text.len() <= MAX_CRASH_TEXT_BYTES);
        assert!(text.len() > MAX_CRASH_TEXT_BYTES - 3);
        assert!(text.chars().all(|c| c == '’'));
    }
}

#[test]
fn a_bare_record_serializes_only_what_is_known() {
    let record = CrashRecord::new(PanicReport::new("boom", None), 3, 4);
    assert_eq!(
        serde_json::to_string(&record).unwrap(),
        r#"{"message":"boom","pid":3,"unix_ms":4}"#
    );
}

#[test]
fn a_full_record_round_trips() {
    let record = CrashRecord::new(PanicReport::new("second", Some("a.rs:1:2")), 7, 8)
        .in_call("edit")
        .running(vec!["edit".into(), "grep".into()])
        .after(Some(PanicReport::new("first", Some("b.rs:3:4"))))
        .provisional()
        .for_session("telegram:123");
    let json = serde_json::to_string(&record).unwrap();
    assert!(json.contains(r#""provisional":true"#), "{json}");
    let back: CrashRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(back, record);
}

#[test]
fn at_most_the_first_running_calls_are_named() {
    let calls = (0..MAX_RUNNING_CALLS + 5)
        .map(|i| format!("t{i}"))
        .collect();
    let record = CrashRecord::new(PanicReport::new("m", None), 1, 1).running(calls);
    assert_eq!(record.running.len(), MAX_RUNNING_CALLS);
    assert_eq!(record.running[0], "t0");
}

/// #2192 review: a record names its session exactly, and only that key is
/// its own — not one that sanitizes to the same file name.
#[test]
fn a_record_is_for_exactly_the_session_it_names() {
    let bare = CrashRecord::new(PanicReport::new("m", None), 1, 1);
    assert!(!bare.is_for_session("telegram:123"), "a record naming none");
    let named = bare.for_session("telegram:123");
    assert!(named.is_for_session("telegram:123"));
    assert!(!named.is_for_session("telegram_123"));
    assert!(!named.is_for_session(""));
    let json = serde_json::to_string(&named).unwrap();
    assert!(json.contains(r#""session":"telegram:123""#), "{json}");
    let bounded = named.clone().bounded();
    assert_eq!(bounded.session, named.session, "bounding keeps the key");
}
