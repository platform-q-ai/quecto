//! Tests for `app_subagents.rs` — the `update_subagent_bar` merge logic
//! and `tick_subagent_animation` methods (issue #729).
//!
//! These drive the real `App` via the headless render harness (no TTY,
//! drained socket) to exercise the subagent bar lifecycle.

use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn info(id: &str, status: &str) -> crate::infrastructure::client::SubagentInfoEvent {
    crate::infrastructure::client::SubagentInfoEvent {
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        pid: 0,
        socket_path: None,
        parent_id: None,
        workflow: None,
    }
}

fn info_with_workflow(
    id: &str,
    status: &str,
    mode: &str,
    done: u32,
    total: u32,
) -> crate::infrastructure::client::SubagentInfoEvent {
    crate::infrastructure::client::SubagentInfoEvent {
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        pid: 0,
        socket_path: None,
        parent_id: None,
        workflow: Some(crate::infrastructure::client::SubagentWorkflow {
            mode: mode.to_string(),
            steps_completed: done,
            steps_total: total,
        }),
    }
}

// ── update_subagent_bar: basic merge ──────────────────────────────────

#[tokio::test]
async fn update_subagent_bar_inserts_new_agents() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    assert_eq!(a.subagent_local.len(), 1);
    assert!(a.subagent_local.contains_key("w1"));
}

#[tokio::test]
async fn update_subagent_bar_updates_existing_status() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    a.update_subagent_bar(vec![info("w1", "idle")]);
    assert_eq!(a.subagent_local["w1"].info.status, "idle");
}

#[tokio::test]
async fn update_subagent_bar_preserves_started_at_on_update() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    let original_started = a.subagent_local["w1"].started_at;
    a.update_subagent_bar(vec![info("w1", "idle")]);
    assert_eq!(
        a.subagent_local["w1"].started_at, original_started,
        "started_at should be preserved across updates"
    );
}

#[tokio::test]
async fn update_subagent_bar_removes_absent_agents_without_grace() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running"), info("w2", "running")]);
    // Server push drops w2 (still running, not exited) → removed immediately.
    a.update_subagent_bar(vec![info("w1", "running")]);
    assert!(a.subagent_local.contains_key("w1"));
    assert!(
        !a.subagent_local.contains_key("w2"),
        "absent running agent should be removed"
    );
}

#[tokio::test]
async fn update_subagent_bar_preserves_exited_within_grace() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running"), info("w2", "running")]);
    // Mark w2 as exited.
    a.update_subagent_bar(vec![info("w1", "running"), info("w2", "exited")]);
    // Now a server push that drops w2 — w2 should survive the grace period.
    a.update_subagent_bar(vec![info("w1", "running")]);
    assert!(
        a.subagent_local.contains_key("w2"),
        "exited agent within grace period should be preserved"
    );
}

#[tokio::test]
async fn update_subagent_bar_replaces_all_entries() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running"), info("w2", "running")]);
    a.update_subagent_bar(vec![info("w3", "running")]);
    assert_eq!(a.subagent_local.len(), 1);
    assert!(a.subagent_local.contains_key("w3"));
    assert!(!a.subagent_local.contains_key("w1"));
    assert!(!a.subagent_local.contains_key("w2"));
}

#[tokio::test]
async fn update_subagent_bar_empty_clears_all() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    a.update_subagent_bar(vec![]);
    assert!(a.subagent_local.is_empty());
}

#[tokio::test]
async fn update_subagent_bar_sanitizes_agent_id() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w\u{0007}1", "running")]);
    assert!(
        a.subagent_local.contains_key("w1"),
        "control chars in agent_id should be stripped"
    );
}

#[tokio::test]
async fn update_subagent_bar_preserves_workflow_on_workflowless_poll() {
    let mut h = harness().await;
    let a = h.app_mut();
    // First push with workflow info.
    a.update_subagent_bar(vec![info_with_workflow("w1", "running", "active", 2, 3)]);
    assert!(a.subagent_local["w1"].info.workflow.is_some());
    // Second push without workflow (get_subagents poll).
    a.update_subagent_bar(vec![info("w1", "running")]);
    assert!(
        a.subagent_local["w1"].info.workflow.is_some(),
        "workflow should be preserved through workflowless poll"
    );
}

// ── tick_subagent_animation ───────────────────────────────────────────

#[tokio::test]
async fn tick_subagent_animation_noop_without_agents() {
    let mut h = harness().await;
    let a = h.app_mut();
    assert!(
        !a.tick_subagent_animation(),
        "no agents → no animation needed"
    );
}

#[tokio::test]
async fn tick_subagent_animation_noop_when_all_idle() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "idle")]);
    assert!(
        !a.tick_subagent_animation(),
        "idle agents → no animation needed"
    );
}

#[tokio::test]
async fn tick_subagent_animation_advances_when_active() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    let frame_before = a.subagent_frame;
    assert!(
        a.tick_subagent_animation(),
        "active agent → animation should advance"
    );
    assert_eq!(
        a.subagent_frame,
        frame_before.wrapping_add(1),
        "frame should increment"
    );
}

#[tokio::test]
async fn tick_subagent_animation_advances_with_mixed_statuses() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "idle"), info("w2", "running")]);
    let frame_before = a.subagent_frame;
    assert!(
        a.tick_subagent_animation(),
        "one active agent → animation needed"
    );
    assert_eq!(a.subagent_frame, frame_before.wrapping_add(1));
}

#[tokio::test]
async fn tick_subagent_animation_wraps_around() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    // Set frame to max and tick — should wrap to 0.
    a.subagent_frame = usize::MAX;
    a.tick_subagent_animation();
    assert_eq!(a.subagent_frame, 0, "frame should wrap around");
}

// ── gc_exited_subagents (App method) ──────────────────────────────────

#[tokio::test]
async fn gc_exited_subagents_noop_when_empty() {
    let mut h = harness().await;
    let a = h.app_mut();
    assert!(!a.gc_exited_subagents(), "empty map → no GC needed");
}

#[tokio::test]
async fn gc_exited_subagents_keeps_running() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    assert!(!a.gc_exited_subagents(), "running agent should not be GC'd");
    assert_eq!(a.subagent_local.len(), 1);
}

#[tokio::test]
async fn gc_exited_subagents_keeps_recent_exit() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "exited")]);
    assert!(!a.gc_exited_subagents(), "recent exit should not be GC'd");
    assert_eq!(a.subagent_local.len(), 1);
}

#[tokio::test]
async fn gc_exited_subagents_removes_expired_exit() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "exited")]);
    // Backdate the exited_at timestamp to beyond the grace period.
    let grace = EXITED_SUBAGENT_GRACE;
    let old = tokio::time::Instant::now() - grace - Duration::from_secs(1);
    if let Some(entry) = a.subagent_local.get_mut("w1") {
        entry.exited_at = Some(old);
    }
    assert!(a.gc_exited_subagents(), "expired exit should be GC'd");
    assert!(a.subagent_local.is_empty());
}

// ── subagent bar rendering via compose_bottom ─────────────────────────

#[tokio::test]
async fn subagent_bar_no_longer_renders_in_compose_bottom() {
    // Sub-agent-first (#820): sub-agents moved out of the bottom stack into the
    // always-on left panel, so compose_bottom must NOT list them; compose_frame
    // (which prefixes the panel) does.
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("worker-1", "running")]);
    let bottom = a.compose_bottom(120).join("\n");
    assert!(
        !bottom.contains("worker-1"),
        "subagent must NOT render in the bottom stack any more: {bottom}"
    );
    let frame = super::app_methods::strip_ansi(&a.compose_frame().join("\n"));
    assert!(
        frame.contains("worker-1"),
        "subagent must render in the left panel instead:\n{frame}"
    );
}

#[tokio::test]
async fn subagent_bar_cleared_from_compose_when_empty() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("worker-1", "running")]);
    a.update_subagent_bar(vec![]);
    let bottom = a.compose_bottom(120);
    let joined = bottom.join("\n");
    assert!(
        !joined.contains("worker-1"),
        "subagent bar should be cleared from compose_bottom: {joined}"
    );
}

// ── #831: a state-changed dropping a subtree clears the panel + footer ──

fn info_with_parent(
    id: &str,
    status: &str,
    parent: &str,
) -> crate::infrastructure::client::SubagentInfoEvent {
    let mut i = info(id, status);
    i.parent_id = Some(parent.to_string());
    i
}

#[tokio::test]
async fn killed_subtree_state_changed_clears_panel_and_footer() {
    let mut h = harness().await;
    let a = h.app_mut();
    // Parent → child → grandchild, plus one unrelated live sibling.
    a.update_subagent_bar(vec![
        info("parent", "running"),
        info_with_parent("child", "running", "parent"),
        info_with_parent("gchild", "running", "child"),
        info("sibling", "running"),
    ]);
    assert_eq!(a.subagent_local.len(), 4);

    // Server cascade-removed parent's subtree and broadcasts the survivor set.
    a.update_subagent_bar(vec![info("sibling", "running")]);

    // The whole dead subtree is gone from the panel; the live sibling stays.
    assert_eq!(a.subagent_local.len(), 1, "dead subtree must be dropped");
    assert!(a.subagent_local.contains_key("sibling"));
    assert!(!a.subagent_local.contains_key("parent"));
    assert!(!a.subagent_local.contains_key("child"));
    assert!(!a.subagent_local.contains_key("gchild"));

    // Footer "N working" reflects the live set (1), not the stale 4.
    let footer = super::app_methods::strip_ansi(&a.compose_bottom(120).join("\n"));
    assert!(
        footer.contains("1 subagent working"),
        "footer must reflect the live set, got: {footer}"
    );
}

#[tokio::test]
async fn surviving_descendant_carries_its_intermediate_ancestors() {
    // #831 nesting guard: the kernel cascade broadcasts the FULL survivor set,
    // so an omitted ancestor only happens for a forwarded child's-eye-view push
    // that lists a sub-tree. When a deeper node survives but its parent chain is
    // omitted from THIS push, the ancestor-preservation loop must carry the
    // intermediate parents over from the previous roster (so nesting depth is
    // preserved and a grandchild is never re-rooted above its parent). This is
    // exactly the path #831's lingering interacted with; assert it explicitly.
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![
        info("parent", "running"),
        info_with_parent("child", "running", "parent"),
        info_with_parent("gchild", "running", "child"),
    ]);
    assert_eq!(a.subagent_local.len(), 3);

    // A forwarded sub-tree push that lists ONLY the surviving grandchild.
    a.update_subagent_bar(vec![info_with_parent("gchild", "running", "child")]);

    // The grandchild survives AND its intermediate ancestors are carried over,
    // not dropped — nesting is preserved rather than re-rooted.
    assert!(a.subagent_local.contains_key("gchild"));
    assert!(
        a.subagent_local.contains_key("child"),
        "intermediate parent must be carried for the surviving descendant"
    );
    assert!(
        a.subagent_local.contains_key("parent"),
        "grandparent must be carried transitively"
    );
}

#[tokio::test]
async fn state_changed_dropping_all_clears_footer_count() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![
        info("parent", "running"),
        info_with_parent("child", "running", "parent"),
    ]);
    a.update_subagent_bar(vec![]);
    assert!(a.subagent_local.is_empty(), "all agents cleared");
    let footer = super::app_methods::strip_ansi(&a.compose_bottom(120).join("\n"));
    assert!(
        !footer.contains("working"),
        "no 'N working' footer once every agent is gone, got: {footer}"
    );
}
