use super::*;

fn make_tracked(id: &str, status: &str) -> (String, super::TrackedSubagent) {
    (
        id.to_string(),
        super::TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
            agent_id: id.to_string(),
            status: status.to_string(),
            last_tool: None,
            last_error: None,
            pid: 0,
            parent_id: None,
            workflow: None,
        }),
    )
}

#[test]
fn gc_removes_expired_exited_subagent() {
    let mut map = std::collections::BTreeMap::new();
    let (id, mut entry) = make_tracked("w1", "exited");
    // Backdate the exited_at to 10 seconds ago.
    entry.exited_at = Some(tokio::time::Instant::now() - Duration::from_secs(10));
    map.insert(id, entry);

    let removed = super::gc_exited_subagents(
        &mut map,
        tokio::time::Instant::now(),
        Duration::from_secs(5),
    );
    assert!(removed, "should have removed expired entry");
    assert!(map.is_empty());
}

#[test]
fn gc_keeps_recent_exited_subagent() {
    let mut map = std::collections::BTreeMap::new();
    let (id, entry) = make_tracked("w1", "exited");
    // exited_at is just now — within grace period.
    map.insert(id, entry);

    let removed = super::gc_exited_subagents(
        &mut map,
        tokio::time::Instant::now(),
        Duration::from_secs(5),
    );
    assert!(!removed, "should not remove recent exit");
    assert_eq!(map.len(), 1);
}

#[test]
fn gc_keeps_running_subagent() {
    let mut map = std::collections::BTreeMap::new();
    let (id, entry) = make_tracked("w1", "running");
    map.insert(id, entry);

    let removed = super::gc_exited_subagents(
        &mut map,
        tokio::time::Instant::now(),
        Duration::from_secs(5),
    );
    assert!(!removed, "should not remove running subagent");
    assert_eq!(map.len(), 1);
}

#[test]
fn gc_mixed_removes_only_expired() {
    let mut map = std::collections::BTreeMap::new();

    let (id1, entry1) = make_tracked("active", "running");
    map.insert(id1, entry1);

    let (id2, mut entry2) = make_tracked("old-exit", "exited");
    entry2.exited_at = Some(tokio::time::Instant::now() - Duration::from_secs(10));
    map.insert(id2, entry2);

    let (id3, entry3) = make_tracked("recent-exit", "exited");
    map.insert(id3, entry3);

    let removed = super::gc_exited_subagents(
        &mut map,
        tokio::time::Instant::now(),
        Duration::from_secs(5),
    );
    assert!(removed, "should have removed old-exit");
    assert_eq!(map.len(), 2);
    assert!(map.contains_key("active"));
    assert!(map.contains_key("recent-exit"));
    assert!(!map.contains_key("old-exit"));
}

#[test]
fn tracked_subagent_new_sets_exited_at_for_exited() {
    let entry = super::TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
        agent_id: "w1".into(),
        status: "exited".into(),
        last_tool: None,
        last_error: None,
        pid: 0,
        parent_id: None,
        workflow: None,
    });
    assert!(entry.exited_at.is_some());
}

#[test]
fn tracked_subagent_new_no_exited_at_for_running() {
    let entry = super::TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
        agent_id: "w1".into(),
        status: "running".into(),
        last_tool: None,
        last_error: None,
        pid: 0,
        parent_id: None,
        workflow: None,
    });
    assert!(entry.exited_at.is_none());
}

#[test]
fn tracked_subagent_update_sets_exited_at_on_transition() {
    let mut entry = super::TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
        agent_id: "w1".into(),
        status: "running".into(),
        last_tool: None,
        last_error: None,
        pid: 0,
        parent_id: None,
        workflow: None,
    });
    assert!(entry.exited_at.is_none());

    entry.update_info(crate::infrastructure::client::SubagentInfoEvent {
        agent_id: "w1".into(),
        status: "exited".into(),
        last_tool: None,
        last_error: None,
        pid: 0,
        parent_id: None,
        workflow: None,
    });
    assert!(entry.exited_at.is_some());
}

#[test]
fn tracked_subagent_update_clears_exited_at_on_revival() {
    let mut entry = super::TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
        agent_id: "w1".into(),
        status: "exited".into(),
        last_tool: None,
        last_error: None,
        pid: 0,
        parent_id: None,
        workflow: None,
    });
    assert!(entry.exited_at.is_some());

    entry.update_info(crate::infrastructure::client::SubagentInfoEvent {
        agent_id: "w1".into(),
        status: "running".into(),
        last_tool: None,
        last_error: None,
        pid: 0,
        parent_id: None,
        workflow: None,
    });
    assert!(entry.exited_at.is_none());
}

#[test]
fn exited_subagent_grace_is_reasonable() {
    assert!(super::EXITED_SUBAGENT_GRACE.as_secs() >= 2);
    assert!(super::EXITED_SUBAGENT_GRACE.as_secs() <= 30);
}

// ── Mouse highlight tests (#546) ─────────────────────────────────

#[test]
fn highlight_plain_text_full_line() {
    let result = super::apply_line_highlight("hello world", 0, 11);
    assert!(result.contains("\x1b[7m"), "should contain reverse-on");
    assert!(result.contains("\x1b[27m"), "should contain reverse-off");
    assert!(result.contains("hello world"));
}

#[test]
fn highlight_plain_text_partial() {
    let result = super::apply_line_highlight("hello world", 2, 7);
    // Before highlight: "he"
    // Highlighted: "llo w"
    // After highlight: "orld"
    assert!(result.contains("\x1b[7m"));
    assert!(result.contains("\x1b[27m"));
}

#[test]
fn highlight_noop_when_start_equals_end() {
    let result = super::apply_line_highlight("hello", 3, 3);
    assert_eq!(result, "hello");
}

#[test]
fn highlight_noop_when_start_exceeds_end() {
    let result = super::apply_line_highlight("hello", 5, 2);
    assert_eq!(result, "hello");
}

#[test]
fn highlight_with_ansi_escapes() {
    let line = "\x1b[32mgreen\x1b[0m text";
    let result = super::apply_line_highlight(line, 0, 5);
    // Should highlight "green" (5 visible chars)
    assert!(result.contains("\x1b[7m"));
    assert!(result.contains("\x1b[27m"));
    // ANSI codes should be preserved
    assert!(result.contains("\x1b[32m"));
}

#[test]
fn highlight_closes_at_line_end() {
    let result = super::apply_line_highlight("abc", 1, 100);
    // Start at col 1, end beyond line length
    assert!(result.contains("\x1b[7m"));
    assert!(result.contains("\x1b[27m"), "must close highlight at end");
}

#[test]
fn selection_range_normalizes_forward() {
    let sel = super::TextSelection {
        start: super::SelectionAnchor { col: 5, row: 2 },
        end: super::SelectionAnchor { col: 10, row: 4 },
    };
    let (sr, sc, er, ec) = super::selection_range(&sel);
    assert_eq!((sr, sc, er, ec), (2, 5, 4, 10));
}

#[test]
fn selection_range_normalizes_backward() {
    let sel = super::TextSelection {
        start: super::SelectionAnchor { col: 10, row: 4 },
        end: super::SelectionAnchor { col: 5, row: 2 },
    };
    let (sr, sc, er, ec) = super::selection_range(&sel);
    assert_eq!((sr, sc, er, ec), (2, 5, 4, 10));
}

#[test]
fn selection_range_same_row_normalizes() {
    let sel = super::TextSelection {
        start: super::SelectionAnchor { col: 10, row: 3 },
        end: super::SelectionAnchor { col: 2, row: 3 },
    };
    let (sr, sc, er, ec) = super::selection_range(&sel);
    assert_eq!((sr, sc, er, ec), (3, 2, 3, 10));
}
