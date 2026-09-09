//! Inference-admission observability in the TUI (#1679 P4): the master's
//! waiting/cooldown label rides on the footer and the working spinner; a
//! forwarded child view paints a compact label on that child's panel row.
//! Admission never changes a lifecycle state: no session is marked idle or
//! stalled by it.

use super::super::*;
use crate::protocol::admission_payloads::{AdmissionView, parse_admission};

pub(in crate::shell::app) const WORKING_MESSAGE: &str = "Working... (Esc to interrupt)";

impl App {
    /// Resolve direct-connection identity before applying shared admission order.
    /// Return true only when the event was consumed by admission handling.
    pub(in crate::shell::app) fn route_child_admission_event(
        &mut self,
        agent_id: &str,
        event: &Event,
    ) -> bool {
        match event {
            Event::AdmissionStateChanged {
                agent_id: inner_id,
                admission,
            } => {
                let target = match inner_id.as_deref() {
                    None | Some("") => agent_id,
                    Some(id) => id,
                };
                if let Some(view) =
                    parse_admission(admission, &crate::components::ansi::sanitize_control)
                {
                    self.apply_child_admission(target, &view);
                }
                true
            }
            Event::AgentEnd { .. } | Event::TurnEnd { .. } => {
                self.clear_child_admission_wait(agent_id);
                false
            }
            _ => false,
        }
    }

    /// `admission_state_changed`: `None`/own id → master; a child id → roster.
    pub(in crate::shell::app) fn handle_admission_state(
        &mut self,
        agent_id: Option<&str>,
        admission: &serde_json::Value,
    ) {
        let Some(view) = parse_admission(admission, &crate::components::ansi::sanitize_control)
        else {
            return;
        };
        let own = agent_id
            .is_none_or(|id| id.is_empty() || self.ac().connected_agent_id.as_deref() == Some(id));
        if own {
            self.apply_master_admission(&view);
        } else if let Some(id) = agent_id {
            self.apply_child_admission(id, &view);
        }
    }

    /// `get_state`: the slim state carries the view whenever the agent shares
    /// an authority, so its absence means "no admission here" and clears a
    /// label left over from another agent or an earlier connection.
    pub(in crate::shell::app) fn apply_get_state_admission(
        &mut self,
        view: Option<&AdmissionView>,
    ) {
        match view {
            Some(view) => self.apply_master_admission(view),
            None => self.clear_master_admission(),
        }
    }

    pub(in crate::shell::app) fn apply_master_admission(&mut self, view: &AdmissionView) {
        let label = view.status_label();
        let waiting = view.waiting > 0;
        self.ac_mut().admission_observed_at = tokio::time::Instant::now();
        self.ac_mut().admission_view = Some(view.clone());
        self.ac_mut()
            .master_session
            .footer
            .set_admission(label.clone(), view.compact_label());
        match label.filter(|_| waiting) {
            Some(label) => {
                let message = format!("⏳ {} (Esc to interrupt)", capitalize(&label));
                if let Some(spinner) = &mut self.ac_mut().spinner {
                    spinner.set_message(&message);
                }
                self.ac_mut().admission_spinner_message = Some(message);
            }
            None => self.restore_spinner_after_wait(),
        }
    }

    /// Put the plain working message back, but only over a message this
    /// module wrote: a tool's own "Spawning reviewer..." is left alone.
    fn restore_spinner_after_wait(&mut self) {
        let Some(ours) = self.ac_mut().admission_spinner_message.take() else {
            return;
        };
        if let Some(spinner) = &mut self.ac_mut().spinner
            && spinner.message() == ours
        {
            spinner.set_message(WORKING_MESSAGE);
        }
    }

    /// No admission view any more (disconnect, or an agent without an
    /// authority): drop the label and release the spinner message.
    pub(in crate::shell::app) fn clear_master_admission(&mut self) {
        self.ac_mut().admission_view = None;
        self.ac_mut()
            .master_session
            .footer
            .set_admission(None, None);
        self.restore_spinner_after_wait();
    }

    /// End of a run: an attempt cannot be waiting any more, so the view is
    /// re-derived with nothing waiting (a cooldown survives, a wait does not).
    pub(in crate::shell::app) fn clear_master_admission_wait(&mut self) {
        if let Some(view) = self.ac().admission_view.clone()
            && view.waiting > 0
        {
            let mut projected = elapsed_view(
                &view,
                self.ac().admission_observed_at,
                tokio::time::Instant::now(),
            );
            projected.waiting = 0;
            projected.longest_wait_seconds = None;
            self.apply_master_admission(&projected);
        }
    }

    /// Labels exist only for tracked children: a view for an unknown id is
    /// ignored (a peer cannot grow the map) and labels of children that left
    /// the roster are pruned on every update.
    pub(in crate::shell::app) fn apply_child_admission(&mut self, id: &str, view: &AdmissionView) {
        let state = self.ac_mut();
        let tracked = &state.roster.tracked;
        state
            .admission_children
            .retain(|child, _| tracked.contains_key(child));
        state
            .roster
            .admission_labels
            .retain(|child, _| tracked.contains_key(child));
        let newer = state
            .admission_children
            .get(id)
            .is_none_or(|(previous, _)| {
                view.revision > previous.revision
                    || (view.revision == previous.revision
                        && view.waiting > 0
                        && state.admission_unversioned_clears.contains(id))
            });
        if tracked
            .get(id)
            .is_some_and(|child| child.exited_at.is_none())
            && newer
        {
            debug_assert!(
                tracked.contains_key(id),
                "admission cannot create a roster entry"
            );
            // Keep quiet/granted snapshots too: their revision is the watermark
            // that prevents delayed forwarded waits from resurrecting a label.
            state
                .admission_children
                .insert(id.to_string(), (view.clone(), tokio::time::Instant::now()));
            state.admission_unversioned_clears.remove(id);
            match view.compact_label() {
                Some(label) => {
                    state.roster.admission_labels.insert(id.to_string(), label);
                }
                None => {
                    state.roster.admission_labels.remove(id);
                }
            }
        }
    }

    /// Move the authoritative view and its projection as one unit. The
    /// destination wins identity collisions, including quiet grant watermarks.
    pub(in crate::shell::app) fn rekey_child_admission(&mut self, from: &str, to: &str) {
        if let Some(snapshot) = self.ac_mut().admission_children.remove(from) {
            self.ac_mut()
                .admission_children
                .entry(to.to_string())
                .or_insert(snapshot);
        }
        self.ac_mut().roster.admission_labels.remove(from);
        let label = self
            .ac()
            .admission_children
            .get(to)
            .and_then(|(view, observed)| {
                elapsed_view(view, *observed, tokio::time::Instant::now()).compact_label()
            });
        match label {
            Some(label) => {
                self.ac_mut()
                    .roster
                    .admission_labels
                    .insert(to.to_string(), label);
            }
            None => {
                self.ac_mut().roster.admission_labels.remove(to);
            }
        }
    }

    /// Successful child snapshots use the same revision order as both event
    /// transports. Duplicate snapshots must not reset the local elapsed clock.
    pub(in crate::shell::app) fn apply_child_get_state_admission(
        &mut self,
        id: &str,
        view: Option<&AdmissionView>,
    ) {
        match view {
            Some(view) => self.apply_child_admission(id, view),
            None => {
                // Absence is authoritative, but preserve ordering knowledge.
                if let Some((view, _)) = self.ac_mut().admission_children.get_mut(id) {
                    let revision = view.revision;
                    *view = AdmissionView {
                        revision,
                        ..AdmissionView::default()
                    };
                }
                self.ac_mut().roster.admission_labels.remove(id);
            }
        }
    }

    pub(in crate::shell::app) fn clear_child_admission_wait(&mut self, id: &str) {
        if let Some((view, observed)) = self.ac().admission_children.get(id) {
            // Terminals are unversioned projections. Keep the authoritative view
            // intact so a same-revision state event can repair cross-feed order.
            let terminal_view =
                child_projection(view, *observed, tokio::time::Instant::now(), true);
            let label = terminal_view.compact_label();
            self.ac_mut()
                .admission_unversioned_clears
                .insert(id.to_string());
            match label {
                Some(label) => {
                    self.ac_mut()
                        .roster
                        .admission_labels
                        .insert(id.to_string(), label);
                }
                None => {
                    self.ac_mut().roster.admission_labels.remove(id);
                }
            }
        }
    }

    /// Keep ticking while a clock is active OR its final projection is pending.
    /// Select may be rebuilt after expiry before the armed timer is serviced.
    pub(in crate::shell::app) fn needs_admission_tick(&self) -> bool {
        let now = tokio::time::Instant::now();
        self.tabs.values().any(|state| {
            state.admission_view.as_ref().is_some_and(|view| {
                let projected = elapsed_view(view, state.admission_observed_at, now);
                projected.waiting > 0
                    || projected.has_active_dated_cooldown_after(0)
                    || state.master_session.footer.admission()
                        != projected.status_label().as_deref()
            }) || state
                .admission_children
                .iter()
                .any(|(id, (view, observed))| {
                    let projected = child_projection(
                        view,
                        *observed,
                        now,
                        state.admission_unversioned_clears.contains(id),
                    );
                    state.roster.tracked.contains_key(id)
                        && (projected.waiting > 0
                            || projected.has_active_dated_cooldown_after(0)
                            || state.roster.admission_labels.get(id)
                                != projected.compact_label().as_ref())
                })
        })
    }

    /// Transition events are snapshots, not clock ticks. Project elapsed local
    /// monotonic time without mutating the authoritative admission snapshot.
    pub(in crate::shell::app) fn tick_admission_labels(&mut self) -> bool {
        let now = tokio::time::Instant::now();
        let mut changed = false;
        for state in self.tabs.values_mut() {
            if let Some(view) = &state.admission_view {
                let view = elapsed_view(view, state.admission_observed_at, now);
                let label = view.status_label();
                if state.master_session.footer.admission() != label.as_deref() {
                    if view.waiting > 0 {
                        if let Some(label) = &label {
                            let message = format!("⏳ {} (Esc to interrupt)", capitalize(label));
                            if let Some(spinner) = &mut state.spinner
                                && state.admission_spinner_message.as_deref()
                                    == Some(spinner.message())
                            {
                                spinner.set_message(&message);
                            }
                            state.admission_spinner_message = Some(message);
                        }
                    }
                    state
                        .master_session
                        .footer
                        .set_admission(label, view.compact_label());
                    changed = true;
                }
            }
            state
                .admission_children
                .retain(|id, _| state.roster.tracked.contains_key(id));
            state
                .roster
                .admission_labels
                .retain(|id, _| state.roster.tracked.contains_key(id));
            for (id, (view, observed)) in &state.admission_children {
                let projected = child_projection(
                    view,
                    *observed,
                    now,
                    state.admission_unversioned_clears.contains(id),
                );
                let label = projected.compact_label();
                if state.roster.admission_labels.get(id) != label.as_ref() {
                    match label {
                        Some(label) => {
                            state.roster.admission_labels.insert(id.clone(), label);
                        }
                        None => {
                            state.roster.admission_labels.remove(id);
                        }
                    }
                    changed = true;
                }
            }
        }
        changed
    }
}

/// Terminal overlays clear only waiting, never the authoritative cooldown clock.
fn child_projection(
    view: &AdmissionView,
    observed: tokio::time::Instant,
    now: tokio::time::Instant,
    wait_cleared: bool,
) -> AdmissionView {
    let mut projected = elapsed_view(view, observed, now);
    if wait_cleared {
        projected.waiting = 0;
        projected.longest_wait_seconds = None;
        debug_assert_eq!(
            projected.revision, view.revision,
            "terminal overlay preserves revision"
        );
    }
    projected
}

fn elapsed_view(
    view: &AdmissionView,
    observed: tokio::time::Instant,
    now: tokio::time::Instant,
) -> AdmissionView {
    let mut projected = view.clone();
    let elapsed = now.saturating_duration_since(observed).as_secs();
    if view.waiting > 0 {
        projected.longest_wait_seconds = view
            .longest_wait_seconds
            .map(|seconds| seconds.saturating_add(elapsed));
    }
    for group in &mut projected.groups {
        if let Some(cooldown) = &mut group.cooldown
            && cooldown.state == "until"
        {
            cooldown.remaining_seconds = cooldown
                .remaining_seconds
                .map(|seconds| seconds.saturating_sub(elapsed));
        }
    }
    debug_assert_eq!(
        projected.revision, view.revision,
        "projection preserves revision"
    );
    projected
}

fn capitalize(label: &str) -> String {
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
#[path = "app_admission_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "app_admission_reconciliation_tests.rs"]
mod reconciliation_tests;
