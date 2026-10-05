//! A sub-agent's footer shows ITS OWN cache-hit ratio (#805 parity with the
//! master footer): the TUI asks the child for its session stats when its feed
//! connects and whenever its run ends (completed, failed or aborted), with at
//! most one request outstanding per child.
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
    TuiHarness, assert_no_further_child_commands, child_command_type,
    drain_child_commands_until_quiet, spawn_subagent_socket_with_commands, subagent_with_socket,
    subagents_changed,
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
    feed_with_authority(crate::agents::feed::FeedAuthority::WarmSync)
}

fn feed_with_authority(
    authority: crate::agents::feed::FeedAuthority,
) -> (FeedState, mpsc::Receiver<Command>) {
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
        authority,
        stats_refresh: crate::agents::feed::StatsRefresh::Settled,
    };
    (feed, cmd_rx)
}

fn footer_text(app: &mut App) -> String {
    let lines = app.active_session_mut().footer.render(200);
    crate::components::ansi::strip_ansi(&lines.join("\n"))
}

/// The ids of every `get_session_stats` the TUI sent the child so far.
fn stats_requests(rx: &mut mpsc::Receiver<Command>) -> Vec<Option<String>> {
    let mut ids = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        if let Command::GetSessionStats { id } = cmd {
            ids.push(id);
        }
    }
    ids
}

/// Answer every `get_session_stats` the TUI sent the child, as the child would.
fn answer_stats_requests(app: &mut App, rx: &mut mpsc::Receiver<Command>) -> usize {
    let ids = stats_requests(rx);
    for id in &ids {
        assert_eq!(id.as_deref(), Some("subagent-stats"), "request id");
        app.route_subagent_event(AGENT, coordinator_stats(id.clone()));
    }
    ids.len()
}

fn turn_end() -> Event {
    Event::TurnEnd {
        message: json!({
            "contextTokens": LAST_CONTEXT,
            "maxContextTokens": SWARM_CEILING,
        }),
    }
}

fn child_reply(command: &str, success: bool) -> Event {
    Event::Response {
        id: None,
        command: command.into(),
        success,
        data: None,
        error: match success {
            true => None,
            false => Some("provider failed".into()),
        },
    }
}

/// A tracked child with a direct feed of `authority`, optionally focused.
async fn tracked_child(
    authority: crate::agents::feed::FeedAuthority,
    focused: bool,
) -> (TuiHarness, mpsc::Receiver<Command>) {
    let mut h = TuiHarness::new().await;
    let app = h.app_mut();
    app.update_subagent_bar(vec![info(AGENT, "running")]);
    let (feed, rx) = feed_with_authority(authority);
    app.ac_mut().roster.feeds.insert(AGENT.into(), feed);
    if focused {
        app.select_agent(Some(AGENT));
    }
    (h, rx)
}

#[tokio::test]
async fn subagent_turn_end_refreshes_its_own_cache_hit_ratio() {
    let (mut h, mut rx) = tracked_child(crate::agents::feed::FeedAuthority::WarmSync, true).await;
    let app = h.app_mut();

    app.route_subagent_event(AGENT, turn_end());
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
async fn an_unfocused_synced_child_still_refreshes_its_stats() {
    // The common swarm case: a warm, ledger-authoritative feed nobody is
    // looking at. Its chat skips live events, but its stats must not.
    let (mut h, mut rx) = tracked_child(
        crate::agents::feed::FeedAuthority::SyncedAuthoritative,
        false,
    )
    .await;
    let app = h.app_mut();

    app.route_subagent_event(AGENT, turn_end());
    assert_eq!(answer_stats_requests(app, &mut rx), 1);

    app.select_agent(Some(AGENT));
    let footer = footer_text(app);
    assert!(
        footer.contains("hit 92.4%"),
        "the unfocused child's footer must hold its own ratio when focused: {footer}"
    );
}

#[tokio::test]
async fn a_failed_or_aborted_run_also_refreshes_the_stats() {
    // The harness records usage for every outcome but reports `turn_end`
    // only for a completed run: a failed run ends with an `agent_error`
    // reply, an aborted one with the `abort` acknowledgement.
    for run_end in [
        child_reply("agent_error", false),
        child_reply("abort", true),
    ] {
        let (mut h, mut rx) =
            tracked_child(crate::agents::feed::FeedAuthority::WarmSync, true).await;
        let app = h.app_mut();
        app.route_subagent_event(AGENT, run_end.clone());
        assert_eq!(
            answer_stats_requests(app, &mut rx),
            1,
            "{run_end:?} must ask the child for its stats"
        );
        let footer = footer_text(app);
        assert!(footer.contains("hit 92.4%"), "{run_end:?}: {footer}");
    }
}

#[tokio::test]
async fn turn_ends_in_quick_succession_keep_one_request_outstanding() {
    let (mut h, mut rx) = tracked_child(crate::agents::feed::FeedAuthority::WarmSync, true).await;
    let app = h.app_mut();

    for _ in 0..5 {
        app.route_subagent_event(AGENT, turn_end());
    }
    let first = stats_requests(&mut rx);
    assert_eq!(
        first.len(),
        1,
        "one request while it is unanswered: {first:?}"
    );

    // Its reply brings one follow-up for the turn ends seen meanwhile.
    app.route_subagent_event(AGENT, coordinator_stats(first[0].clone()));
    let follow_up = stats_requests(&mut rx);
    assert_eq!(follow_up.len(), 1, "one follow-up: {follow_up:?}");

    // The follow-up's reply settles it: nothing more is sent.
    app.route_subagent_event(AGENT, coordinator_stats(follow_up[0].clone()));
    assert!(stats_requests(&mut rx).is_empty());

    // A later run end asks again.
    app.route_subagent_event(AGENT, turn_end());
    assert_eq!(stats_requests(&mut rx).len(), 1);
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

    app.route_subagent_event(AGENT, turn_end());

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

    let commands = drain_until_stats_requests(&mut child_rx, 1).await;
    assert_eq!(
        child_stats_request_count(&commands),
        1,
        "connect must ask the child for its stats exactly once: {commands:?}"
    );

    // A run end before the connect request is answered waits for that reply
    // (one request outstanding), then sends one follow-up.
    h.route(AGENT, turn_end());
    assert_no_further_child_commands(
        &mut child_rx,
        "a run end must not stack a second request on the unanswered connect one",
    )
    .await;
    h.route(AGENT, coordinator_stats(Some("subagent-stats".into())));
    let follow_up = drain_until_stats_requests(&mut child_rx, 1).await;
    assert_eq!(
        child_stats_request_count(&follow_up),
        1,
        "the connect reply brings one follow-up for the run end: {follow_up:?}"
    );
}

fn child_stats_request_count(commands: &[String]) -> usize {
    commands
        .iter()
        .filter(|line| child_command_type(line).as_deref() == Some("get_session_stats"))
        .count()
}

/// Drain the child socket until `want` stats requests have arrived, or a
/// bounded number of quiet windows has passed (load-tolerant).
async fn drain_until_stats_requests(rx: &mut mpsc::Receiver<String>, want: usize) -> Vec<String> {
    let mut commands = Vec::new();
    for _ in 0..40 {
        commands.extend(drain_child_commands_until_quiet(rx).await);
        if child_stats_request_count(&commands) >= want {
            break;
        }
    }
    commands
}

#[tokio::test]
async fn a_child_whose_roster_status_leaves_running_refreshes_its_stats() {
    // A parent's or launcher's abort (`ack: accept`) is acknowledged to that
    // client alone, and the cancelled run reports neither `turn_end` nor a
    // broadcast reply: the roster's running → idle is the one signal left.
    let (mut h, mut rx) = tracked_child(crate::agents::feed::FeedAuthority::WarmSync, true).await;
    let app = h.app_mut();
    assert!(stats_requests(&mut rx).is_empty());

    app.update_subagent_bar(vec![info(AGENT, "idle")]);
    assert_eq!(
        answer_stats_requests(app, &mut rx),
        1,
        "running → idle must ask the child for its stats"
    );
    assert!(footer_text(app).contains("hit 92.4%"));

    // A roster refresh that changes nothing asks for nothing.
    app.update_subagent_bar(vec![info(AGENT, "idle")]);
    assert!(stats_requests(&mut rx).is_empty());
}

#[tokio::test]
async fn a_request_the_feed_could_not_queue_is_retried_on_the_next_child_event() {
    let mut h = TuiHarness::new().await;
    h.app_mut()
        .update_subagent_bar(vec![info(AGENT, "running")]);
    let mut rx = h.insert_full_channel_feed(AGENT);
    let app = h.app_mut();

    app.route_subagent_event(AGENT, turn_end());
    // The queue held only its prefilled command: the request was not queued.
    assert!(matches!(rx.try_recv(), Ok(Command::GetState { .. })));
    assert!(rx.try_recv().is_err());

    // The next event from the child, of any kind, retries the owed request.
    app.route_subagent_event(AGENT, Event::Token { token: "x".into() });
    assert!(
        matches!(rx.try_recv(), Ok(Command::GetSessionStats { .. })),
        "the owed stats request must be retried"
    );
}
