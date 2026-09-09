//! Inference-admission observability in the TUI (#1679 P4): the master's
//! waiting/cooldown label rides on the footer and the working spinner; a
//! forwarded child view paints a compact label on that child's panel row.
//! Admission never changes a lifecycle state: no session is marked idle or
//! stalled by it.

use super::super::*;
use crate::protocol::admission_payloads::{AdmissionView, parse_admission};

pub(in crate::shell::app) const WORKING_MESSAGE: &str = "Working... (Esc to interrupt)";

impl App {
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
        self.ac_mut().admission_view = Some(view.clone());
        self.ac_mut()
            .master_session
            .footer
            .set_admission(label.clone(), view.compact_label());
        match label.filter(|_| waiting) {
            Some(label) => {
                let message = format!("{} (Esc to interrupt)", capitalize(&label));
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
        if let Some(mut view) = self.ac().admission_view.clone()
            && view.waiting > 0
        {
            view.waiting = 0;
            view.longest_wait_seconds = None;
            self.apply_master_admission(&view);
        }
    }

    /// Labels exist only for tracked children: a view for an unknown id is
    /// ignored (a peer cannot grow the map) and labels of children that left
    /// the roster are pruned on every update.
    fn apply_child_admission(&mut self, id: &str, view: &AdmissionView) {
        let roster = &mut self.ac_mut().roster;
        let tracked = &roster.tracked;
        roster
            .admission_labels
            .retain(|child, _| tracked.contains_key(child));
        if !tracked.contains_key(id) {
            return;
        }
        match view.compact_label() {
            Some(label) => {
                roster.admission_labels.insert(id.to_string(), label);
            }
            None => {
                roster.admission_labels.remove(id);
            }
        }
    }
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
