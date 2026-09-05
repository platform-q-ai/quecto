//! #1605 investigation: real direct sockets, not root-routed inspection feeds.
//! These are characterization probes until the reported larger-roster freeze
//! is reproduced; passing them is not evidence that the production bug is fixed.

use super::app_event_loop::StreamRenderCoalescer;
use super::tui_harness::{self, TuiHarness};
use crate::components::chat::ChatEntry;
use crate::components::component::Component;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

struct Child {
    id: String,
    events: mpsc::Sender<Value>,
    commands: mpsc::Receiver<Value>,
    _dir: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Child {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Child {
    fn new(index: usize) -> (Self, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("child.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let (events, mut event_rx) = mpsc::channel::<Value>(256);
        let (command_tx, commands) = mpsc::channel(256);
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = socket.into_split();
            let mut reader = BufReader::new(reader);
            // Keep frame reads alive while writing events: cancelling a partial
            // read at each write would make this fixture corrupt its own wire.
            tokio::select! {
                _ = async {
                    while let Some(event) = event_rx.recv().await {
                        let line = format!("{event}\n");
                        if writer.write_all(line.as_bytes()).await.is_err() { break; }
                    }
                } => {},
                _ = async {
                    while let Ok(Some(frame)) = quecto_line_io::read_frame_or_legacy_line(
                        &mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
                    ).await {
                        let (quecto_line_io::Incoming::Frame(bytes)
                            | quecto_line_io::Incoming::LegacyLine(bytes)) = frame;
                        if !bytes.is_empty() {
                            let command = serde_json::from_slice(&bytes).unwrap();
                            if command_tx.send(command).await.is_err() { break; }
                        }
                    }
                } => {},
            }
        });
        (
            Self {
                id: format!("child-{index:02}"),
                events,
                commands,
                _dir: dir,
                task,
            },
            path,
        )
    }

    async fn send(&self, event: Value) {
        self.events.send(event).await.unwrap();
    }

    async fn command(&mut self, kind: &str) -> Value {
        let command = tokio::time::timeout(Duration::from_secs(5), self.commands.recv())
            .await
            .expect("child command deadline")
            .expect("child socket alive");
        assert_eq!(command["type"], kind);
        command
    }
}

async fn pump(h: &mut TuiHarness, count: usize, paint: &mut StreamRenderCoalescer) {
    for _ in 0..count {
        let app = h.app_mut();
        let item = tokio::time::timeout(Duration::from_secs(5), app.subagents.event_rx.recv())
            .await
            .expect("direct fan-in deadline")
            .expect("direct fan-in alive");
        let decision = app.route_sourced(item);
        app.apply_sourced_render(decision, paint);
    }
}

fn delta(rev: u64, messages: Value) -> Value {
    json!({"type":"response", "command":"sync", "success":true,
        "data":{"epoch":1,"rev":rev,"messages":messages,"caughtUp":true,
            "resync":false,"nextRev":null}})
}

async fn scenario(roster_size: usize, live_count: usize, nested: bool) {
    let mut h = TuiHarness::new().await;
    h.app_mut().suppress_paint = true;
    let mut children = Vec::new();
    let mut roster = Vec::new();
    for index in 0..roster_size {
        let (child, socket) = Child::new(index);
        let mut info = tui_harness::subagent_with_socket(
            &child.id,
            if index < live_count {
                "running"
            } else {
                "dead"
            },
            None,
            (index < live_count).then_some(socket),
        );
        info.agent_uuid = Some(child.id.clone());
        if nested && index > 0 {
            info.parent_id = Some("child-00".into());
        }
        roster.push(info);
        children.push(child);
    }
    h.event(tui_harness::subagents_changed(roster));
    let focused = children[live_count - 1].id.clone();
    h.select(Some(&focused));
    assert_eq!(h.app_mut().ac().roster.tracked.len(), roster_size);
    assert_eq!(h.app_mut().ac().roster.feeds.len(), live_count);
    let mut paint = StreamRenderCoalescer::default();
    for child in &mut children[..live_count] {
        assert!(!h.app_mut().ac().roster.feeds[&child.id].inspection_only);
        child.command("get_state").await;
        child.command("sync").await;
        child
            .send(
                json!({"type":"response","command":"get_state","success":true,
            "data":{"capabilities":{"sync":1}}}),
            )
            .await;
        let history = (0..40)
            .map(|i| {
                json!({"id":format!("history-{i}"),
            "role":"user","content":format!("history line {i}")})
            })
            .collect::<Vec<_>>();
        child.send(delta(1, json!(history))).await;
        child.send(json!({"type":"turn_start"})).await;
    }
    pump(&mut h, live_count * 3, &mut paint).await;

    // Identical producing workload per live child. Check the focused transcript
    // and rendered frame before any completion or focus-change catch-up.
    for chunk in 0..12 {
        for child in &children[..live_count] {
            child
                .send(json!({"type":"token","token":format!("{}-part{chunk:02}\n", child.id)}))
                .await;
        }
        pump(&mut h, live_count, &mut paint).await;
        let marker = format!("{focused}-part{chunk:02}");
        assert!(
            h.full_frame().contains(&marker),
            "focused token missing: roster={roster_size}, live={live_count}, nested={nested}"
        );
    }
    // Exercise the same direct sync path with the viewport deliberately above
    // the tail, then verify that returning to the tail resumes following.
    let chat = &mut h
        .app_mut()
        .ac_mut()
        .roster
        .sessions
        .get_mut(&focused)
        .unwrap()
        .chat;
    chat.scroll_up(20);
    let scrolled = chat.render(80);
    let body = |id: &str| {
        (0..12)
            .map(|i| format!("{id}-part{i:02}\n"))
            .collect::<String>()
    };
    for child in &children[..live_count] {
        child
            .send(json!({"type":"ledger_advanced","epoch":1,"rev":2}))
            .await;
    }
    pump(&mut h, live_count, &mut paint).await;
    for child in &mut children[..live_count] {
        let command = child.command("sync").await;
        assert_eq!(command["sinceRev"], 1);
        child
            .send(delta(
                2,
                json!([{"id":"answer","role":"assistant","content":body(&child.id)}]),
            ))
            .await;
        child.send(json!({"type":"turn_end","message":{}})).await;
    }
    pump(&mut h, live_count * 2, &mut paint).await;
    assert_eq!(h.app_mut().ac().roster.feeds[&focused].rev, 2);
    let before = h.app_mut().ac().roster.sessions[&focused]
        .chat
        .entries()
        .to_vec();
    assert_eq!(
        before
            .iter()
            .filter(|entry| matches!(entry,
        ChatEntry::Assistant { text, .. } if text == &body(&focused)))
            .count(),
        1
    );
    let chat = &mut h
        .app_mut()
        .ac_mut()
        .roster
        .sessions
        .get_mut(&focused)
        .unwrap()
        .chat;
    assert_eq!(
        chat.render(80),
        scrolled,
        "direct sync must preserve scrolled viewport"
    );
    assert!(chat.scroll_offset() > 0);
    chat.scroll_down(usize::MAX);
    assert!(chat.render(80).join("\n").contains("part11"));
    assert_eq!(chat.scroll_offset(), 0);
    h.select(None).select(Some(&focused));
    assert_eq!(
        format!(
            "{:?}",
            h.app_mut().ac().roster.sessions[&focused].chat.entries()
        ),
        format!("{before:?}")
    );
}

#[tokio::test]
async fn issue_1605_direct_roster_workload_matrix() {
    for size in [1, 4, 5, 8, 16, 30] {
        let mut live_counts = vec![1, size.min(4), size];
        live_counts.sort_unstable();
        live_counts.dedup();
        for live in live_counts {
            for nested in [false, true] {
                scenario(size, live, nested).await;
            }
        }
    }
}
