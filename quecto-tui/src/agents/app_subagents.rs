use super::*;

pub(crate) fn usable_socket_path(path: Option<&str>) -> bool {
    path.is_some_and(|p| {
        let p = p.trim();
        let path = std::path::Path::new(p);
        if p.is_empty()
            || !path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            let Ok(metadata) = std::fs::symlink_metadata(path) else {
                return false;
            };
            metadata.file_type().is_socket() && !metadata.file_type().is_symlink()
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}

/// Grace window during which an unconfirmed optimistic subagent entry (created
/// from the spawn ToolStart, before the kernel registers the child) survives a
/// snapshot that omits it. Generous enough to bridge slow socket readiness, yet
/// bounded so a never-confirmed spawn (failed launch) can't linger forever (#866).
const OPTIMISTIC_SUBAGENT_GRACE: Duration = Duration::from_secs(30);

impl App {
    pub(super) fn delete_all_subagents(&mut self) {
        if !self.send_command(Command::DeleteAllSubagents {
            id: Some("delete-all-subagents".into()),
        }) {
            return;
        }
        self.subagents.tracked.clear();
        self.subagents.sessions.clear();
        self.subagents.session_order.clear();
        self.subagents.feeds.clear();
        self.subagents.active_agent_id = None;
        self.subagents.awaited_agent_id = None;
        self.subagents.panel_nav = crate::components::list_navigator::ListNavigator::new();
        self.notify("Deleting all subagents", NotifyLevel::Info);
    }

    /// Update a session's OWN footer (context-window / cost / model) from a
    /// forwarded sub-agent event, mirroring the master footer path (#805):
    /// `get_state` carries the model and window, `turn_end` the live context
    /// usage, and `get_session_stats` the cumulative cost (plus usage fallback).
    pub(super) fn update_session_footer(session: &mut SessionView, ev: &Event) {
        use crate::protocol::session_payloads;
        match ev {
            Event::Response {
                command,
                data: Some(data),
                success: true,
                ..
            } if command == "get_state" => {
                session.footer.apply_get_state(data);
            }
            Event::Response {
                command,
                data: Some(data),
                success: true,
                ..
            } if command == "get_session_stats" => {
                let stats = session_payloads::parse_session_stats(data);
                session.footer.apply_session_stats(&stats);
            }
            Event::TurnEnd { message } => {
                let used = message.get("contextTokens").and_then(|v| v.as_u64());
                let window = message
                    .get("maxContextTokens")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                if let (Some(used), Some(window)) = (used, window) {
                    session.footer.update_context_usage(used, window);
                }
            }
            _ => {}
        }
    }

    pub(super) fn update_subagent_bar(
        &mut self,
        subagents: Vec<crate::protocol::client::SubagentInfoEvent>,
    ) {
        self.update_subagent_bar_from_source(None, subagents);
    }

    pub(super) fn update_subagent_bar_from_source(
        &mut self,
        source_agent_id: Option<&str>,
        subagents: Vec<crate::protocol::client::SubagentInfoEvent>,
    ) {
        let source_agent_id = source_agent_id.map(sanitize_agent_id);
        let mut candidates = std::collections::BTreeMap::new();
        for mut s in subagents {
            if !usable_socket_path(s.socket_path.as_deref()) {
                s.socket_path = None;
            }
            candidates.insert(sanitize_agent_id(&s.agent_id), s);
        }

        crate::agents::roster::apply_roster_snapshot(
            &mut self.subagents.tracked,
            source_agent_id.as_deref(),
            candidates,
            tokio::time::Instant::now(),
            EXITED_SUBAGENT_GRACE,
            OPTIMISTIC_SUBAGENT_GRACE,
        );

        let warm_ids = self
            .subagents
            .tracked
            .iter()
            .filter(|(_, tracked)| usable_socket_path(tracked.info.socket_path.as_deref()))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in warm_ids {
            self.ensure_session(&id);
            self.ensure_synced_subagent_feed(&id);
        }
        self.enforce_warm_feed_cap();
        self.clamp_panel_selection();
    }

    /// Advance the subagent spinner animation. Returns `true` if a re-render is
    /// needed (i.e. at least one agent is active). Driven by the spinner tick so
    /// running agents animate and their elapsed-time clocks stay current.
    pub(super) fn tick_subagent_animation(&mut self) -> bool {
        // Advance while ANY tracked child is active OR the selected sub-agent is
        // mid-turn on its own child feed (#820): the selected session's
        // `running` flag is the source for its working spinner, and the
        // master's `subagent_local` status may still read idle for it.
        let any_active =
            self.active_subagent_running() || self.subagents.tracked_active_count() > 0;
        if !any_active {
            return false;
        }
        self.subagents.frame = self.subagents.frame.wrapping_add(1);
        // The sub-agent-first panel reads live state directly, so the only
        // consumer of `subagent_frame` is the "N working" activity line (#820
        // review). The frame bump above is all this tick needs.
        true
    }

    /// GC exited subagent bars whose grace period has elapsed (#540).
    /// Returns `true` if the bar was modified.
    pub(super) fn gc_exited_subagents(&mut self) -> bool {
        if self.subagents.tracked.is_empty() {
            return false;
        }
        gc_exited_subagents(
            &mut self.subagents.tracked,
            tokio::time::Instant::now(),
            EXITED_SUBAGENT_GRACE,
        )
    }
}
