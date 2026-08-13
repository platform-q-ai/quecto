//! Fan-in seam driving for the headless harness (#1462, epic #1467).
//!
//! Drives events through the sourced fan-in path the event loop drains now
//! that the master connection lives behind a feed task. At N=1, `event()`
//! keeps meaning "the (only) tab's master" — these drivers pin that the
//! fan-in path renders identically, and the `wire_*` drivers exercise the
//! FULL production flow: real socket → client reader → connection feed task
//! → dedicated tab fan-in → `route_sourced`.

use super::TuiHarness;
use crate::protocol::client::Event;
use crate::shell::connection::{SourcedEvent, TabId};

/// Bounded wait for fan-in delivery so a broken feed task fails a test
/// quickly instead of hanging it.
const PUMP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl TuiHarness {
    /// Route one fan-in item through the production `route_sourced` and,
    /// when a `Closed` sentinel deferred its #1047 diagnosis off-loop,
    /// synchronously complete it the way the event loop's diagnosis arm
    /// would. The SINGLE completion protocol every sourced driver shares
    /// (#1470 review — previously triplicated).
    async fn route_sourced_item(&mut self, item: SourcedEvent) {
        self.app.route_sourced(item);
        if self.app.conn.disconnect_diag_pending {
            self.pump_disconnect_diagnosis().await;
        }
    }

    /// Deliver a master-connection event through the sourced fan-in routing
    /// (`SourcedEvent::Tab(MASTER, ..)`) and capture the resulting frame.
    pub async fn sourced_master_event(&mut self, ev: Event) -> &mut Self {
        self.route_sourced_item(SourcedEvent::Tab(TabId::MASTER, ev))
            .await;
        self.capture();
        self
    }

    /// Deliver a sub-agent event through the sourced fan-in routing
    /// (`SourcedEvent::Subagent(MASTER, agent_id, ..)`) and capture.
    pub async fn sourced_subagent_event(&mut self, agent_id: &str, ev: Event) -> &mut Self {
        self.route_sourced_item(SourcedEvent::Subagent(
            TabId::MASTER,
            agent_id.to_string(),
            ev,
        ))
        .await;
        self.capture();
        self
    }

    /// Deliver the master connection's explicit `Closed` sentinel (#1462) —
    /// the fan-in replacement for `None`-from-recv — and capture. When a
    /// child-exit watch is attached, the production routing defers the
    /// #1047 diagnosis off-loop; the shared completion in
    /// [`Self::route_sourced_item`] resolves it before capturing.
    pub async fn deliver_closed_sentinel(&mut self) -> &mut Self {
        self.route_sourced_item(SourcedEvent::Closed(TabId::MASTER))
            .await;
        self.capture();
        self
    }

    /// Attach a real child-exit watcher, then deliver the `Closed` sentinel
    /// — the diagnosis (#1047) must survive the seam unchanged.
    pub async fn deliver_closed_sentinel_with_child_watch(
        &mut self,
        watch: crate::shell::child_watch::ChildWatch,
    ) -> &mut Self {
        self.app.set_child_exit_watch(watch);
        self.deliver_closed_sentinel().await
    }

    // ── Real-wire driving (#1462 falsifiability) ──────────────────────
    // These do NOT call `route_sourced` with a hand-built key: the event
    // travels the production path end-to-end, so a regression that leaves
    // the fan-in undrained or the feed task unspawned fails these tests.

    /// Write a raw event line on the agent side of the REAL master socket,
    /// then pump ONE item from the app's fan-in channels through the
    /// production routing, and capture the frame.
    pub async fn wire_master_event_line(&mut self, json: &str) -> &mut Self {
        self.send_agent_event_line(json).await;
        self.pump_sourced().await;
        self.capture();
        self
    }

    /// Write a raw event line on the agent side WITHOUT pumping — for tests
    /// that let `run()`'s own select arms drain the fan-in (#1470).
    pub async fn send_agent_event_line(&mut self, json: &str) {
        self.agent_event_tx
            .as_ref()
            .expect("agent side already closed")
            .send(format!("{json}\n"))
            .await
            .expect("write event line on the agent side");
    }

    /// Close the agent side of the REAL master socket (EOF to the client),
    /// then pump the resulting fan-in item — the feed task's `Closed`
    /// sentinel — through the production routing.
    pub async fn wire_close_master_connection(&mut self) -> &mut Self {
        drop(
            self.agent_event_tx
                .take()
                .expect("agent side already closed"),
        );
        self.pump_sourced().await;
        self.capture();
        self
    }

    /// Drain ONE item from the app's fan-in channels — the exact receivers
    /// the event loop's select arms drain, with the same unbiased select —
    /// and route it through the shared completion protocol.
    pub async fn pump_sourced(&mut self) {
        let item = tokio::time::timeout(PUMP_TIMEOUT, async {
            // Unbiased, matching run()'s select arms — the harness must not
            // assert an inter-channel ordering production does not guarantee
            // (#1470 r3).
            tokio::select! {
                Some(item) = self.app.tab_event_rx.recv() => item,
                Some(item) = self.app.subagents.event_rx.recv() => item,
            }
        })
        .await
        .expect("an item must arrive on a fan-in channel (#1462)");
        self.route_sourced_item(item).await;
    }

    /// Await the off-loop disconnect diagnosis (#1462 scope 3) and finish
    /// the stream-closed disconnect with it — the harness stand-in for the
    /// event loop's disconnect-diagnosis select arm.
    pub async fn pump_disconnect_diagnosis(&mut self) {
        let (tab, detail) = tokio::time::timeout(PUMP_TIMEOUT, self.app.disconnect_diag_rx.recv())
            .await
            .expect("the off-loop disconnect diagnosis must complete (#1462)")
            .expect("the disconnect diagnosis channel must stay open");
        self.app.finish_agent_stream_closed(tab, detail);
    }
}
