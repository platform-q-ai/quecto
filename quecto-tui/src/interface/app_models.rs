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
            Some(ModelEntry {
                id,
                provider,
                is_current: false,
            })
        })
        .collect()
}

impl App {
    pub(super) fn open_model_selector(&mut self) {
        if self.model_registry.0.is_empty() && !self.model_registry.1 {
            self.model_registry.1 = true;
            self.send_command(Command::ListModels {
                id: Some("model-selector".into()),
            });
        }
        self.open_model_selector_now();
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
        let Some(data) = data else { return };
        self.model_registry.0 = parse_model_entries(&data);
        if self.model_registry.1 {
            self.model_registry.1 = false;
            self.open_model_selector_now();
        }
    }
}
