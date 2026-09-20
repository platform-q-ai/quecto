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
        unscoped: false,
        repository_label: None,
        matched: Vec::new(),
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
        new.contains("/elsewhere") && new.ends_with("In another folder"),
        "{new}"
    );
    let undated = rows.items[2].description.as_deref().unwrap();
    assert!(
        undated.starts_with("No folder on record · unknown time")
            && undated.ends_with(" · Can't be resumed"),
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

#[test]
fn searched_rows_keep_the_harness_order_and_name_key_folder_and_unscoped_state() {
    let mut legacy = summary("cli:legacy", Some(99), false, None);
    legacy.unscoped = true;
    let rows = ResumeRows::project_searched(
        vec![
            summary("chat-best", Some(1), true, Some("/work/alpha")),
            legacy,
            summary("chat-unknown", Some(50), false, None),
        ],
        |secs| format!("t{secs}"),
    );
    let ids: Vec<_> = rows.items.iter().map(|i| i.value.as_str()).collect();
    assert_eq!(
        ids,
        [
            "session:chat-best",
            "session:cli:legacy",
            "session:chat-unknown"
        ]
    );
    let described: Vec<_> = rows
        .items
        .iter()
        .map(|i| i.description.as_deref().unwrap())
        .collect();
    assert_eq!(
        described[0],
        "/work/alpha · t1 (3 msgs) · Resume · ID chat-best"
    );
    assert_eq!(
        described[1],
        "No folder recorded (older session) · t99 (3 msgs) · Can't be resumed · ID cli:legacy"
    );
    assert!(
        described[2].starts_with("No folder on record · t50"),
        "{}",
        described[2]
    );
    assert_eq!(rows.home_versions.len(), 3);
    assert_eq!(
        rows.titles.get("cli:legacy").map(String::as_str),
        Some("title cli:legacy")
    );
    let none = ResumeRows::project_searched(Vec::new(), |_| String::new());
    assert_eq!(none.empty_hint, Some("No sessions match."));
    // A listing names an unscoped session the same way, without the key.
    let mut listed = summary("cli:legacy", Some(9), false, None);
    listed.unscoped = true;
    let rows = ResumeRows::project(vec![listed], true, |secs| format!("t{secs}"));
    assert_eq!(
        rows.items[0].description.as_deref(),
        Some("No folder recorded (older session) · t9 (3 msgs) · Can't be resumed")
    );
}

#[test]
fn a_rows_untrusted_text_is_made_safe_for_the_terminal() {
    let mut hostile = summary(
        "cli:e\u{202e}vil\u{200b}",
        Some(1),
        false,
        Some("/w/\u{1b}]0;x\u{7}dir\u{202e}"),
    );
    hostile.title = "evil\u{1b}[2J \u{202e}title\u{200b}".into();
    let rows = ResumeRows::project_searched(vec![hostile], |_| "now".into());
    let item = &rows.items[0];
    let shown = format!("{}{}", item.label, item.description.as_deref().unwrap());
    for hidden in ['\u{1b}', '\u{7}', '\u{202e}', '\u{200b}'] {
        assert!(!shown.contains(hidden), "{hidden:?} in {shown:?}");
    }
    assert!(item.label.contains("evil") && item.label.contains("title"));
    // The identity sent on selection is the key as the harness gave it.
    assert_eq!(item.value, "session:cli:e\u{202e}vil\u{200b}");
}

/// R1-T9 / R1-T10 / R1-T14: a searched row says why it matched and which
/// repository it belongs to, keeps its action and ID on screen whatever the
/// folder's length, and speaks the picker's own vocabulary.
#[test]
fn a_searched_row_names_its_repository_and_match_and_a_long_folder_keeps_its_tail() {
    let mut worktree = summary("cli:wt", Some(1), false, Some("/work/wt/fix-2010"));
    worktree.repository_label = Some("quecto\u{202e}".into());
    worktree.matched = vec!["repository".into(), "path".into()];
    let long = format!("/very/{}/deep/tail-folder", "long-segment/".repeat(300));
    let mut deep = summary("cli:deep", Some(2), true, Some(&long));
    deep.matched = vec!["title".into()];
    let rows = ResumeRows::project_searched(vec![worktree, deep], |secs| format!("t{secs}"));
    assert_eq!(
        rows.items[0].description.as_deref(),
        Some(
            "/work/wt/fix-2010 · t1 (3 msgs) · In another folder · ID cli:wt · repo quecto · matched: repository, path"
        )
    );
    let deep = rows.items[1].description.as_deref().unwrap();
    assert!(
        deep.chars().count() < 160,
        "{}: {deep}",
        deep.chars().count()
    );
    assert!(
        deep.starts_with("/very/long-segment/") && deep.contains("…"),
        "{deep}"
    );
    assert!(
        deep.contains("/deep/tail-folder · t2 (3 msgs) · Resume · ID cli:deep · matched: title"),
        "{deep}"
    );
    // An everything-query matched no field: no marker, and no label → no repo.
    let plain = ResumeRows::project_searched(vec![summary("k", Some(1), true, Some("/w"))], |_| {
        "now".into()
    });
    assert_eq!(
        plain.items[0].description.as_deref(),
        Some("/w · now (3 msgs) · Resume · ID k")
    );
    // A listing row elides the same way and never shows search-only fields.
    let mut listed = summary("cli:deep", Some(2), true, Some(&long));
    listed.repository_label = Some("ignored".into());
    let rows = ResumeRows::project(vec![listed], true, |secs| format!("t{secs}"));
    let listed = rows.items[0].description.as_deref().unwrap();
    assert!(
        listed.ends_with("/deep/tail-folder · t2 (3 msgs) · Resume"),
        "{listed}"
    );
}

#[test]
fn a_session_without_a_folder_is_worded_for_a_user() {
    let mut legacy = summary("cli:legacy", Some(9), false, None);
    legacy.unscoped = true;
    let rows = ResumeRows::project(vec![legacy], true, |secs| format!("t{secs}"));
    assert_eq!(
        rows.items[0].description.as_deref(),
        Some("No folder recorded (older session) · t9 (3 msgs) · Can't be resumed")
    );
}
