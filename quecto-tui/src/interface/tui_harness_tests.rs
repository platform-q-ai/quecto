//! Layout/flicker regression tests driven by the headless render harness.
//!
//! These reproduce the multi-agent scenarios that caused the sub-agent panel
//! judder and the workflow-bar flash, and assert on layout stability — no
//! manual eyeballing of a live session required.

use super::tui_harness::*;
use crate::infrastructure::client::Event;

/// A batch of sub-agents spawning and running workflows must not produce a
/// single-frame "flash" (height spike/dip) or a transient line in the below-chat
/// region. (Monotonic growth as agents appear is allowed; flicker is not.)
#[tokio::test]
async fn multi_agent_spawn_and_workflow_has_no_flash() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);

    // Spawn 5 agents one by one (each registers, then a state push arrives).
    let mut infos = Vec::new();
    for i in 1..=5 {
        let id = format!("ui-test-a{i}");
        h.event(spawn_start(&id));
        infos.push(subagent(&id, "running", Some(("active", 0, 3))));
        h.event(subagents_changed(infos.clone()));
        h.tick();
        h.tick();
    }

    // Workflows advance 0→3 across all agents.
    for done in 1..=3 {
        let infos: Vec<_> = (1..=5)
            .map(|i| {
                subagent(
                    &format!("ui-test-a{i}"),
                    "running",
                    Some(("active", done, 3)),
                )
            })
            .collect();
        h.event(subagents_changed(infos));
        h.tick();
        h.tick();
    }

    // The judder signal is a height spike/dip (a line that pops in then out).
    // Monotonic growth as agents appear is fine; content changes on a stable
    // row (Starting→Running, wf 0/3→1/3) are not flicker.
    assert!(
        h.flashes().is_empty(),
        "below-chat height spiked/dipped (flash): {:?}\nheights = {:?}\n{}",
        h.flashes(),
        h.heights(),
        h.dump()
    );
}

/// A child's forwarded `workflow_state` (including from a not-yet-registered
/// child — the "first load" race) must never render a parent workflow bar.
#[tokio::test]
async fn forwarded_child_workflow_never_renders_parent_bar() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("a1", "running", Some(("active", 0, 5))),
        subagent("a2", "running", Some(("active", 0, 5))),
    ]));

    // Forwarded events for known AND unknown (unregistered) children.
    for done in 1..=5 {
        h.event(forwarded_workflow("a1", done, 5));
        h.event(forwarded_workflow("a2", done, 5));
        h.event(forwarded_workflow("not-yet-registered", done, 5));
    }

    let dump = h.dump();
    // The parent workflow bar would render a percent and an issue line; neither
    // should ever appear (the per-agent rows show "wf …/…" without a percent).
    assert!(
        !dump.contains('%'),
        "a forwarded child workflow rendered a percent (parent bar leak):\n{dump}"
    );
    assert!(
        h.flashes().is_empty(),
        "forwarded child events caused a height flash: {:?}\nheights = {:?}",
        h.flashes(),
        h.heights()
    );
}

/// Sub-agent-first layout (#820): the bottom sub-agent bar — and with it the
/// per-row `awaiting` indicator — no longer renders. Agents now live in the
/// always-on left panel, so the bottom stack must never carry an await line and
/// the agent rows appear in the panel (full frame) instead.
#[tokio::test]
async fn awaiting_indicator_gone_agents_live_in_panel() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("worker", "running", Some(("active", 1, 3))),
        subagent("other", "running", Some(("active", 1, 3))),
    ]));

    h.event(await_start("worker"));
    let dump = h.dump();
    assert!(
        !dump.contains("awaiting"),
        "the awaiting indicator moved out of the bottom stack with the bar:\n{dump}"
    );
    // The agents themselves are listed in the always-on panel (full frame).
    let full = h.dump_full();
    assert!(
        full.contains("worker") && full.contains("other"),
        "sub-agent rows must appear in the left panel:\n{full}"
    );

    h.event(tool_end("tc-await-worker", "agent_cmd"));
    assert!(
        !h.last().contains("awaiting"),
        "no await indicator may render in the bottom stack:\n{}",
        h.last()
    );
}

/// A genuine `agent_cmd get_state` tool call must render a tool box in the chat,
/// consistent with `get_messages`/`await` (#865). The box header carries the
/// `command → agent_id` detail. (The TUI's OWN internal get_state polling flows
/// through Response events / `app_response.rs`, not this tool path, so it is
/// unaffected and stays box-free.)
#[tokio::test]
async fn agent_cmd_get_state_renders_a_tool_box() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "child",
        "running",
        Some(("active", 5, 5)),
    )]));
    h.event(Event::ToolExecutionStart {
        tool_call_id: "t1".into(),
        tool_name: "agent_cmd".into(),
        args: serde_json::json!({"command":"get_state","agent_id":"child"}),
    });
    h.event(Event::ToolExecutionEnd {
        tool_call_id: "t1".into(),
        tool_name: "agent_cmd".into(),
        result: serde_json::json!({"content":[{"type":"text","text":"{\"isStreaming\":false,\"workflow\":{\"mode\":\"complete\",\"progress\":{\"done\":5,\"total\":5,\"percent\":100}}}"}]}),
        is_error: false,
    });
    h.tick();
    let dump = h.dump_full();
    assert!(
        dump.contains("get_state → child"),
        "agent_cmd get_state should render a tool box in the chat (#865):\n{dump}"
    );
    // Pin the box BODY, not just the header: the completed box shows a green
    // success tick, proving the result actually rendered into the box (the same
    // result-preview path as `get_messages`/`await`), not an empty/pending shell.
    assert!(
        dump.contains('✓'),
        "completed get_state box must render its success result body (#865):\n{dump}"
    );
}

/// #871: control/destructive `agent_cmd` commands (abort/kill) append a chat
/// entry on the master path, just like read-only queries do — so the transcript
/// stays complete and it's clear why a sub-agent stopped.
#[tokio::test]
async fn agent_cmd_abort_and_kill_append_chat_entry_on_master_path() {
    for cmd in ["abort", "kill"] {
        let mut h = TuiHarness::new().await;
        let before = h.app_mut().master_session.chat.entry_count();
        h.event(Event::ToolExecutionStart {
            tool_call_id: "t1".into(),
            tool_name: "agent_cmd".into(),
            args: serde_json::json!({"command":cmd,"agent_id":"worker-1"}),
        });
        let after = h.app_mut().master_session.chat.entry_count();
        assert_eq!(
            after,
            before + 1,
            "agent_cmd {cmd} should append a chat entry"
        );
    }
}

/// #871: control/destructive `agent_cmd` commands (abort/kill) must render a
/// tool box in the chat, the same way `get_state`/`await` do. A frame-level
/// assertion guards against a predicate-only false positive.
#[tokio::test]
async fn agent_cmd_abort_and_kill_render_tool_boxes() {
    for cmd in ["abort", "kill"] {
        let mut h = TuiHarness::new().await;
        h.event(Event::AgentStart);
        h.event(subagents_changed(vec![subagent(
            "child",
            "running",
            Some(("active", 5, 5)),
        )]));
        h.event(Event::ToolExecutionStart {
            tool_call_id: "t1".into(),
            tool_name: "agent_cmd".into(),
            args: serde_json::json!({"command":cmd,"agent_id":"child"}),
        });
        h.event(Event::ToolExecutionEnd {
            tool_call_id: "t1".into(),
            tool_name: "agent_cmd".into(),
            result: serde_json::json!({"content":[{"type":"text","text":"ok"}]}),
            is_error: false,
        });
        h.tick();
        let dump = h.dump_full();
        assert!(
            dump.contains(&format!("{cmd} → child")),
            "agent_cmd {cmd} should render a tool box in the chat (#871):\n{dump}"
        );
        assert!(
            dump.contains('✓'),
            "completed {cmd} box must render its success body (#871):\n{dump}"
        );
    }
}

/// #871: control/destructive `agent_cmd` commands routed into a SELECTED
/// sub-agent's direct stream must render the same tool box in that sub-agent's
/// chat, mirroring the master path.
#[tokio::test]
async fn agent_cmd_abort_and_kill_render_tool_boxes_in_subagent_view() {
    for cmd in ["abort", "kill"] {
        let mut h = TuiHarness::new().await;
        h.event(Event::AgentStart);
        h.event(subagents_changed(vec![subagent(
            "child",
            "running",
            Some(("active", 5, 5)),
        )]));
        h.select(Some("child"));
        h.route(
            "child",
            Event::ToolExecutionStart {
                tool_call_id: "s1".into(),
                tool_name: "agent_cmd".into(),
                args: serde_json::json!({"command":cmd,"agent_id":"grandchild"}),
            },
        );
        h.route(
            "child",
            Event::ToolExecutionEnd {
                tool_call_id: "s1".into(),
                tool_name: "agent_cmd".into(),
                result: serde_json::json!({"content":[{"type":"text","text":"ok"}]}),
                is_error: false,
            },
        );
        h.tick();
        let dump = h.dump_full();
        assert!(
            dump.contains(&format!("{cmd} → grandchild")),
            "agent_cmd {cmd} must render a tool box in the sub-agent view too (#871):\n{dump}"
        );
    }
}

/// The sub-agent view path (#865 acceptance: BOTH views). A genuine
/// `agent_cmd get_state` routed into a SELECTED sub-agent's direct stream must
/// render the same tool box in that sub-agent's chat, mirroring the master path.
#[tokio::test]
async fn agent_cmd_get_state_renders_a_tool_box_in_subagent_view() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "child",
        "running",
        Some(("active", 5, 5)),
    )]));
    h.select(Some("child"));
    h.route(
        "child",
        Event::ToolExecutionStart {
            tool_call_id: "s1".into(),
            tool_name: "agent_cmd".into(),
            args: serde_json::json!({"command":"get_state","agent_id":"grandchild"}),
        },
    );
    h.route(
        "child",
        Event::ToolExecutionEnd {
            tool_call_id: "s1".into(),
            tool_name: "agent_cmd".into(),
            result: serde_json::json!({"content":[{"type":"text","text":"ok"}]}),
            is_error: false,
        },
    );
    h.tick();
    let dump = h.dump_full();
    assert!(
        dump.contains("get_state → grandchild"),
        "agent_cmd get_state must render a tool box in the sub-agent view too (#865):\n{dump}"
    );
    assert!(
        dump.contains('✓'),
        "completed get_state box must render its success body in the sub-agent view (#865):\n{dump}"
    );
}

/// The real-log judder: workflow sub-agents fire notifications, so the parent
/// does many short runs, each creating/dropping the spinner. With sub-agents
/// present that 0↔1 spinner toggle must NOT reflow the chat (its line is
/// reserved), or the panel size oscillates (6↔7 / 11↔12) on every run.
#[tokio::test]
async fn spinner_blink_with_subagents_does_not_reflow() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(
        (1..=5)
            .map(|i| subagent(&format!("a{i}"), "running", Some(("active", 1, 3))))
            .collect(),
    ));
    // Parent runs repeatedly (one per subagent notification): spinner blinks.
    for _ in 0..5 {
        h.event(Event::AgentEnd { messages: vec![] }); // spinner off
        h.event(Event::AgentStart); // spinner on
    }
    assert!(
        h.flashes().is_empty(),
        "spinner blink reflowed the chat (height flash): {:?}\nheights = {:?}",
        h.flashes(),
        h.heights()
    );
}

/// End-to-end guard for the parse-error leak: forwarded child workflow_state
/// arrives as a raw wire line WITHOUT `steps`. It must deserialize (so the
/// client never drops it / prints it over the TUI), and it must not render
/// into the frame (it's a child's, ignored by agent_id).
#[tokio::test]
async fn forwarded_canonical_wire_line_parses_and_does_not_leak() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "tui-ui-test-3",
        "running",
        Some(("active", 3, 5)),
    )]));
    // The exact shape the monitor forwards (canonical, no `steps`).
    h.event_line(
        r#"{"type":"workflow_state","agent_id":"tui-ui-test-3","parent_id":null,"mode":"active","progress":{"done":3,"total":5,"percent":60}}"#,
    );
    assert!(
        !h.dump_full().contains("percent"),
        "forwarded workflow_state leaked a percent into the frame:\n{}",
        h.dump_full()
    );
}

/// A `get_subagents` poll carries no workflow snapshot. Sub-agent-first (#820):
/// the per-row `n/n` workflow snapshot no longer renders (the bottom bar is
/// gone), but the agent row itself must survive a workflowless poll — it stays
/// listed in the always-on left panel rather than vanishing.
#[tokio::test]
async fn subagent_row_survives_workflowless_poll() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "a1",
        "running",
        Some(("active", 2, 3)),
    )]));
    assert!(
        h.dump_full().contains("a1"),
        "agent row should show in the panel:\n{}",
        h.dump_full()
    );
    // get_subagents response shape: status only, no workflow.
    h.event(subagents_changed(vec![subagent("a1", "running", None)]));
    assert!(
        h.dump_full().contains("a1"),
        "agent row must persist through a workflowless poll:\n{}",
        h.dump_full()
    );
}

/// While the parent is idle but a child is still active, the reserved spinner
/// slot shows an animated "N working" indicator (not a blank) — the "missing
/// working spinner" report.
#[tokio::test]
async fn idle_parent_shows_subagent_activity() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "a1",
        "running",
        Some(("active", 2, 3)),
    )]));
    h.event(Event::AgentEnd { messages: vec![] }); // parent idle, a1 still running
    assert!(
        h.last().contains("subagent working"),
        "idle parent with an active child should show an activity line:\n{}",
        h.last()
    );
}

#[tokio::test]
async fn harness_gc_removes_exited_subagents_and_captures() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent("done", "exited", None)]));
    let before = h.heights().len();
    h.gc();
    assert_eq!(h.heights().len(), before + 1);
}

#[tokio::test]
async fn transient_detection_normalizes_spinner_and_digits() {
    let frames = vec![
        vec!["worker 1 ⠁".to_string()],
        vec!["flash 123".to_string(), "worker 2 ⠂".to_string()],
        vec!["worker 3 ⠄".to_string()],
    ];
    let transients = transient_in(&frames);
    assert_eq!(transients, vec![(1, "flash 123".to_string())]);
}

#[tokio::test]
async fn event_line_invalid_json_panics_with_context() {
    let mut h = TuiHarness::new().await;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.event_line("not-json");
    }));
    assert!(result.is_err());
}

// ── Regression tests: workflow-bar display cluster (#912/#913/#915 + fleet) ───
// RED-phase: these assert the FIXED behaviour and currently FAIL.
mod workflow_display_regression {
    use super::*;
    use crate::infrastructure::client::{Event, SubagentInfoEvent, SubagentWorkflow};

    fn info(
        id: &str,
        status: &str,
        wf: Option<(&str, u32, u32)>,
        parent: Option<&str>,
    ) -> SubagentInfoEvent {
        SubagentInfoEvent {
            agent_id: id.into(),
            status: status.into(),
            last_tool: None,
            last_error: None,
            pid: 0,
            socket_path: None,
            parent_id: parent.map(|s| s.to_string()),
            workflow: wf.map(|(m, d, t)| SubagentWorkflow {
                mode: m.into(),
                steps_completed: d,
                steps_total: t,
            }),
            read_only: false,
        }
    }
    fn wf_state(
        agent_id: Option<&str>,
        done: u32,
        total: u32,
        with_steps: bool,
        with_issue: bool,
    ) -> Event {
        let steps: Vec<serde_json::Value> = if with_steps {
            (0..total).map(|i| serde_json::json!({"index": i+1, "label": format!("Step {}", i+1), "phase": "build", "done": i < done})).collect()
        } else {
            Vec::new()
        };
        Event::WorkflowState {
            agent_id: agent_id.map(|s| s.to_string()),
            steps,
            progress: serde_json::json!({"done": done, "total": total, "percent": done.checked_mul(100).and_then(|n| n.checked_div(total)).unwrap_or(0)}),
            active_issue: if with_issue {
                Some(serde_json::json!({"number": 42, "title": "demo"}))
            } else {
                None
            },
            mode: Some("active".into()),
            active_template: None,
            available_templates: None,
        }
    }
    fn get_state_dormant(auto: bool) -> Event {
        Event::Response {
            id: None,
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({"isStreaming": false, "messageCount": 1,
                // A real `--workflow` boot is selector mode WITH templates
                // available but nothing selected (#912b) — this must still hide.
                "workflow": {"mode":"selecting_template","progress":{"done":0,"total":0},"steps":[],
                    "availableTemplates":[{"id":"feature","label":"Feature"},{"id":"fix","label":"Fix"}],
                    "automation":{"autoContinue":auto,"completionNudge":false}}})),
            error: None,
        }
    }

    // #912: a master with workflow ENABLED but nothing SELECTED (dormant
    // selecting_template) must NOT render a workflow bar.
    #[tokio::test]
    async fn master_dormant_selecting_template_shows_no_bar() {
        let mut h = TuiHarness::sized(100, 24).await;
        h.event(Event::AgentStart);
        h.event(Event::AgentEnd { messages: vec![] });
        h.select(None);
        h.event(get_state_dormant(true));
        let pane = h.main_pane();
        assert!(
            !pane.contains("starting") && !pane.contains("auto:"),
            "#912: master with no workflow selected must show NO workflow bar, got:\n{pane}"
        );
    }

    // #913: selecting a sub-agent that has a workflow in the registry snapshot
    // must render its main-pane bar immediately (seeded from the snapshot),
    // without needing a routed get_state/live workflow_state.
    #[tokio::test]
    async fn subagent_bar_seeded_from_snapshot_on_select() {
        let mut h = TuiHarness::sized(100, 24).await;
        h.event(Event::AgentStart);
        h.event(spawn_start("wfsub"));
        h.event(subagents_changed(vec![info(
            "wfsub",
            "running",
            Some(("active", 2, 6)),
            None,
        )]));
        h.select(Some("wfsub"));
        let pane = h.main_pane();
        assert!(
            pane.contains("2/6") || pane.contains("Step"),
            "#913: selecting a sub-agent with a workflow snapshot must show its bar (2/6), got:\n{pane}"
        );
    }

    // #915: a transient 0/0-with-issue workflow_state must NOT regress an
    // already-advanced bar down to 'starting…'.
    #[tokio::test]
    async fn sticky_bar_not_regressed_by_zero_with_issue() {
        let mut h = TuiHarness::sized(100, 24).await;
        h.event(Event::AgentStart);
        h.event(spawn_start("wfsub"));
        h.event(subagents_changed(vec![info(
            "wfsub",
            "running",
            Some(("active", 2, 6)),
            None,
        )]));
        h.select(Some("wfsub"));
        h.route("wfsub", wf_state(None, 2, 6, true, true)); // 2/6 advanced bar
        h.route("wfsub", wf_state(None, 0, 0, false, true)); // transient 0/0 WITH issue
        let pane = h.main_pane();
        assert!(
            pane.contains("2/6") && !pane.contains("starting"),
            "#915: a 0/0-with-issue event must not regress an advanced bar to 'starting…', got:\n{pane}"
        );
    }

    // Complex fleet: master triages simple + workflow + nested children. The
    // display must stay stable (no flicker/judder) AND each fixed behaviour holds.
    #[tokio::test]
    async fn fleet_triage_display_is_correct_and_stable() {
        let mut h = TuiHarness::sized(110, 44).await;
        h.event(Event::AgentStart);
        let fleet = vec![
            info("simple-1", "running", None, None),
            info("wf-1", "running", Some(("active", 0, 5)), None),
            info("wf-2", "running", Some(("active", 2, 18)), None),
            info("nested-parent", "running", Some(("active", 1, 3)), None),
            info(
                "gc-wf",
                "running",
                Some(("active", 1, 2)),
                Some("nested-parent"),
            ),
        ];
        for id in ["simple-1", "wf-1", "wf-2", "nested-parent", "gc-wf"] {
            h.event(spawn_start(id));
        }
        h.event(subagents_changed(fleet.clone()));
        h.tick();
        h.tick();
        // stable, no flicker
        assert!(
            h.flashes().is_empty(),
            "fleet: flicker detected: {:?}\n{}",
            h.flashes(),
            h.dump()
        );
        // select a workflow child -> bar shows immediately from snapshot (#913)
        h.select(Some("wf-2"));
        assert!(
            h.main_pane().contains("2/18") || h.main_pane().contains("Step"),
            "fleet: selected wf-2 must show its bar from the snapshot:\n{}",
            h.main_pane()
        );
        // completion-note coalescing (deferred then flushed)
        h.event(Event::SubagentNotification{agent_id:"wf-1".into(), sequence:1, message:"Sub-agent 'wf-1' finished. Review with agent_cmd get_messages when you need its output.".into()});
        h.event(Event::SubagentNotification{agent_id:"wf-2".into(), sequence:1, message:"Sub-agent 'wf-2' finished. Review with agent_cmd get_messages when you need its output.".into()});
        h.event(Event::AgentEnd { messages: vec![] });
        let frame = h.full_frame();
        assert!(
            frame.contains("2 sub-agents finished"),
            "fleet: 2 completion notes must coalesce into one summary, got:\n{frame}"
        );
    }
}
