use super::*;

use crate::shell::socket_path::usable_socket_path;

/// Grace window during which an unconfirmed optimistic subagent entry (created
/// from the spawn ToolStart, before the kernel registers the child) survives a
/// snapshot that omits it. Generous enough to bridge slow socket readiness, yet
/// bounded so a never-confirmed spawn (failed launch) can't linger forever (#866).
const OPTIMISTIC_SUBAGENT_GRACE: Duration = Duration::from_secs(30);

impl App {
    pub(super) fn delete_all_subagents(&mut self) {
        if !self.send_command(Command::DeleteAllSubagents {
            id: Some(self.ac().namespaced_id("delete-all-subagents")),
        }) {
            return;
        }
        self.ac_mut().roster.tracked.clear();
        self.ac_mut().roster.sessions.clear();
        self.ac_mut().roster.session_order.clear();
        for (_, feed) in std::mem::take(&mut self.ac_mut().roster.feeds) {
            feed.handle.abort();
        }
        self.ac_mut().roster.active_agent_id = None;
        self.ac_mut().roster.selected_environment = None;
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

        if let Some(session) = self.ac_mut().roster.sessions.remove(from) {
            self.ac_mut()
                .roster
                .sessions
                .entry(to.to_string())
                .or_insert(session);
        }

        if let Some(feed) = self.ac_mut().roster.feeds.remove(from) {
            if self.ac().roster.feeds.contains_key(to) {
                feed.handle.abort();
            } else {
                self.ac_mut().roster.feeds.insert(to.to_string(), feed);
            }
        }

        let mut saw_to = false;
        self.ac_mut().roster.session_order.retain_mut(|id| {
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

        for pending in self.ac_mut().pending_message_recovery.values_mut() {
            if pending.agent_id.as_deref() == Some(from) {
                pending.agent_id = Some(to.to_string());
            }
        }
        for batch in self.ac_mut().message_recovery_batches.values_mut() {
            if batch.agent_id.as_deref() == Some(from) {
                batch.agent_id = Some(to.to_string());
            }
        }

        if self.ac().roster.active_agent_id.as_deref() == Some(from) {
            self.ac_mut().roster.active_agent_id = Some(to.to_string());
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

    pub(super) fn merge_subagent_bar_delta(
        &mut self,
        subagents: Vec<crate::protocol::client::SubagentInfoEvent>,
    ) {
        let mut snapshot = self
            .ac()
            .roster
            .tracked
            .values()
            .map(|tracked| tracked.info.clone())
            .collect::<Vec<_>>();
        snapshot.extend(subagents);
        self.update_subagent_bar(snapshot);
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
            for (identity, info) in resolve_roster_identities(&self.ac().roster.tracked, s) {
                candidates.insert(identity, info);
            }
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
            if let Some(entry) = self.ac().roster.tracked.get(&display_key) {
                if entry.optimistic && !self.ac().roster.tracked.contains_key(uuid_key) {
                    if let Some(mut migrated) = self.ac_mut().roster.tracked.remove(&display_key) {
                        migrated.info.agent_uuid = Some(uuid_key.clone());
                        if migrated.info.display_name.is_none() {
                            migrated.info.display_name = Some(display_key.clone());
                        }
                        self.ac_mut()
                            .roster
                            .tracked
                            .insert(uuid_key.clone(), migrated);
                        pending_rekeys.push((display_key, uuid_key.clone()));
                    }
                }
            }
        }
        for (from, to) in pending_rekeys {
            self.rekey_agent_collections(&from, &to);
        }

        let roster = &mut self.ac_mut().roster;
        crate::agents::roster::apply_roster_snapshot(
            &mut roster.tracked,
            source_agent_id.as_deref(),
            candidates,
            crate::agents::roster::RosterApplyTiming {
                now: tokio::time::Instant::now(),
                exited_grace: EXITED_SUBAGENT_GRACE,
                optimistic_grace: OPTIMISTIC_SUBAGENT_GRACE,
            },
            &mut roster.expired_terminal_uuids,
        );

        let warm_ids = self
            .ac()
            .roster
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
            self.active_subagent_running() || self.ac().roster.tracked_active_count() > 0;
        if !any_active {
            return false;
        }
        self.ac_mut().roster.frame = self.ac_mut().roster.frame.wrapping_add(1);
        // The sub-agent-first panel reads live state directly, so the only
        // consumer of `subagent_frame` is the "N working" activity line (#820
        // review). The frame bump above is all this tick needs.
        true
    }

    /// GC exited subagent bars whose grace period has elapsed (#540).
    /// Returns `true` if the bar was modified.
    pub(super) fn gc_exited_subagents(&mut self) -> bool {
        if self.ac().roster.tracked.is_empty() {
            return false;
        }
        let roster = &mut self.ac_mut().roster;
        gc_exited_subagents(
            &mut roster.tracked,
            tokio::time::Instant::now(),
            EXITED_SUBAGENT_GRACE,
            &mut roster.expired_terminal_uuids,
        )
    }
}

/// Compact `get_subagents` rows are keyed by display `agentId` on legacy
/// kernels and by `agentUuid` on current kernels. Rematch a legacy compact
/// label onto existing tracked UUIDs. When a legacy label is ambiguous, apply
/// the sparse status update to each matching row instead of collapsing multiple
/// durable identities into one display-name key.
fn resolve_roster_identities<I: crate::agents::roster::RosterInfo>(
    tracked: &std::collections::BTreeMap<String, crate::agents::roster::TrackedSubagent<I>>,
    info: I,
) -> Vec<(String, I)> {
    if let Some(uuid) = info
        .agent_uuid()
        .map(sanitize_agent_id)
        .filter(|value| !value.is_empty())
    {
        return vec![(uuid, info)];
    }
    let label = sanitize_agent_id(info.display_label());
    let is_live_match = |entry: &crate::agents::roster::TrackedSubagent<I>| {
        !crate::agents::roster::subagent_status_is_terminal(entry.info.status())
    };
    if tracked.get(&label).is_some_and(is_live_match) {
        return vec![(label, info)];
    }
    let matches = tracked
        .iter()
        .filter(|(_, entry)| {
            is_live_match(entry) && sanitize_agent_id(entry.info.display_label()) == label
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    if !matches.is_empty() {
        return matches
            .into_iter()
            .map(|id| (id, info.clone()))
            .collect::<Vec<_>>();
    }
    if tracked.values().any(|entry| {
        crate::agents::roster::subagent_status_is_terminal(entry.info.status())
            && sanitize_agent_id(entry.info.display_label()) == label
    }) {
        Vec::new()
    } else {
        vec![(label, info)]
    }
}
