//! Harness drivers: the manual clock, overlay-close seam, frame/selector
//! probes and sub-agent delivery fixtures.

use super::TuiHarness;

impl TuiHarness {
    /// Run the production seam every session switch closes overlays
    /// through (`close_session_switch_overlays`). With a modal open the switch
    /// keys go to the modal, so a test of "the picker was closed by a switch"
    /// drives the seam itself.
    pub fn close_overlays_for_session_switch(&mut self) -> &mut Self {
        self.app.close_session_switch_overlays();
        self.capture();
        self
    }

    /// The harness's manual clock (#2010 R3-T1): it stands still until a
    /// test says time passed, so no deadline depends on how long a step took.
    pub fn now(&self) -> tokio::time::Instant {
        self.app.clock.now()
    }

    /// Move the clock without waking anything: what an answer that beats the
    /// idle tick finds.
    pub fn advance_clock(&mut self, by: std::time::Duration) -> &mut Self {
        self.app.clock.advance(by);
        self
    }

    /// `by` passes with no input and no answer, then the idle loop's timeout
    /// service wakes, as its deadline arm would (#2010 R2-T3).
    pub fn pass_time(&mut self, by: std::time::Duration) -> &mut Self {
        self.app.clock.advance(by);
        self.app.service_search_timeout(self.app.clock.now());
        self.capture();
        self
    }

    /// Feed one raw key byte-sequence through the production parser and key
    /// handler.
    pub fn press_raw(&mut self, bytes: &[u8]) -> &mut Self {
        let (key, _) = crate::shell::keys::parse_key(bytes).expect("parseable key sequence");
        self.app.suppress_paint = true;
        self.app.handle_key(key);
        self
    }

    /// Track a sub-agent roster entry with `status` on the connection.
    pub fn track_subagent(&mut self, id: &str, status: &str) -> &mut Self {
        self.app
            .update_subagent_bar(vec![crate::protocol::client::SubagentInfoEvent {
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
            }]);
        self
    }

    /// The active session's most recent Status chat line, if any.
    pub fn last_status_line(&self) -> Option<String> {
        self.app
            .active_session()
            .chat
            .last_status_text()
            .map(str::to_string)
    }

    /// The most recent notification's message text, if any.
    pub fn last_notification(&self) -> Option<String> {
        self.app.notifications.messages().last().cloned()
    }
}

// ── #1466 round-2 fix-pass drivers/probes ────────────────────────────────

impl TuiHarness {
    /// Compose the full frame through the production path (ANSI intact).
    pub fn frame_lines(&mut self) -> Vec<String> {
        self.app.compose_frame()
    }

    /// The harness terminal's height in rows.
    pub fn terminal_height(&self) -> usize {
        self.app.terminal.height
    }

    /// Whether the tool-policy selector has been requested (its catalogue
    /// fetch is in flight).
    pub fn tool_policy_selector_requested(&self) -> bool {
        self.app.tool_policy_modal_pending_catalogue_id.is_some()
    }

    /// Focus a running sub-agent restored by a session resume: focused BEFORE
    /// its live socket was known, whose socket then becomes reachable — the
    /// #1466 round-2 field state behind "not attached" send failures.
    pub fn focus_restored_running_subagent(&mut self, id: &str) {
        self.track_subagent(id, "running");
        self.app.select_agent(Some(id));
        let socket = super::events::spawn_subagent_socket(id);
        self.app
            .ac_mut()
            .roster
            .tracked
            .get_mut(id)
            .expect("tracked sub-agent")
            .info
            .socket_path = Some(socket.to_string_lossy().into_owned());
    }

    /// Focus a sub-agent still marked detached whose registry socket is live.
    pub fn focus_detached_reachable_subagent(&mut self, id: &str) {
        let socket = super::events::spawn_subagent_socket(id);
        self.app
            .update_subagent_bar(vec![super::events::subagent_with_socket(
                id,
                "detached",
                None,
                Some(socket),
            )]);
        self.app.select_agent(Some(id));
    }

    /// Focus a dead sub-agent (no reachable socket).
    pub fn focus_dead_subagent(&mut self, id: &str) {
        self.track_subagent(id, "dead");
        self.app.select_agent(Some(id));
    }

    /// User entries in the ACTIVE session's transcript.
    pub fn active_user_entries(&self) -> Vec<String> {
        self.app
            .active_session()
            .chat
            .entries()
            .iter()
            .filter_map(|e| match e {
                crate::components::chat::ChatEntry::User { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }
}
