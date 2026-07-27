use super::tui_harness::*;
use crate::protocol::client::Event;

#[tokio::test]
async fn read_only_subagent_shows_observer_marker_without_shifting_rows() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent_readonly("reviewer", "running", Some(("active", 1, 3)), None),
        subagent("worker", "running", Some(("active", 1, 3))),
    ]));

    let panel = h.left_panel();
    let reviewer = panel
        .lines()
        .find(|line| line.contains("reviewer"))
        .unwrap_or_else(|| panic!("reviewer row not found:\n{panel}"));
    let worker = panel
        .lines()
        .find(|line| line.contains("worker"))
        .unwrap_or_else(|| panic!("worker row not found:\n{panel}"));

    assert!(
        reviewer.contains(crate::interface::theme::OBSERVER_GLYPH),
        "read-only sub-agent row must show an observer marker:\n{panel}"
    );
    assert!(
        !worker.contains(crate::interface::theme::OBSERVER_GLYPH),
        "read-write sub-agent row must not show an observer marker:\n{panel}"
    );
    let marker_col = reviewer
        .find(crate::interface::theme::OBSERVER_GLYPH)
        .expect("reviewer row should include the observer marker");
    let reviewer_name_end = reviewer.find("reviewer").unwrap() + "reviewer".len();
    assert_eq!(
        &reviewer[reviewer_name_end..marker_col],
        " ",
        "observer marker should appear in the next cell after the read-only sub-agent name:\n{panel}"
    );
    let timer_col = |row: &str| {
        row.rfind("0:")
            .map(|i| unicode_width::UnicodeWidthStr::width(&row[..i]))
    };
    assert_eq!(
        timer_col(reviewer),
        timer_col(worker),
        "observer marker must not shift the timer/status column between rows:\n{panel}"
    );
    assert_eq!(
        panel
            .lines()
            .map(unicode_width::UnicodeWidthStr::width)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1,
        "left panel rows must remain column-aligned after adding the observer marker:\n{panel}"
    );
}

#[tokio::test]
async fn observer_marker_disappears_when_read_only_subagent_leaves() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent_readonly("reviewer", "running", Some(("active", 1, 3)), None),
        subagent("worker", "running", Some(("active", 1, 3))),
    ]));
    assert!(
        h.left_panel()
            .contains(crate::interface::theme::OBSERVER_GLYPH)
    );

    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 1, 3)),
    )]));

    let panel = h.left_panel();
    assert!(
        !panel.contains(crate::interface::theme::OBSERVER_GLYPH),
        "observer marker must leave the panel with the read-only sub-agent:\n{panel}"
    );
    assert!(
        !panel.contains("reviewer"),
        "the read-only sub-agent row should leave the panel:\n{panel}"
    );
    assert!(
        panel.contains("worker"),
        "the read-write sub-agent row should remain visible after the read-only sub-agent leaves:\n{panel}"
    );
}
