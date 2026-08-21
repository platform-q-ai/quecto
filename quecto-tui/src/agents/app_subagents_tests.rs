//! Tests for `app_subagents.rs` — the `update_subagent_bar` merge logic
//! and `tick_subagent_animation` methods (issue #729).
//!
//! These drive the real `App` via the headless render harness (no TTY,
//! drained socket) to exercise the subagent bar lifecycle.

use super::tui_harness::{TuiHarness, spawn_subagent_socket};
use super::*;

pub(super) async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

pub(super) fn info(id: &str, status: &str) -> crate::protocol::client::SubagentInfoEvent {
    crate::protocol::client::SubagentInfoEvent {
        agent_uuid: None,
        display_name: None,
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        compact: false,
        pid: 0,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
        execution_backend: None,
        environment: None,
    }
}

fn info_with_workflow(
    id: &str,
    status: &str,
    mode: &str,
    done: u32,
    total: u32,
) -> crate::protocol::client::SubagentInfoEvent {
    crate::protocol::client::SubagentInfoEvent {
        agent_uuid: None,
        display_name: None,
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        compact: false,
        pid: 0,
        socket_path: None,
        parent_id: None,
        workflow: Some(crate::protocol::client::SubagentWorkflow {
            mode: mode.to_string(),
            steps_completed: done,
            steps_total: total,
        }),
        read_only: false,
        execution_backend: None,
        environment: None,
    }
}

// ── update_subagent_bar: basic merge ──────────────────────────────────

#[tokio::test]
async fn update_subagent_bar_inserts_new_agents() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    assert_eq!(a.ac().roster.tracked.len(), 1);
    assert!(a.ac().roster.tracked.contains_key("w1"));
}

#[tokio::test]
async fn update_subagent_bar_updates_existing_status() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    a.update_subagent_bar(vec![info("w1", "idle")]);
    assert_eq!(a.ac().roster.tracked["w1"].info.status, "idle");
}

#[tokio::test]
async fn update_subagent_bar_preserves_started_at_on_update() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    let original_started = a.ac().roster.tracked["w1"].started_at;
    a.update_subagent_bar(vec![info("w1", "idle")]);
    assert_eq!(
        a.ac().roster.tracked["w1"].started_at,
        original_started,
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
    assert!(a.ac().roster.tracked.contains_key("w1"));
    assert!(
        !a.ac().roster.tracked.contains_key("w2"),
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
        a.ac().roster.tracked.contains_key("w2"),
        "exited agent within grace period should be preserved"
    );
}

#[tokio::test]
async fn update_subagent_bar_replaces_all_entries() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running"), info("w2", "running")]);
    a.update_subagent_bar(vec![info("w3", "running")]);
    assert_eq!(a.ac().roster.tracked.len(), 1);
    assert!(a.ac().roster.tracked.contains_key("w3"));
    assert!(!a.ac().roster.tracked.contains_key("w1"));
    assert!(!a.ac().roster.tracked.contains_key("w2"));
}

#[tokio::test]
async fn update_subagent_bar_empty_clears_all() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    a.update_subagent_bar(vec![]);
    assert!(a.ac().roster.tracked.is_empty());
}

#[tokio::test]
async fn update_subagent_bar_sanitizes_agent_id() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w\u{0007}1", "running")]);
    assert!(
        a.ac().roster.tracked.contains_key("w1"),
        "control chars in agent_id should be stripped"
    );
}

#[tokio::test]
async fn update_subagent_bar_preserves_workflow_on_workflowless_poll() {
    let mut h = harness().await;
    let a = h.app_mut();
    // First push with workflow info.
    a.update_subagent_bar(vec![info_with_workflow("w1", "running", "active", 2, 3)]);
    assert!(a.ac().roster.tracked["w1"].info.workflow.is_some());
    // Second push without workflow (get_subagents poll).
    a.update_subagent_bar(vec![info("w1", "running")]);
    assert!(
        a.ac().roster.tracked["w1"].info.workflow.is_some(),
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
    let frame_before = a.ac().roster.frame;
    assert!(
        a.tick_subagent_animation(),
        "active agent → animation should advance"
    );
    assert_eq!(
        a.ac().roster.frame,
        frame_before.wrapping_add(1),
        "frame should increment"
    );
}

#[tokio::test]
async fn tick_subagent_animation_advances_with_mixed_statuses() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "idle"), info("w2", "running")]);
    let frame_before = a.ac().roster.frame;
    assert!(
        a.tick_subagent_animation(),
        "one active agent → animation needed"
    );
    assert_eq!(a.ac().roster.frame, frame_before.wrapping_add(1));
}

#[tokio::test]
async fn tick_subagent_animation_wraps_around() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "running")]);
    // Set frame to max and tick — should wrap to 0.
    a.ac_mut().roster.frame = usize::MAX;
    a.tick_subagent_animation();
    assert_eq!(a.ac().roster.frame, 0, "frame should wrap around");
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
    assert_eq!(a.ac().roster.tracked.len(), 1);
}

#[tokio::test]
async fn gc_exited_subagents_keeps_recent_exit() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "exited")]);
    assert!(!a.gc_exited_subagents(), "recent exit should not be GC'd");
    assert_eq!(a.ac().roster.tracked.len(), 1);
}

#[tokio::test]
async fn gc_exited_subagents_removes_expired_exit() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("w1", "exited")]);
    // Backdate the exited_at timestamp to beyond the grace period.
    let grace = EXITED_SUBAGENT_GRACE;
    let old = tokio::time::Instant::now() - grace - Duration::from_secs(1);
    if let Some(entry) = a.ac_mut().roster.tracked.get_mut("w1") {
        entry.exited_at = Some(old);
    }
    assert!(a.gc_exited_subagents(), "expired exit should be GC'd");
    assert!(a.ac().roster.tracked.is_empty());
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

pub(super) fn info_with_parent(
    id: &str,
    status: &str,
    parent: &str,
) -> crate::protocol::client::SubagentInfoEvent {
    let mut i = info(id, status);
    i.parent_id = Some(parent.to_string());
    i
}

pub(super) fn info_with_parent_and_socket(
    id: &str,
    status: &str,
    parent: &str,
    socket: Option<&str>,
) -> crate::protocol::client::SubagentInfoEvent {
    let mut i = info_with_parent(id, status, parent);
    i.socket_path = socket.map(str::to_string);
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
    assert_eq!(a.ac().roster.tracked.len(), 4);

    // Server cascade-removed parent's subtree and broadcasts the survivor set.
    a.update_subagent_bar(vec![info("sibling", "running")]);

    // The whole dead subtree is gone from the panel; the live sibling stays.
    assert_eq!(
        a.ac().roster.tracked.len(),
        1,
        "dead subtree must be dropped"
    );
    assert!(a.ac().roster.tracked.contains_key("sibling"));
    assert!(!a.ac().roster.tracked.contains_key("parent"));
    assert!(!a.ac().roster.tracked.contains_key("child"));
    assert!(!a.ac().roster.tracked.contains_key("gchild"));

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
    assert_eq!(a.ac().roster.tracked.len(), 3);

    // A forwarded sub-tree push that lists ONLY the surviving grandchild.
    a.update_subagent_bar(vec![info_with_parent("gchild", "running", "child")]);

    // The grandchild survives AND its intermediate ancestors are carried over,
    // not dropped — nesting is preserved rather than re-rooted.
    assert!(a.ac().roster.tracked.contains_key("gchild"));
    assert!(
        a.ac().roster.tracked.contains_key("child"),
        "intermediate parent must be carried for the surviving descendant"
    );
    assert!(
        a.ac().roster.tracked.contains_key("parent"),
        "grandparent must be carried transitively"
    );
}

#[tokio::test]
async fn idle_nested_grandchild_does_not_count_as_working() {
    let mut h = harness().await;
    let a = h.app_mut();

    a.update_subagent_bar(vec![info_with_parent("grandchild", "idle", "child")]);

    assert_eq!(a.ac().roster.tracked["grandchild"].info.status, "idle");
    let frame = a.compose_frame().join("\n");
    let plain_frame = super::app_methods::strip_ansi(&frame);
    assert!(
        plain_frame.contains("grandchild"),
        "idle grandchild must remain visible in the panel: {plain_frame}"
    );
    assert!(
        frame.contains(&crate::components::theme::yellow("grandchild")),
        "idle grandchild name must render with idle colour, got: {frame:?}"
    );
    assert!(
        !frame.contains(&crate::components::theme::green("grandchild")),
        "idle grandchild must not render with running colour, got: {frame:?}"
    );

    let footer = super::app_methods::strip_ansi(&a.compose_bottom(120).join("\n"));
    assert!(
        !footer.contains("working"),
        "idle nested grandchild must not count as working, got: {footer}"
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
    assert!(a.ac().roster.tracked.is_empty(), "all agents cleared");
    let footer = super::app_methods::strip_ansi(&a.compose_bottom(120).join("\n"));
    assert!(
        !footer.contains("working"),
        "no 'N working' footer once every agent is gone, got: {footer}"
    );
}

// ── #838: non-running panel timers must be FROZEN (independent of per-frame now) ──
//
// These tests call `panel_row_elapsed` directly with two explicit `now`s rather
// than driving a full `compose_frame`. That is deliberate: `compose_frame` samples
// `now = Instant::now()` internally (app_methods.rs) and takes no clock argument,
// so a render-level test cannot inject the advanced clock that reproduces the bug
// without a wall-clock dependency. The helper is the single site that maps a
// per-frame `now` to the displayed timer, so exercising it with two `now`s pins
// exactly the regression (a non-running row's value must not move when `now` does)
// while also asserting the frozen value is a plausible run duration, not a constant.

async fn two_running() -> TuiHarness {
    let mut h = TuiHarness::new().await;
    h.event(crate::protocol::client::Event::AgentStart);
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("worker", "running", Some(("active", 1, 3))),
        super::tui_harness::subagent("other", "running", Some(("active", 2, 3))),
    ]));
    h
}

/// An idle sub-agent's panel timer must NOT advance when an incidental (non-tick)
/// render re-samples `now` (scroll/selection/resize). It reads a stable value
/// frozen at the moment it stopped working.
#[tokio::test(start_paused = true)]
async fn idle_panel_timer_is_frozen_across_advancing_now() {
    let mut h = two_running().await;
    // Let the worker accumulate a KNOWN run duration before it goes idle, so the
    // frozen value is non-zero. With a known value we can assert the exact `m:ss`,
    // which distinguishes a genuine freeze from a collapsed-to-constant regression
    // (e.g. always "idle (ran 0:00)") that a mere `v1 == v2` check would miss (#838 review).
    tokio::time::advance(std::time::Duration::from_secs(45)).await;
    // worker goes idle while `other` keeps running (so ticks could still fire).
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("worker", "idle", Some(("active", 1, 3))),
        super::tui_harness::subagent("other", "running", Some(("active", 2, 3))),
    ]));
    let now1 = tokio::time::Instant::now();
    let now2 = now1 + std::time::Duration::from_secs(60);
    let v1 = h.app_mut().panel_row_elapsed(Some("worker"), now1);
    let v2 = h.app_mut().panel_row_elapsed(Some("worker"), now2);
    assert_eq!(
        v1, v2,
        "idle sub-agent timer must be frozen, not advance with a re-sampled now: \
         {v1:?} vs {v2:?}"
    );
    // Exact frozen run duration (start→stopped_at = 45s), independent of `now`.
    assert_eq!(
        v1, "idle (ran 0:45)",
        "idle row must freeze the exact run duration, got: {v1:?}"
    );
}

/// Exited/errored rows are likewise frozen.
#[tokio::test(start_paused = true)]
async fn exited_panel_timer_is_frozen_across_advancing_now() {
    let mut h = two_running().await;
    tokio::time::advance(std::time::Duration::from_secs(75)).await;
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("worker", "exited", Some(("active", 1, 3))),
        super::tui_harness::subagent("other", "running", Some(("active", 2, 3))),
    ]));
    let now1 = tokio::time::Instant::now();
    let now2 = now1 + std::time::Duration::from_secs(90);
    let v1 = h.app_mut().panel_row_elapsed(Some("worker"), now1);
    let v2 = h.app_mut().panel_row_elapsed(Some("worker"), now2);
    assert_eq!(
        v1, v2,
        "exited sub-agent timer must be frozen: {v1:?} vs {v2:?}"
    );
    // Exact frozen run duration (75s), not a collapsed constant.
    assert_eq!(
        v1, "1:15",
        "exited row must freeze the exact run duration, got: {v1:?}"
    );
}

/// An actively-running sub-agent's timer MUST still advance with `now`.
#[tokio::test]
async fn running_panel_timer_still_advances_with_now() {
    let mut h = two_running().await; // both running
    let now1 = tokio::time::Instant::now();
    let now2 = now1 + std::time::Duration::from_secs(60);
    let v1 = h.app_mut().panel_row_elapsed(Some("other"), now1);
    let v2 = h.app_mut().panel_row_elapsed(Some("other"), now2);
    assert_ne!(
        v1, v2,
        "a running sub-agent's timer must keep tracking now: {v1:?} vs {v2:?}"
    );
}

// ── #866: an unconfirmed (optimistic) local entry survives an omitting push ──

#[tokio::test]
async fn optimistic_starting_entry_survives_omitting_payload() {
    // The spawn ToolStart creates a local "starting" entry before the kernel has
    // registered the child. A snapshot taken in that window omits the new child;
    // it must NOT be dropped or the agent stays invisible during a long first
    // turn (#866).
    let mut h = harness().await;
    let a = h.app_mut();
    a.track_starting_subagent(&serde_json::json!({ "agent_id": "w1" }));
    a.update_subagent_bar(vec![info("other", "running")]);
    assert!(
        a.ac().roster.tracked.contains_key("w1"),
        "#866: an unconfirmed local starting entry must not be dropped by a payload that predates its registration"
    );
}

#[tokio::test]
async fn confirmed_running_entry_still_dropped_when_omitted() {
    // #831 non-regression: once the kernel has confirmed an entry (it appeared in
    // a snapshot), a later survivor-set broadcast that omits it (cascade-removed
    // / killed subtree) must still drop it.
    let mut h = harness().await;
    let a = h.app_mut();
    a.track_starting_subagent(&serde_json::json!({ "agent_id": "w1" }));
    a.update_subagent_bar(vec![info("w1", "running"), info("other", "running")]);
    a.update_subagent_bar(vec![info("other", "running")]);
    assert!(
        !a.ac().roster.tracked.contains_key("w1"),
        "#831: a kernel-confirmed entry omitted from a later snapshot must be removed"
    );
}

#[tokio::test]
async fn track_starting_does_not_clobber_confirmed_entry() {
    // A re-played / duplicate spawn ToolStart for an already kernel-confirmed
    // (non-optimistic) id must NOT reset it to an unconfirmed "starting" guess,
    // which would reset its timer and re-open the #831 drop path for the grace
    // window (review).
    let mut h = harness().await;
    let a = h.app_mut();
    a.track_starting_subagent(&serde_json::json!({ "agent_id": "w1" }));
    a.update_subagent_bar(vec![info("w1", "running")]);
    let confirmed_started_at = a.ac().roster.tracked.get("w1").unwrap().started_at;
    assert!(!a.ac().roster.tracked.get("w1").unwrap().optimistic);
    // A stray duplicate spawn ToolStart for the same id.
    a.track_starting_subagent(&serde_json::json!({ "agent_id": "w1" }));
    let entry = a.ac().roster.tracked.get("w1").unwrap();
    assert!(
        !entry.optimistic,
        "a confirmed entry must not revert to optimistic on a duplicate spawn ToolStart"
    );
    assert_eq!(
        entry.started_at, confirmed_started_at,
        "a confirmed entry's started_at must not be reset by a duplicate spawn ToolStart"
    );
    // It must then still drop on an omitting snapshot (#831 preserved).
    a.update_subagent_bar(vec![info("other", "running")]);
    assert!(!a.ac().roster.tracked.contains_key("w1"));
}

#[tokio::test]
async fn optimistic_entry_expires_if_never_confirmed() {
    // A spawn that never registers (e.g. failed launch) must not linger forever:
    // once the optimistic grace elapses, an omitting push removes it.
    let mut h = harness().await;
    let a = h.app_mut();
    a.track_starting_subagent(&serde_json::json!({ "agent_id": "w1" }));
    let old = tokio::time::Instant::now() - std::time::Duration::from_secs(3600);
    a.ac_mut().roster.tracked.get_mut("w1").unwrap().started_at = old;
    a.update_subagent_bar(vec![info("other", "running")]);
    assert!(
        !a.ac().roster.tracked.contains_key("w1"),
        "#866: an unconfirmed optimistic entry past the grace window must be removed"
    );
}

#[tokio::test]
async fn source_scoped_child_roster_preserves_unrelated_sibling() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running"), info("b", "running")]);

    a.update_subagent_bar_from_source(Some("a"), vec![info_with_parent("a1", "running", "a")]);

    assert!(a.ac().roster.tracked.contains_key("b"));
    assert_eq!(
        a.ac().roster.tracked["a1"].info.parent_id.as_deref(),
        Some("a")
    );
}

#[tokio::test]
async fn source_scoped_child_roster_removes_only_source_subtree() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running"), info("b", "running")]);
    a.update_subagent_bar_from_source(
        Some("a"),
        vec![
            info_with_parent("a1", "running", "a"),
            info_with_parent("a2", "running", "a"),
        ],
    );

    a.update_subagent_bar_from_source(Some("a"), vec![]);

    assert!(a.ac().roster.tracked.contains_key("a"));
    assert!(a.ac().roster.tracked.contains_key("b"));
    assert!(!a.ac().roster.tracked.contains_key("a1"));
    assert!(!a.ac().roster.tracked.contains_key("a2"));
}

#[tokio::test]
async fn source_scoped_child_feed_takes_precedence_for_own_subtree() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![
        info("a", "running"),
        info_with_parent("old", "running", "a"),
    ]);

    a.update_subagent_bar_from_source(Some("a"), vec![info_with_parent("fresh", "running", "a")]);

    assert!(a.ac().roster.tracked.contains_key("fresh"));
    assert!(!a.ac().roster.tracked.contains_key("old"));
}

#[tokio::test]
async fn malformed_or_non_socket_paths_are_not_registered_for_connection() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running")]);

    let non_socket =
        std::env::temp_dir().join(format!("quecto-tui-not-a-socket-{}", std::process::id()));
    std::fs::write(&non_socket, b"not a socket").unwrap();
    let real_socket = spawn_subagent_socket("socket-ok");

    a.update_subagent_bar_from_source(
        Some("a"),
        vec![
            info_with_parent_and_socket("empty", "running", "a", Some("   ")),
            info_with_parent_and_socket("relative", "running", "a", Some("relative.sock")),
            info_with_parent_and_socket(
                "file",
                "running",
                "a",
                Some(&non_socket.to_string_lossy()),
            ),
            info_with_parent_and_socket(
                "socket-ok",
                "running",
                "a",
                Some(&real_socket.to_string_lossy()),
            ),
        ],
    );
    assert_eq!(a.ac().roster.tracked["empty"].info.socket_path, None);
    assert_eq!(a.ac().roster.tracked["relative"].info.socket_path, None);
    assert_eq!(a.ac().roster.tracked["file"].info.socket_path, None);
    assert!(
        a.ac().roster.tracked["socket-ok"]
            .info
            .socket_path
            .is_some()
    );
    assert!(!a.ac().roster.feeds.contains_key("empty"));
    assert!(!a.ac().roster.feeds.contains_key("relative"));
    assert!(!a.ac().roster.feeds.contains_key("file"));
    assert!(a.ac().roster.feeds.contains_key("socket-ok"));
    let _ = std::fs::remove_file(non_socket);
}

#[tokio::test]
async fn subagent_state_changed_does_not_make_synced_feed_authoritative() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running")]);

    a.route_subagent_event(
        "a",
        crate::protocol::client::Event::SubagentStateChanged {
            subagents: vec![info_with_parent("a1", "running", "a")],
        },
    );

    assert_eq!(
        a.ac().roster.tracked["a1"].info.parent_id.as_deref(),
        Some("a")
    );
    assert!(a.ac().roster.active_agent_id.is_none());
}
