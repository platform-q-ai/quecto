//! Behavioural tests for coalescing the `◆` sub-agent completion DISPLAY notes
//! when a burst flushes together at the parent's idle boundary (#900).
//!
//! These mirror the context-side coalescing #894 fixed on the LLM path, but for
//! the TUI render path (`app_subagent_stream.rs` defer/flush). Driven through
//! the headless render harness so they pin what the operator actually sees.

use crate::infrastructure::client::Event;
use crate::interface::ansi::strip_ansi;
use crate::interface::app::tui_harness::*;

/// Build a completion note message EXACTLY as the kernel's
/// `SubagentNotification::Completed::to_message` does (subagent_registry), so the
/// display-side detection is tested against the real wording, not a stand-in.
fn completion_msg(name: &str) -> String {
    format!(
        "Sub-agent '{name}' finished. Review with agent_cmd get_messages when you need its output."
    )
}

/// Push a completion note for `name` onto the (mid-turn) master session.
fn notify_completion(h: &mut TuiHarness, seq: u64, name: &str) {
    h.event(Event::SubagentNotification {
        agent_id: name.into(),
        sequence: seq,
        message: completion_msg(name),
    });
}

/// Count the rendered `◆` status lines in a (ansi-stripped) frame.
fn bullet_lines(frame: &str) -> usize {
    frame.matches('◆').count()
}

#[tokio::test]
async fn burst_of_completions_coalesces_to_one_summary_line() {
    // AC1/AC5: ≥2 completion notes deferred during a busy parent turn flush as
    // ONE coalesced `◆` summary line listing the names, not N separate lines.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(Event::Token {
        token: "master-response".into(),
    });
    notify_completion(&mut h, 0, "issue-895-feature");
    notify_completion(&mut h, 1, "basic-10");
    notify_completion(&mut h, 2, "basic-11");
    h.event(Event::AgentEnd);

    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert_eq!(
        bullet_lines(&frame),
        1,
        "a burst of completions must coalesce to ONE `◆` line:\n{frame}"
    );
    assert!(
        frame.contains("3 sub-agents finished:"),
        "the summary must count the completions:\n{frame}"
    );
    for name in ["issue-895-feature", "basic-10", "basic-11"] {
        assert!(
            frame.contains(name),
            "the summary must list `{name}`:\n{frame}"
        );
    }
    assert!(
        !frame.contains("Review with agent_cmd"),
        "the verbose per-agent completion text must NOT survive coalescing:\n{frame}"
    );
}

#[tokio::test]
async fn single_completion_keeps_its_own_line() {
    // AC2: a lone completion still renders its own one-line `◆` note verbatim.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    notify_completion(&mut h, 0, "solo-agent");
    h.event(Event::AgentEnd);

    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert_eq!(
        bullet_lines(&frame),
        1,
        "a single completion is one `◆` line:\n{frame}"
    );
    assert!(
        frame.contains("Sub-agent 'solo-agent' finished"),
        "a single completion keeps its own verbatim note:\n{frame}"
    );
    assert!(
        !frame.contains("sub-agents finished:"),
        "a single completion must NOT be reworded as a coalesced summary:\n{frame}"
    );
}

#[tokio::test]
async fn errored_and_exited_are_not_folded_into_summary() {
    // AC3: errored/exited notes pass through as their OWN `◆` lines with detail;
    // only the successful completions coalesce into the summary.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    notify_completion(&mut h, 0, "good-1");
    notify_completion(&mut h, 1, "good-2");
    h.event(Event::SubagentNotification {
        agent_id: "boom".into(),
        sequence: 2,
        message: "Agent 'boom' failed: disk full".into(),
    });
    h.event(Event::SubagentNotification {
        agent_id: "gone".into(),
        sequence: 3,
        message: "Agent 'gone' exited unexpectedly".into(),
    });
    h.event(Event::AgentEnd);

    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    // One summary line + one error line + one exited line = three `◆` lines.
    assert_eq!(
        bullet_lines(&frame),
        3,
        "summary + error + exited = three `◆` lines:\n{frame}"
    );
    assert!(
        frame.contains("2 sub-agents finished:"),
        "the two completions coalesce into one summary:\n{frame}"
    );
    assert!(
        frame.contains("Agent 'boom' failed: disk full"),
        "the errored note must render verbatim, not be folded:\n{frame}"
    );
    assert!(
        frame.contains("Agent 'gone' exited unexpectedly"),
        "the exited note must render verbatim, not be folded:\n{frame}"
    );
}

#[tokio::test]
async fn completion_names_are_capped_with_more_tail() {
    // AC1: the listed names are capped (~10) with a `(+M more)` tail.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    // Short names so the whole coalesced line (incl. the tail) stays within the
    // chat pane width and is not clipped by the viewport.
    const N: u64 = 13;
    for i in 0..N {
        notify_completion(&mut h, i, &format!("a{i}"));
    }
    h.event(Event::AgentEnd);

    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert_eq!(
        bullet_lines(&frame),
        1,
        "many completions still coalesce to one line:\n{frame}"
    );
    assert!(
        frame.contains("13 sub-agents finished:"),
        "the count reflects ALL completions, not just the shown names:\n{frame}"
    );
    assert!(
        frame.contains("(+3 more)"),
        "names past the cap of 10 are summarized as a `(+M more)` tail:\n{frame}"
    );
}

#[tokio::test]
async fn coalesced_notes_still_defer_until_idle() {
    // AC4: coalescing does not change the defer-until-idle policy — nothing
    // appears mid-turn; the summary only surfaces once the parent goes idle.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    notify_completion(&mut h, 0, "child-a");
    notify_completion(&mut h, 1, "child-b");

    let mid = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        !mid.contains("sub-agents finished") && bullet_lines(&mid) == 0,
        "completion notes must stay DEFERRED while the parent is mid-turn:\n{mid}"
    );

    h.event(Event::AgentEnd);
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("2 sub-agents finished:"),
        "the coalesced summary surfaces once the parent is idle:\n{frame}"
    );
}
