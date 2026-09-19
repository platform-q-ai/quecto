use crate::protocol::session_payloads::ResumeSessionSummary;
use crate::sessions::resume_rows::{ResumeRows, SESSION_ROW_PREFIX};

fn summary(key: &str, at: Option<u64>, eligible: bool, dir: Option<&str>) -> ResumeSessionSummary {
    ResumeSessionSummary {
        key: key.into(),
        title: format!("title {key}"),
        message_count: 3,
        updated_unix_secs: at,
        execution_dir: dir.map(str::to_string),
        resume_eligible: eligible,
        home_version: at.map(|at| format!("h1-{at:016x}")),
    }
}

#[test]
fn rows_are_newest_first_with_stable_ids_and_their_listed_versions() {
    let rows = ResumeRows::project(
        vec![
            summary("chat-old", Some(10), true, Some("/repo")),
            summary("chat-undated", None, false, None),
            summary("chat-new", Some(30), false, Some("/elsewhere")),
        ],
        true,
        |secs| format!("t{secs}"),
    );
    assert!(rows.empty_hint.is_none());
    let ids: Vec<_> = rows.items.iter().map(|i| i.value.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "session:chat-new",
            "session:chat-old",
            "session:chat-undated"
        ]
    );
    assert!(SESSION_ROW_PREFIX == "session:");
    // Every row that was listed with a version carries it, eligible or not.
    assert_eq!(
        rows.home_versions.get("chat-old").map(String::as_str),
        Some("h1-000000000000000a")
    );
    assert!(rows.home_versions.contains_key("chat-new"));
    assert!(!rows.home_versions.contains_key("chat-undated"));
    let old = rows.items[1].description.as_deref().unwrap();
    assert!(old.contains("/repo · t10 (3 msgs) · Resume"), "{old}");
    let new = rows.items[0].description.as_deref().unwrap();
    assert!(
        new.contains("/elsewhere") && new.ends_with("Saved in another folder — Enter for options"),
        "{new}"
    );
    let undated = rows.items[2].description.as_deref().unwrap();
    assert!(
        undated.starts_with("No folder on record · unknown time")
            && undated.ends_with(" · Enter for options"),
        "{undated}"
    );
}

#[test]
fn empty_hints_distinguish_no_records_from_no_resumable_rows() {
    let none = ResumeRows::project(Vec::new(), false, |_| String::new());
    assert_eq!(none.empty_hint, Some("No persisted sessions found."));
    assert!(none.items.is_empty() && none.home_versions.is_empty());
    let filtered = ResumeRows::project(Vec::new(), true, |_| String::new());
    assert_eq!(
        filtered.empty_hint,
        Some("No resumable CLI sessions found.")
    );
}
