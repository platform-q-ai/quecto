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
        self.subagents.selected_environment = None;
        self.subagents.panel_nav = crate::components::list_navigator::ListNavigator::new();
        self.subagents.panel_nav_key = Some("master".to_string());
        self.notify("Deleting all subagents", NotifyLevel::Info);
    }

    /// Move every agent-keyed collection from `from` → `to` together (#1378).
    /// Keeps dual-identity gaps from orphaning sessions/feeds when an optimistic
    /// display row collapses onto a durable UUID. No-op when keys match.
    ///
    /// When `to` already holds a value, that destination wins and the `from`
    /// value is dropped (feeds abort) so we never create dual rows.
    pub(super) fn rekey_agent_collections(&mut self, from: &str, to: &str) {
        if from == to || from.is_empty() || to.is_empty() {
            return;
        }

        if let Some(session) = self.subagents.sessions.remove(from) {
            self.subagents
                .sessions
                .entry(to.to_string())
                .or_insert(session);
        }

        if let Some(feed) = self.subagents.feeds.remove(from) {
            if self.subagents.feeds.contains_key(to) {
                feed.handle.abort();
            } else {
                self.subagents.feeds.insert(to.to_string(), feed);
            }
        }

        let mut saw_to = false;
        self.subagents.session_order.retain_mut(|id| {
            if id == from {
                if saw_to {
                    return false;
                }
                *id = to.to_string();
                saw_to = true;
                true
            } else if id == to {
                if saw_to {
                    return false;
                }
                saw_to = true;
                true
            } else {
                true
            }
        });

        if self.subagents.active_agent_id.as_deref() == Some(from) {
            self.subagents.active_agent_id = Some(to.to_string());
        }
        let from_key = format!("agent:{from}");
        if self.subagents.panel_nav_key.as_deref() == Some(from_key.as_str()) {
            self.subagents.panel_nav_key = Some(format!("agent:{to}"));
        }

        self.sync_panel_selection_to_active();
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
                let payload = crate::protocol::presentation_payloads::parse_turn_end(message);
                let used = payload.context_tokens;
                let window = payload.max_context_tokens;
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
            let identity = s.agent_uuid.as_deref().unwrap_or(&s.agent_id);
            candidates.insert(sanitize_agent_id(identity), s);
        }

        // #1378: if an optimistic spawn row is still keyed by display label
        // while the authoritative snapshot arrives under UUID, migrate the
        // optimistic entry onto the UUID key before merge so we never keep
        // dual rows for the optimistic grace window. Move sessions/feeds/
        // session_order/active with the tracked key.
        let mut pending_rekeys = Vec::new();
        for (uuid_key, info) in &candidates {
            let display = info
                .display_name
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(info.agent_id.as_str());
            let display_key = sanitize_agent_id(display);
            if display_key == *uuid_key {
                continue;
            }
            if let Some(entry) = self.subagents.tracked.get(&display_key) {
                if entry.optimistic && !self.subagents.tracked.contains_key(uuid_key) {
                    if let Some(mut migrated) = self.subagents.tracked.remove(&display_key) {
                        migrated.info.agent_uuid = Some(uuid_key.clone());
                        if migrated.info.display_name.is_none() {
                            migrated.info.display_name = Some(display_key.clone());
                        }
                        self.subagents.tracked.insert(uuid_key.clone(), migrated);
                        pending_rekeys.push((display_key, uuid_key.clone()));
                    }
                }
            }
        }
        for (from, to) in pending_rekeys {
            self.rekey_agent_collections(&from, &to);
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
