use super::*;
use crate::interface::components::model_selector::ModelEntry;

pub(super) fn parse_model_entries(data: &serde_json::Value) -> Vec<ModelEntry> {
    // Protocol boundary (#1220): raw payload interpretation lives in the
    // application-layer mapper; this seam only adapts the typed DTO into the
    // interface's own view model.
    crate::application::model_payloads::parse_model_list(
        data,
        &crate::interface::ansi::sanitize_control,
    )
    .into_iter()
    .map(|entry| ModelEntry {
        id: entry.id,
        provider: entry.provider,
        auth: entry.auth,
        is_current: false,
    })
    .collect()
}

impl App {
    /// Send `set_model` over the socket. When a sub-agent is focused, route
    /// via its own UDS connection (mirrors `send_set_effort`, #1085). Child
    /// display state remains authoritative: it updates only from the
    /// post-success `get_state` resync, so a rejected switch keeps the
    /// previously active model visible.
    pub(super) fn send_set_model(&mut self, model: &str) {
        let cmd = Command::SetModel {
            id: Some("sm".into()),
            model: Some(model.to_string()),
            provider: None,
            model_id: None,
        };
        if self.subagents.active_agent_id.is_some() {
            if !self.send_to_active_subagent(cmd) {
                self.notify(
                    "Selected sub-agent is not ready for model changes yet",
                    NotifyLevel::Error,
                );
                return;
            }
            // Unlike the master path below, do not update the focused child's
            // footer or selector marker optimistically. The child's set_model
            // acknowledgement has no model payload, so its follow-up get_state
            // is the authoritative point at which both values change.
            return;
        }
        self.send_command(cmd);
        self.master_session.footer.set_model(model);
        self.current_model = Some(model.to_string());
        self.context_stats_requested = false;
    }

    pub(super) fn open_model_selector(&mut self) {
        // On-consume reload (ADR-0002): always re-request the model list when the
        // selector is opened so edits to `models.json` are reflected, not just on
        // the first open. The kernel gates the underlying file read by mtime/hash,
        // so this is cheap when nothing changed. We defer opening the selector
        // until the fresh list arrives (handled in `handle_list_models`) so the
        // list is always correct rather than a stale cached snapshot.
        if !self.model_registry.open_pending {
            self.model_registry.open_pending = true;
            self.send_command(Command::ListModels {
                id: Some("model-selector".into()),
            });
        }
    }

    pub(super) fn open_model_selector_now(&mut self) {
        let selector = if self.model_registry.entries.is_empty() {
            ModelSelector::new(self.current_model.as_deref())
        } else {
            let entries = self.model_registry.entries.clone();
            ModelSelector::with_models(entries, self.current_model.as_deref())
        };
        self.model_selector = Some(selector);
    }

    pub(super) fn handle_model_selector_key(&mut self, key: &Key) {
        if let Some(selector) = &mut self.model_selector {
            selector.handle_input(key);

            match selector.take_result() {
                ModelSelectorResult::Selected(model) => {
                    self.model_selector = None;
                    self.send_set_model(&model);
                }
                ModelSelectorResult::Dismissed => {
                    self.model_selector = None;
                }
                ModelSelectorResult::Pending => {}
            }
        }
    }

    pub(super) fn handle_list_models(&mut self, data: Option<serde_json::Value>) {
        let Some(data) = data else {
            // No data on the response: clear the pending flag so a later open can
            // re-request, and fall back to opening with whatever we have cached.
            if self.model_registry.open_pending {
                self.model_registry.open_pending = false;
                self.open_model_selector_now();
            }
            return;
        };
        self.model_registry.entries = parse_model_entries(&data);
        if self.model_registry.open_pending {
            self.model_registry.open_pending = false;
            self.open_model_selector_now();
        }
    }
}

#[cfg(test)]
#[path = "app_model_focus_1085_tests.rs"]
mod app_model_focus_1085_tests;
#[cfg(test)]
#[path = "app_models_protocol_characterization_tests.rs"]
mod app_models_protocol_characterization_tests;
#[cfg(test)]
#[path = "app_models_tests.rs"]
mod tests;
