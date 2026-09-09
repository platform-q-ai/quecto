//! Inference-admission observability in the TUI (#1679 P4): the master's
//! waiting/cooldown label rides on the footer and the working spinner; a
//! forwarded child view paints a label on that child's panel row. Admission
//! never changes a lifecycle state: no session is marked idle or stalled by it.

use super::*;
use crate::protocol::admission_payloads::{AdmissionView, parse_admission};

pub(super) const WORKING_MESSAGE: &str = "Working... (Esc to interrupt)";

impl App {
    /// `admission_state_changed`: `None`/own id → master; a child id → roster.
    pub(super) fn handle_admission_state(
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

    pub(super) fn apply_master_admission(&mut self, view: &AdmissionView) {
        let label = view.status_label();
        self.ac_mut()
            .master_session
            .footer
            .set_admission(label.clone());
        if let Some(spinner) = &mut self.ac_mut().spinner {
            match (&label, view.waiting > 0) {
                (Some(label), true) => {
                    spinner.set_message(&format!("{} (Esc to interrupt)", capitalize(label)))
                }
                _ => spinner.set_message(WORKING_MESSAGE),
            }
        }
    }

    /// End of a run: an attempt cannot be waiting any more, so only a
    /// cooldown-only label survives.
    pub(super) fn clear_master_admission_wait(&mut self) {
        let footer = &mut self.ac_mut().master_session.footer;
        if footer
            .admission()
            .is_some_and(|label| label.starts_with("waiting"))
        {
            footer.set_admission(None);
        }
    }

    fn apply_child_admission(&mut self, id: &str, view: &AdmissionView) {
        let labels = &mut self.ac_mut().roster.admission_labels;
        match view.compact_label() {
            Some(label) => {
                labels.insert(id.to_string(), label);
            }
            None => {
                labels.remove(id);
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
