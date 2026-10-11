use super::{
    AbortOnDrop, BoardHints, BoardViewCtx, MALFORMED, Seen, SwarmBoards, answer, changes,
    intercept, spawn_change_hints,
};
use crate::application::environments::dto::{
    BoardSighting, HostedBoardRevision, SwarmBoardSnapshot,
};
use crate::application::environments::ports::{HostedSwarmBoard, PortFuture};
use crate::application::environments::use_cases::ReadSwarmBoard;
use crate::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::infrastructure::tools::subagent_registry::SubagentRegistry;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::time::Duration;

/// A board at `C1` only, at revision 7 of run `r1`.
struct OneBoard;

impl HostedSwarmBoard for OneBoard {
    fn board<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<SwarmBoardSnapshot>, String>> {
        let board = (record.environment_ref == "C1").then(|| {
            SwarmBoardSnapshot::object(
                json!({"revision": 7, "run": null, "items": [], "truncated": false}),
            )
            .unwrap()
        });
        Box::pin(async move { Ok(board) })
    }

    fn revision<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<HostedBoardRevision>, String>> {
        let revision = (record.environment_ref == "C1").then(|| HostedBoardRevision {
            run: Some("r1".into()),
            revision: 7,
        });
        Box::pin(async move { Ok(revision) })
    }
}

/// The board at `C1` only, run `r1`, at the revision the test holds; each
/// read of it moves it on by `step`.
struct Moving {
    revision: Arc<AtomicI64>,
    step: i64,
}

impl HostedSwarmBoard for Moving {
    fn board<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<SwarmBoardSnapshot>, String>> {
        Box::pin(async { Ok(SwarmBoardSnapshot::object(json!({}))) })
    }

    fn revision<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<HostedBoardRevision>, String>> {
        let revision = (record.environment_ref == "C1").then(|| HostedBoardRevision {
            run: Some("r1".into()),
            revision: self.revision.fetch_add(self.step, Ordering::SeqCst),
        });
        Box::pin(async move { Ok(revision) })
    }
}

/// A board whose read waits until the test releases it.
struct Gated(Arc<tokio::sync::Notify>);

impl HostedSwarmBoard for Gated {
    fn board<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<SwarmBoardSnapshot>, String>> {
        let gate = self.0.clone();
        Box::pin(async move {
            gate.notified().await;
            Ok(SwarmBoardSnapshot::object(json!({})))
        })
    }

    fn revision<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<HostedBoardRevision>, String>> {
        Box::pin(async { Ok(None) })
    }
}

fn record(reference: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: reference.into(),
        environment_id: format!("runtime-{reference}"),
        environment_uuid: format!("uuid-{reference}"),
        name: None,
        workspace_path: "/workspace".into(),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

/// The session's swarms `C1` and `C2`, read through `port`.
fn read_over(port: Arc<dyn HostedSwarmBoard>) -> Arc<ReadSwarmBoard> {
    let registry = EnvironmentRegistry::new();
    registry.commit(record("C1"));
    registry.commit(record("C2"));
    Arc::new(ReadSwarmBoard::new(registry, port))
}

fn view(read: Arc<ReadSwarmBoard>) -> SwarmBoards {
    SwarmBoards::new(move || Some(read.clone()))
}

fn wire(event: crate::interface::cli::protocol::AgentEvent) -> Value {
    serde_json::from_str(&event.to_json_line()).unwrap()
}

fn hint(swarm: &str, run: &str, revision: i64) -> String {
    json!({"type": "swarm_board_changed", "swarm": swarm, "run": run, "revision": revision, "removed": false})
        .to_string()
}

fn removal(swarm: &str) -> String {
    json!({"type": "swarm_board_changed", "swarm": swarm, "run": null, "revision": null, "removed": true})
        .to_string()
}

#[tokio::test]
async fn get_swarm_board_answers_the_board_with_its_swarm() {
    let read = read_over(Arc::new(OneBoard));
    let response = wire(answer(Some(&read), Some("b1"), Some("C1")).await);
    assert_eq!(
        (response["type"].clone(), response["command"].clone()),
        (json!("response"), json!("get_swarm_board"))
    );
    assert_eq!(
        (response["id"].clone(), response["success"].clone()),
        (json!("b1"), json!(true))
    );
    let data = &response["data"];
    assert_eq!(
        (
            data["swarm"].clone(),
            data["revision"].clone(),
            data["truncated"].clone()
        ),
        (json!("C1"), json!(7), json!(false))
    );
}

#[tokio::test]
async fn get_swarm_board_refuses_an_unknown_boardless_malformed_or_missing_swarm() {
    let read = read_over(Arc::new(OneBoard));
    for (swarm, error) in [
        (Some("C9"), "unknown swarm C9"),
        (Some("C2"), "swarm C2 hosts no board this session can read"),
        (
            Some("C1; echo secret"),
            "swarm must be an environment ref such as C1",
        ),
        (None, "swarm is required (an environment ref such as C1)"),
    ] {
        let response = wire(answer(Some(&read), Some("b1"), swarm).await);
        assert_eq!(
            (response["success"].clone(), response["error"].clone()),
            (json!(false), json!(error)),
            "{swarm:?}"
        );
    }
    let unavailable = wire(answer(None, None, Some("C1")).await);
    assert_eq!(
        unavailable["error"],
        json!("swarm boards are unavailable in this harness")
    );
}

fn sighting(run: &str, revision: i64) -> BoardSighting {
    BoardSighting::Board(HostedBoardRevision {
        run: Some(run.into()),
        revision,
    })
}

/// Each poll's hints against what was seen: a swarm first seen, moved or
/// made again (another run at the same revision, L5) is hinted; one no
/// longer read is hinted removed (L3); one that could not be read keeps
/// what was last seen and is not hinted (L3).
#[test]
fn a_swarm_is_hinted_when_first_seen_when_it_moves_and_when_it_goes() {
    let poll = |seen: &Seen, now: Vec<(&str, BoardSighting)>| {
        changes(
            seen,
            now.into_iter()
                .map(|(swarm, sighting)| (swarm.to_owned(), sighting))
                .collect::<BTreeMap<_, _>>(),
        )
    };
    let (lines, seen) = poll(
        &Seen::new(),
        vec![("C1", sighting("r1", 3)), ("C2", sighting("r2", 9))],
    );
    assert_eq!(lines, [hint("C1", "r1", 3), hint("C2", "r2", 9)]);
    let (lines, seen) = poll(
        &seen,
        vec![("C1", sighting("r1", 3)), ("C2", sighting("r2", 9))],
    );
    assert!(lines.is_empty(), "unchanged: no hint");
    let (lines, seen) = poll(
        &seen,
        vec![("C1", sighting("r1", 4)), ("C2", sighting("r2", 9))],
    );
    assert_eq!(lines, [hint("C1", "r1", 4)]);
    let (lines, seen) = poll(&seen, vec![("C1", sighting("r1", 4))]);
    assert_eq!(lines, [removal("C2")], "a board no longer read");
    let (lines, seen) = poll(
        &seen,
        vec![("C1", sighting("r1", 4)), ("C2", sighting("r2", 9))],
    );
    assert_eq!(
        lines,
        [hint("C2", "r2", 9)],
        "is hinted again when it reads"
    );
    let (lines, seen) = poll(
        &seen,
        vec![
            ("C1", BoardSighting::Unreadable("contended".into())),
            ("C2", sighting("r2", 9)),
        ],
    );
    assert!(lines.is_empty(), "a busy read neither forgets nor hints");
    let (lines, seen) = poll(
        &seen,
        vec![("C1", sighting("r1", 4)), ("C2", sighting("r2", 9))],
    );
    assert!(
        lines.is_empty(),
        "and the board is not hinted again after it"
    );
    let (lines, _) = poll(
        &seen,
        vec![("C1", sighting("r7", 4)), ("C2", sighting("r2", 9))],
    );
    assert_eq!(lines, [hint("C1", "r7", 4)], "a board made again");
}

/// M6: with a board that moves on every read, a hint every poll proves
/// the poll's rate: one at the first tick after a client connects, none
/// 999 ms later, the next at the second; no client, no poll.
#[tokio::test(start_paused = true)]
async fn hints_are_polled_at_most_once_a_second_and_only_with_a_client() {
    let revision = Arc::new(AtomicI64::new(1));
    let read = read_over(Arc::new(Moving {
        revision: revision.clone(),
        step: 1,
    }));
    let (tx, mut rx) = tokio::sync::broadcast::channel(16);
    let clients = Arc::new(AtomicU32::new(0));
    let _poll = AbortOnDrop(Some(spawn_change_hints(
        view(read),
        BoardHints::default(),
        tx,
        clients.clone(),
    )));
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert!(rx.try_recv().is_err(), "no client, no poll");
    assert_eq!(revision.load(Ordering::SeqCst), 1, "no board was read");
    clients.store(1, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(501)).await;
    let first = rx.try_recv().expect("the first tick with a client hints");
    assert!(first.contains("\"C1\""), "{first}");
    assert!(rx.try_recv().is_err(), "one hint per tick");
    tokio::time::sleep(Duration::from_millis(998)).await;
    assert!(rx.try_recv().is_err(), "nothing 999 ms after the hint");
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert!(rx.try_recv().is_ok(), "the next tick, a second after");
}

/// H1: a poll that finds no receiver (the last client went mid-tick)
/// keeps polling, and takes nothing as seen while nobody hears it: the
/// next client is told the board's revision, moved meanwhile.
#[tokio::test(start_paused = true)]
async fn a_client_leaving_mid_tick_does_not_end_the_hints() {
    let revision = Arc::new(AtomicI64::new(5));
    let read = read_over(Arc::new(Moving {
        revision: revision.clone(),
        step: 0,
    }));
    let (tx, rx) = tokio::sync::broadcast::channel(16);
    drop(rx);
    let hints = BoardHints::default();
    let _poll = AbortOnDrop(Some(spawn_change_hints(
        view(read),
        hints.clone(),
        tx.clone(),
        Arc::new(AtomicU32::new(1)),
    )));
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert!(
        hints.connect_lines().is_empty(),
        "nothing is taken as seen while nobody hears it"
    );
    revision.store(6, Ordering::SeqCst);
    let mut rx = tx.subscribe();
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert_eq!(rx.try_recv().unwrap(), hint("C1", "r1", 6));
}

/// M1: a client that connects after a board was hinted is told it on
/// connect: the boards' last-seen revisions, as connect-time lines.
#[tokio::test(start_paused = true)]
async fn a_late_client_is_told_every_board_already_seen() {
    let read = read_over(Arc::new(OneBoard));
    let (tx, mut rx) = tokio::sync::broadcast::channel(16);
    let hints = BoardHints::default();
    let _poll = AbortOnDrop(Some(spawn_change_hints(
        view(read),
        hints.clone(),
        tx,
        Arc::new(AtomicU32::new(1)),
    )));
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(rx.try_recv().unwrap(), hint("C1", "r1", 7));
    assert_eq!(hints.connect_lines(), [hint("C1", "r1", 7) + "\n"]);
}

/// M6: the poll ends when its owner, the accept loop, drops it.
#[tokio::test(start_paused = true)]
async fn the_poll_ends_with_its_owner() {
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let poll = spawn_change_hints(
        view(read_over(Arc::new(OneBoard))),
        BoardHints::default(),
        tx,
        Arc::new(AtomicU32::new(1)),
    );
    let ended = poll.abort_handle();
    drop(AbortOnDrop(Some(poll)));
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(ended.is_finished());
}

/// One connected client's reply channel, as the accept loop registers it.
fn client() -> (
    crate::interface::cli::uds_ext_protocol::ClientToolRegistry,
    tokio::sync::mpsc::Receiver<String>,
) {
    let clients = crate::interface::cli::uds_ext_protocol::new_client_tool_registry();
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    crate::interface::cli::uds_ext_protocol::register_client_writer(&clients, 7, tx);
    (clients, rx)
}

async fn reply(rx: &mut tokio::sync::mpsc::Receiver<String>) -> Value {
    let line = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("answered")
        .expect("a reply");
    serde_json::from_str(line.trim_end()).unwrap()
}

/// M6: `get_swarm_board` is answered on the connection's reader, a turn
/// running or not; any other line is left to the dispatch.
#[tokio::test]
async fn get_swarm_board_is_answered_on_the_reader() {
    let (clients, mut rx) = client();
    let boards = view(read_over(Arc::new(OneBoard)));
    let none: Option<SubagentRegistry> = None;
    let in_flight = super::board_requests();
    let ctx = || BoardViewCtx {
        boards: Some(&boards),
        clients: &clients,
        subagents: &none,
        client_id: 7,
        in_flight: &in_flight,
    };
    assert!(intercept(
        r#"{"type":"get_swarm_board","id":"b1","swarm":"C1"}"#,
        ctx()
    ));
    let answered = reply(&mut rx).await;
    assert_eq!(
        (
            answered["success"].clone(),
            answered["data"]["swarm"].clone()
        ),
        (json!(true), json!("C1"))
    );
    assert!(!intercept(r#"{"type":"get_state","id":"s1"}"#, ctx()));
    assert!(
        rx.try_recv().is_err(),
        "another command is not answered here"
    );
    let unavailable = BoardViewCtx {
        boards: None,
        clients: &clients,
        subagents: &none,
        client_id: 7,
        in_flight: &in_flight,
    };
    assert!(intercept(
        r#"{"type":"get_swarm_board","id":"b2","swarm":"C1"}"#,
        unavailable
    ));
    assert_eq!(
        reply(&mut rx).await["error"],
        json!("swarm boards are unavailable in this harness")
    );
}

/// M6: with `agent_id`, the board is asked of that descendant, routed as
/// `swarm_control` routes (here: no sub-agent registry to route through).
#[tokio::test]
async fn a_board_asked_of_a_descendant_is_routed_to_it() {
    let (clients, mut rx) = client();
    let boards = view(read_over(Arc::new(OneBoard)));
    let none: Option<SubagentRegistry> = None;
    let in_flight = super::board_requests();
    assert!(intercept(
        r#"{"type":"get_swarm_board","id":"b3","swarm":"C1","agent_id":"child-1"}"#,
        BoardViewCtx {
            boards: Some(&boards),
            clients: &clients,
            subagents: &none,
            client_id: 7,
            in_flight: &in_flight,
        }
    ));
    let answered = reply(&mut rx).await;
    assert_eq!(
        (
            answered["command"].clone(),
            answered["id"].clone(),
            answered["error"].clone()
        ),
        (
            json!("get_swarm_board"),
            json!("b3"),
            json!("no sub-agent registry available")
        )
    );
}

/// L7: a `get_swarm_board` whose fields are not strings is refused here,
/// at once, never queued behind the turn as a command.
#[tokio::test]
async fn a_malformed_get_swarm_board_is_refused_not_queued() {
    let (clients, mut rx) = client();
    let boards = view(read_over(Arc::new(OneBoard)));
    let none: Option<SubagentRegistry> = None;
    let in_flight = super::board_requests();
    for (line, id) in [
        (
            r#"{"type":"get_swarm_board","id":"b4","swarm":7}"#,
            json!("b4"),
        ),
        (
            r#"{"type":"get_swarm_board","id":5,"swarm":"C1"}"#,
            json!(null),
        ),
        (
            r#"{"type":"get_swarm_board","swarm":"C1","agent_id":["x"]}"#,
            json!(null),
        ),
    ] {
        assert!(
            intercept(
                line,
                BoardViewCtx {
                    boards: Some(&boards),
                    clients: &clients,
                    subagents: &none,
                    client_id: 7,
                    in_flight: &in_flight,
                }
            ),
            "{line}"
        );
        let refused = reply(&mut rx).await;
        assert_eq!(
            (
                refused["success"].clone(),
                refused["id"].clone(),
                refused["error"].clone()
            ),
            (
                json!(false),
                id,
                json!("get_swarm_board takes string fields: id, swarm and agent_id")
            ),
            "{line}"
        );
    }
}

/// M3: the board is read off the connection's reader, which takes its
/// next line while the read waits.
#[tokio::test]
async fn the_reader_is_not_held_while_a_board_is_read() {
    let (clients, mut rx) = client();
    let gate = Arc::new(tokio::sync::Notify::new());
    let boards = view(read_over(Arc::new(Gated(gate.clone()))));
    let none: Option<SubagentRegistry> = None;
    let in_flight = super::board_requests();
    assert!(intercept(
        r#"{"type":"get_swarm_board","id":"b5","swarm":"C1"}"#,
        BoardViewCtx {
            boards: Some(&boards),
            clients: &clients,
            subagents: &none,
            client_id: 7,
            in_flight: &in_flight,
        }
    ));
    tokio::task::yield_now().await;
    assert!(rx.try_recv().is_err(), "the read still waits");
    gate.notify_one();
    assert_eq!(reply(&mut rx).await["success"], json!(true));
}

/// Round 2 M1 (#2488): a `get_swarm_board` nested deeper than the JSON
/// reader goes (in a field the command ignores) is refused as malformed,
/// never a panic on the connection's reader, whichever route it names.
#[tokio::test]
async fn a_deeply_nested_get_swarm_board_does_not_panic_the_reader() {
    let (clients, mut rx) = client();
    let boards = view(read_over(Arc::new(OneBoard)));
    let none: Option<SubagentRegistry> = None;
    let in_flight = super::board_requests();
    for route in [r#","agent_id":"child-1""#, ""] {
        let deep = format!(
            r#"{{"type":"get_swarm_board","id":"b6","swarm":"C1"{route},"extra":{}{}}}"#,
            "[".repeat(200),
            "]".repeat(200)
        );
        assert!(intercept(
            &deep,
            BoardViewCtx {
                boards: Some(&boards),
                clients: &clients,
                subagents: &none,
                client_id: 7,
                in_flight: &in_flight,
            },
        ));
        let refused = reply(&mut rx).await;
        assert_eq!(
            (refused["id"].clone(), refused["error"].clone()),
            (json!("b6"), json!(MALFORMED)),
            "{route}"
        );
    }
}

/// Round 2 M2 (#2488): subscribing and reading what was seen happen under
/// the lock the poll publishes under, so a client sees every revision after
/// the ones it was told on connect: none twice, none missed, in order.
#[test]
fn a_client_is_told_each_revision_once_and_in_order() {
    let (tx, _keep) = tokio::sync::broadcast::channel::<String>(8_192);
    let hints = BoardHints::default();
    const LAST: i64 = 3_000;
    let publisher = {
        let (hints, tx) = (hints.clone(), tx.clone());
        std::thread::spawn(move || {
            for revision in 1..=LAST {
                hints.publish(&tx, |_| {
                    let now = Seen::from([(
                        "C1".to_owned(),
                        HostedBoardRevision {
                            run: Some("r1".into()),
                            revision,
                        },
                    )]);
                    (vec![hint("C1", "r1", revision)], now)
                });
            }
        })
    };
    let revision_of = |line: &str| -> i64 {
        serde_json::from_str::<Value>(line.trim_end()).unwrap()["revision"]
            .as_i64()
            .unwrap()
    };
    let mut subscribers = Vec::new();
    while !publisher.is_finished() {
        let (rx, lines) = hints.subscribe(&tx);
        let told = lines.first().map_or(0, |line| revision_of(line));
        subscribers.push((rx, told));
        std::thread::yield_now();
    }
    publisher.join().unwrap();
    assert!(!subscribers.is_empty());
    for (mut rx, told) in subscribers {
        let mut expected = told + 1;
        while let Ok(line) = rx.try_recv() {
            assert_eq!(revision_of(&line), expected, "told {told} on connect");
            expected += 1;
        }
        assert_eq!(
            expected,
            LAST + 1,
            "told {told}: every later revision arrived"
        );
    }
}

/// Round 2 M3 (#2488): at most two board requests are in flight per
/// connection; a third is refused as busy at once.
#[tokio::test]
async fn board_requests_in_flight_are_bounded_per_connection() {
    let (clients, mut rx) = client();
    let gate = Arc::new(tokio::sync::Notify::new());
    let boards = view(read_over(Arc::new(Gated(gate.clone()))));
    let none: Option<SubagentRegistry> = None;
    let in_flight = super::board_requests();
    let ask = |id: &str| {
        intercept(
            &format!(r#"{{"type":"get_swarm_board","id":"{id}","swarm":"C1"}}"#),
            BoardViewCtx {
                boards: Some(&boards),
                clients: &clients,
                subagents: &none,
                client_id: 7,
                in_flight: &in_flight,
            },
        )
    };
    assert!(ask("q1") && ask("q2") && ask("q3"));
    let busy = reply(&mut rx).await;
    assert_eq!(
        (busy["id"].clone(), busy["error"].clone()),
        (
            json!("q3"),
            json!("get_swarm_board is busy: at most 2 requests in flight per connection")
        )
    );
    // One permit per waiting read: no reliance on when each one waits.
    gate.notify_one();
    gate.notify_one();
    for _ in 0..2 {
        assert_eq!(reply(&mut rx).await["success"], json!(true));
    }
    assert!(ask("q4"), "a slot is free again");
    gate.notify_one();
    assert_eq!(reply(&mut rx).await["id"], json!("q4"));
}

/// Round 2 L5 (#2488): while no client is connected nothing seen is kept,
/// so a client connecting later is never told a stale revision.
#[tokio::test(start_paused = true)]
async fn what_was_seen_is_forgotten_while_no_client_is_connected() {
    let (tx, mut rx) = tokio::sync::broadcast::channel(16);
    let hints = BoardHints::default();
    let clients = Arc::new(AtomicU32::new(1));
    let _poll = AbortOnDrop(Some(spawn_change_hints(
        view(read_over(Arc::new(OneBoard))),
        hints.clone(),
        tx.clone(),
        clients.clone(),
    )));
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(rx.try_recv().unwrap(), hint("C1", "r1", 7));
    assert_eq!(hints.connect_lines().len(), 1);
    clients.store(0, Ordering::SeqCst);
    drop(rx);
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert!(
        hints.connect_lines().is_empty(),
        "nobody connected: forgotten"
    );
    let mut rx = tx.subscribe();
    clients.store(1, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert_eq!(rx.try_recv().unwrap(), hint("C1", "r1", 7), "hinted afresh");
}

/// A board whose projection names a `swarm` of its own.
struct Clashing;

impl HostedSwarmBoard for Clashing {
    fn board<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<SwarmBoardSnapshot>, String>> {
        Box::pin(async { Ok(SwarmBoardSnapshot::object(json!({"swarm": "C7"}))) })
    }

    fn revision<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<HostedBoardRevision>, String>> {
        Box::pin(async { Ok(None) })
    }
}

/// Round 2 L6 (#2488): an answer the view cannot present is refused to
/// the client, never a panic that leaves it unanswered.
#[tokio::test]
async fn an_answer_the_view_cannot_present_is_refused() {
    let read = read_over(Arc::new(Clashing));
    let refused = wire(answer(Some(&read), Some("b7"), Some("C1")).await);
    assert_eq!(
        (refused["success"].clone(), refused["error"].clone()),
        (
            json!(false),
            json!("swarm board answer malformed: it names a swarm of its own")
        )
    );
}

/// Verification item 2 (#2488): what was seen is forgotten under the lock,
/// and only while nobody is subscribed: a connection that has subscribed
/// (and been told what was seen) but is not yet counted keeps it.
#[test]
fn what_was_seen_is_forgotten_only_with_no_receiver() {
    let (tx, rx) = tokio::sync::broadcast::channel::<String>(4);
    let hints = BoardHints::default();
    hints.publish(&tx, |_| {
        let now = Seen::from([(
            "C1".to_owned(),
            HostedBoardRevision {
                run: Some("r1".into()),
                revision: 7,
            },
        )]);
        (vec![], now)
    });
    hints.forget(&tx);
    assert_eq!(hints.connect_lines().len(), 1, "a subscriber holds it");
    drop(rx);
    hints.forget(&tx);
    assert!(hints.connect_lines().is_empty());
}

/// Verification item 1 (#2488): a refusal (malformed or busy) is written
/// only when the client's channel has room, and dropped otherwise: no task
/// waits on a client that does not read.
#[tokio::test]
async fn refusals_to_a_client_that_does_not_read_are_dropped() {
    let clients = crate::interface::cli::uds_ext_protocol::new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    crate::interface::cli::uds_ext_protocol::register_client_writer(&clients, 7, tx.clone());
    tx.try_send("filler\n".to_owned()).unwrap();
    let boards = view(read_over(Arc::new(OneBoard)));
    let none: Option<SubagentRegistry> = None;
    let full = Arc::new(tokio::sync::Semaphore::new(0));
    for line in [
        r#"{"type":"get_swarm_board","id":"m","swarm":7}"#,
        r#"{"type":"get_swarm_board","id":"b","swarm":"C1"}"#,
    ] {
        for _ in 0..5 {
            assert!(intercept(
                line,
                BoardViewCtx {
                    boards: Some(&boards),
                    clients: &clients,
                    subagents: &none,
                    client_id: 7,
                    in_flight: &full,
                },
            ));
        }
    }
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert_eq!(rx.recv().await.unwrap(), "filler\n");
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert!(
        rx.try_recv().is_err(),
        "nothing waited behind the full channel"
    );
}
