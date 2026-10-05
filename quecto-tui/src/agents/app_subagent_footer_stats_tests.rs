//! A sub-agent's footer shows ITS OWN cache-hit ratio (#805 parity with the
//! master footer): the TUI asks the child for its session stats when a turn
//! ends and when it first connects, exactly as the master footer does.
//!
//! Shape of the evidence: a swarm coordinator on anthropic-api/claude-opus-5-5
//! made 20 requests whose normalized usage sums to 42 uncached input, 530,402
//! cache-read and 43,394 cache-write tokens, so its harness reported
//! `cacheHitRatio` 530402 / (42 + 530402 + 43394) = 0.924. Its last request
//! occupied 43,396 context tokens under the 300,000-token swarm ceiling. Before
//! the fix the TUI never asked the child for stats after a turn, so the
//! coordinator's footer carried no cache figure at all; its only percentage
//! was the context gauge `43k/300k (14.5%)`.

use super::app_subagents_tests::info;
use super::tui_harness::{
    TuiHarness, drain_child_commands_until_quiet, spawn_subagent_socket_with_commands,
    subagent_with_socket, subagents_changed,
};
use super::*;
use serde_json::json;

const AGENT: &str = "coordinator";
const UNCACHED_INPUT: u64 = 42;
const CACHE_READ: u64 = 530_402;
const CACHE_WRITE: u64 = 43_394;
const LAST_CONTEXT: u64 = 43_396;
const SWARM_CEILING: u64 = 300_000;

/// The normalized ratio the coordinator's harness logged for these totals.
fn harness_ratio() -> f64 {
    let ratio = CACHE_READ as f64 / (UNCACHED_INPUT + CACHE_READ + CACHE_WRITE) as f64;
    assert!((ratio - 0.9243).abs() < 1e-4, "fixture drifted: {ratio}");
    ratio
}

/// The child's `get_session_stats` payload, as its harness serializes it.
fn coordinator_stats(id: Option<String>) -> Event {
    Event::Response {
        id,
        command: "get_session_stats".into(),
        success: true,
        data: Some(json!({
            "sessionKey": format!("cli:{AGENT}"),
            "totalMessages": 61,
            "tokens": {
                "input": UNCACHED_INPUT,
                "output": 15_890,
                "cacheRead": CACHE_READ,
                "cacheWrite": CACHE_WRITE,
                "total": UNCACHED_INPUT + 15_890,
            },
            "costMicroUsd": 641_012,
            "cacheHitRatio": harness_ratio(),
            "contextTokens": LAST_CONTEXT,
            "maxContextTokens": SWARM_CEILING,
        })),
        error: None,
    }
}

fn feed_with_rx() -> (FeedState, mpsc::Receiver<Command>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let feed = FeedState {
        cmd_tx,
        handle: tokio::spawn(async {}),
        inspection_only: false,
        epoch: 0,
        rev: 0,
        last_fresh_at: None,
        supports_sync: false,
        pending_rev: None,
        transcript: crate::agents::ledger::LedgerTranscript::default(),
        authority: crate::agents::feed::FeedAuthority::WarmSync,
    };
    (feed, cmd_rx)
}

fn footer_text(app: &mut App) -> String {
    let lines = app.active_session_mut().footer.render(200);
    crate::components::ansi::strip_ansi(&lines.join("\n"))
}

/// Answer every `get_session_stats` the TUI sent the child, as the child would.
fn answer_stats_requests(app: &mut App, rx: &mut mpsc::Receiver<Command>) -> usize {
    let mut answered = 0;
    while let Ok(cmd) = rx.try_recv() {
        if let Command::GetSessionStats { id } = cmd {
            app.route_subagent_event(AGENT, coordinator_stats(id));
            answered += 1;
        }
    }
    answered
}

#[tokio::test]
async fn subagent_turn_end_refreshes_its_own_cache_hit_ratio() {
    let mut h = TuiHarness::new().await;
    let app = h.app_mut();
    app.update_subagent_bar(vec![info(AGENT, "running")]);
    let (feed, mut rx) = feed_with_rx();
    app.ac_mut().roster.feeds.insert(AGENT.into(), feed);
    app.select_agent(Some(AGENT));

    app.route_subagent_event(
        AGENT,
        Event::TurnEnd {
            message: json!({
                "contextTokens": LAST_CONTEXT,
                "maxContextTokens": SWARM_CEILING,
            }),
        },
    );
    let at_turn_end = footer_text(app);
    assert!(
        at_turn_end.contains("43k/300k (14.5%)") && !at_turn_end.contains("hit "),
        "the context gauge is the footer's only percentage before stats land: {at_turn_end}"
    );

    let answered = answer_stats_requests(app, &mut rx);
    let footer = footer_text(app);
    assert_eq!(
        answered, 1,
        "a sub-agent's turn end must ask that child for its own stats once; footer: {footer}"
    );
    assert!(
        footer.contains("hit 92.4%"),
        "the sub-agent footer must show its harness's normalized cache-hit ratio: {footer}"
    );
    assert!(
        footer.contains("43k/300k (14.5%)"),
        "the context gauge keeps the child's occupancy beside its cache figures: {footer}"
    );
    assert!(
        footer.contains("cache 530k/43k"),
        "the sub-agent footer must show its own cache read/write: {footer}"
    );
}

#[tokio::test]
async fn subagent_stats_request_never_touches_an_inspection_only_feed() {
    // An inspection feed reaches the child through the master, whose routing
    // allowlist carries no session stats; the request must not leak into it.
    let mut h = TuiHarness::new().await;
    let app = h.app_mut();
    app.update_subagent_bar(vec![info(AGENT, "running")]);
    let (mut feed, mut rx) = feed_with_rx();
    feed.inspection_only = true;
    app.ac_mut().roster.feeds.insert(AGENT.into(), feed);

    app.route_subagent_event(
        AGENT,
        Event::TurnEnd {
            message: json!({
                "contextTokens": LAST_CONTEXT,
                "maxContextTokens": SWARM_CEILING,
            }),
        },
    );

    let mut sent = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        sent.push(cmd);
    }
    assert!(
        sent.iter()
            .all(|cmd| !matches!(cmd, Command::GetSessionStats { .. })),
        "no session-stats request may be sent on an inspection-only feed: {sent:?}"
    );
}

#[tokio::test]
async fn connecting_to_a_subagent_asks_for_its_own_stats() {
    // A child whose run ended before the TUI connected pushes no busy
    // snapshot, so the connect itself must ask for the child's stats.
    let (socket, mut child_rx) = spawn_subagent_socket_with_commands(AGENT);
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent_with_socket(
        AGENT,
        "idle",
        None,
        Some(socket),
    )]));

    let commands = drain_child_commands_until_quiet(&mut child_rx).await;
    let stats_requests = commands
        .iter()
        .filter(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
                .as_deref()
                == Some("get_session_stats")
        })
        .count();
    assert_eq!(
        stats_requests, 1,
        "connect must ask the child for its stats exactly once: {commands:?}"
    );
}
