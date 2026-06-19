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

/// The `awaiting` indicator marks only the awaited row, and clears when the
/// await tool ends — with no shared, oscillating await line.
#[tokio::test]
async fn await_indicator_marks_only_the_awaited_row() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("worker", "running", Some(("active", 1, 3))),
        subagent("other", "running", Some(("active", 1, 3))),
    ]));

    h.event(await_start("worker"));
    let dump = h.dump();
    assert!(
        dump.lines()
            .any(|l| l.contains("worker") && l.contains("awaiting")),
        "awaited row should show the indicator:\n{dump}"
    );
    assert!(
        dump.lines()
            .filter(|l| l.contains("other"))
            .all(|l| !l.contains("awaiting")),
        "non-awaited row must not show the indicator:\n{dump}"
    );

    h.event(tool_end("tc-await-worker", "agent_cmd"));
    assert!(
        !h.last().contains("awaiting"),
        "await indicator should clear when the await tool ends:\n{}",
        h.last()
    );
}

/// A state-query result (`get_state`) carries raw workflow JSON
/// (`progress:{done,total,percent}`). It must NOT render in the chat — that raw
/// JSON was the "percent 100 / total 5" flash; the sub-agent panel already shows
/// the progress.
#[tokio::test]
async fn state_query_result_json_never_renders_in_chat() {
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
        !dump.contains("percent") && !dump.contains("isStreaming"),
        "get_state result JSON leaked into the chat (the percent/total flash):\n{dump}"
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
