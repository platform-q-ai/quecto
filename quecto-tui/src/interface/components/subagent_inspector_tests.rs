//! Behavioural tests for the sub-agent inspector component (#795).

use super::*;

fn rows() -> Vec<AgentRow> {
    vec![
        AgentRow {
            agent_id: "worker".into(),
            label: "● worker  wf active 1/3".into(),
        },
        AgentRow {
            agent_id: "other".into(),
            label: "● other  wf active 2/3".into(),
        },
    ]
}

fn detail(id: &str) -> AgentDetail {
    AgentDetail {
        agent_id: id.into(),
        status: "running".into(),
        elapsed_secs: 12,
        workflow: Some(("active".into(), 1, 3)),
        output: vec!["hello from the agent".into()],
    }
}

#[test]
fn starts_focused_on_list_and_first_agent() {
    let insp = SubagentInspector::new(rows());
    assert_eq!(insp.focus(), InspectorFocus::List);
    assert_eq!(insp.selected_agent_id().as_deref(), Some("worker"));
}

#[test]
fn down_changes_selection_and_reports_change() {
    let mut insp = SubagentInspector::new(rows());
    let action = insp.handle_key(&Key::Down);
    assert_eq!(action, InspectorAction::SelectionChanged);
    assert_eq!(insp.selected_agent_id().as_deref(), Some("other"));
}

#[test]
fn focus_state_machine_list_detail_close() {
    let mut insp = SubagentInspector::new(rows());
    // Enter: List -> Detail.
    assert_eq!(insp.handle_key(&Key::Enter), InspectorAction::Consumed);
    assert_eq!(insp.focus(), InspectorFocus::Detail);
    // Esc: Detail -> List.
    assert_eq!(insp.handle_key(&Key::Escape), InspectorAction::Consumed);
    assert_eq!(insp.focus(), InspectorFocus::List);
    // Esc: List -> Close.
    assert_eq!(insp.handle_key(&Key::Escape), InspectorAction::Close);
}

#[test]
fn scroll_keys_only_apply_in_detail_focus() {
    let mut insp = SubagentInspector::new(rows());
    let long: Vec<String> = (0..50).map(|i| format!("line {i}")).collect();
    insp.set_tail("worker", long);
    // In list focus, PageDown does not scroll the detail.
    insp.handle_key(&Key::PageDown);
    let d = detail("worker");
    let before = insp.render(Some(&d), 100, 20);
    // Focus detail, then PageDown scrolls.
    insp.handle_key(&Key::Enter);
    insp.handle_key(&Key::PageDown);
    let after = insp.render(Some(&d), 100, 20);
    assert_ne!(
        before, after,
        "PageDown in detail focus should scroll output"
    );
}

#[test]
fn renders_master_detail_with_status_header() {
    let mut insp = SubagentInspector::new(rows());
    let d = detail("worker");
    let frame = insp.render(Some(&d), 100, 20).join("\n");
    let plain = crate::interface::ansi::strip_ansi(&frame);
    assert!(plain.contains("Sub-agents"), "title:\n{plain}");
    assert!(plain.contains("worker"), "left list + header:\n{plain}");
    assert!(plain.contains("1/3"), "workflow status header:\n{plain}");
    assert!(
        plain.contains("hello from the agent"),
        "output body:\n{plain}"
    );
}

#[test]
fn render_is_idempotent() {
    let mut insp = SubagentInspector::new(rows());
    let d = detail("worker");
    let a = insp.render(Some(&d), 100, 20);
    let b = insp.render(Some(&d), 100, 20);
    assert_eq!(a, b, "compose must be render-idempotent (no flash)");
    assert_eq!(a.len(), 20, "frame must fill exactly the height");
}

#[test]
fn cached_tail_used_when_detail_output_empty() {
    let mut insp = SubagentInspector::new(rows());
    insp.set_tail("worker", vec!["cached output".into()]);
    assert!(insp.has_tail("worker"));
    let mut d = detail("worker");
    d.output.clear();
    let frame = crate::interface::ansi::strip_ansi(&insp.render(Some(&d), 100, 20).join("\n"));
    assert!(
        frame.contains("cached output"),
        "should fall back to cache:\n{frame}"
    );
}
