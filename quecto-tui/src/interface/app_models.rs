use super::*;
use crate::interface::components::model_selector::ModelEntry;

pub(super) fn parse_model_entries(data: &serde_json::Value) -> Vec<ModelEntry> {
    let Some(models) = data.get("models").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    models
        .iter()
        .filter_map(|m| {
            let raw_model = m
                .get("model")
                .or_else(|| m.get("id"))
                .and_then(|v| v.as_str())?;
            let id: String = raw_model.chars().filter(|c| !c.is_control()).collect();
            if id.is_empty() {
                return None;
            }
            let provider: String = m
                .get("provider")
                .and_then(|v| v.as_str())
                .or_else(|| id.split_once('/').map(|(provider, _)| provider))
                .unwrap_or("Model")
                .chars()
                .filter(|c| !c.is_control())
                .collect();
            let auth = m
                .get("auth")
                .and_then(|v| v.as_str())
                .map(|s| s.chars().filter(|c| !c.is_control()).collect::<String>())
                .filter(|s| !s.is_empty());
            Some(ModelEntry {
                id,
                provider,
                auth,
                is_current: false,
            })
        })
        .collect()
}

impl App {
    pub(super) fn open_model_selector(&mut self) {
        // On-consume reload (ADR-0002): always re-request the model list when the
        // selector is opened so edits to `models.json` are reflected, not just on
        // the first open. The kernel gates the underlying file read by mtime/hash,
        // so this is cheap when nothing changed. We defer opening the selector
        // until the fresh list arrives (handled in `handle_list_models`) so the
        // list is always correct rather than a stale cached snapshot.
        if !self.model_registry.1 {
            self.model_registry.1 = true;
            self.send_command(Command::ListModels {
                id: Some("model-selector".into()),
            });
        }
    }

    pub(super) fn open_model_selector_now(&mut self) {
        let selector = if self.model_registry.0.is_empty() {
            ModelSelector::new(self.current_model.as_deref())
        } else {
            ModelSelector::with_models(self.model_registry.0.clone(), self.current_model.as_deref())
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
                ModelSelectorResult::Cancelled => {
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
            if self.model_registry.1 {
                self.model_registry.1 = false;
                self.open_model_selector_now();
            }
            return;
        };
        self.model_registry.0 = parse_model_entries(&data);
        if self.model_registry.1 {
            self.model_registry.1 = false;
            self.open_model_selector_now();
        }
    }
}
