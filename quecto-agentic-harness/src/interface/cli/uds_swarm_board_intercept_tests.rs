//! The board view's reader path (#2478): `get_swarm_board` taken off the
//! connection's reader, refused, bounded and routed; and the hints a new
//! connection is told. Split from `uds_swarm_board_tests.rs` for the line
//! ceiling.
use super::tests::{Gated, OneBoard, hint, read_over, view};
use super::{BoardHints, BoardViewCtx, MALFORMED, Seen, intercept};
use crate::application::environments::dto::HostedBoardRevision;
use crate::infrastructure::tools::agents::subagent_registry::SubagentRegistry;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

/// One connected client's reply channel, as the accept loop registers it.
fn client() -> (
    crate::interface::uds::controllers::uds_ext_protocol::ClientToolRegistry,
    tokio::sync::mpsc::Receiver<String>,
) {
    let clients = crate::interface::uds::controllers::uds_ext_protocol::new_client_tool_registry();
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    crate::interface::uds::controllers::uds_ext_protocol::register_client_writer(&clients, 7, tx);
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
    // One permit per read, each given once the last was answered: a permit
    // given before a read waits is kept for it, so nothing relies on when
    // each one starts waiting, nor on the runtime's flavour.
    for _ in 0..2 {
        gate.notify_one();
        assert_eq!(reply(&mut rx).await["success"], json!(true));
    }
    assert!(ask("q4"), "a slot is free again");
    gate.notify_one();
    assert_eq!(reply(&mut rx).await["id"], json!("q4"));
}

/// Verification item 1 (#2488): a refusal (malformed or busy) is written
/// only when the client's channel has room, and dropped otherwise: no task
/// waits on a client that does not read.
#[tokio::test]
async fn refusals_to_a_client_that_does_not_read_are_dropped() {
    let clients = crate::interface::uds::controllers::uds_ext_protocol::new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    crate::interface::uds::controllers::uds_ext_protocol::register_client_writer(
        &clients,
        7,
        tx.clone(),
    );
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
