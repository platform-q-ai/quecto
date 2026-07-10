//! `/effort` runtime reasoning-effort control (#1067): provider-scoped
//! vocabulary, direct set, selector overlay, and response handling.
//!
//! Mirrors the `/model` + `set_model` pattern: bare `/effort` opens a
//! selector limited to the ACTIVE model's provider vocabulary; `/effort
//! <level>` validates locally (so an invalid level never leaves the TUI and
//! the previous setting stays) and sends `set_effort` over the socket. The
//! footer only updates on a successful response.

use super::*;

/// OpenAI's documented reasoning-effort scale (#1066), also what
/// OpenAI-compatible providers accept.
const OPENAI_EFFORT_LEVELS: &[&str] = &["none", "low", "medium", "high", "xhigh"];

/// Anthropic's documented effort scale (`max` is Opus 4.6 only).
const ANTHROPIC_EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "max"];

/// The effort vocabulary valid for the provider serving `model` (a
/// `provider/model-id` pair, or a bare model id). Mirrors the agent-side
/// rule so local validation and the agent always agree.
pub(super) fn effort_levels_for_model(model: Option<&str>) -> &'static [&'static str] {
    let Some(model) = model else {
        return OPENAI_EFFORT_LEVELS;
    };
    let (provider, id) = model.split_once('/').unwrap_or(("", model));
    if provider.contains("anthropic") || id.starts_with("claude") {
        ANTHROPIC_EFFORT_LEVELS
    } else {
        OPENAI_EFFORT_LEVELS
    }
}

impl App {
    /// Handle `/effort` (bare → selector) and `/effort <level>` (direct set).
    pub(super) fn handle_effort_command(&mut self, arg: &str) {
        if arg.is_empty() {
            self.open_effort_selector();
            return;
        }
        let levels = effort_levels_for_model(self.current_model.as_deref());
        if levels.contains(&arg) {
            self.send_set_effort(arg);
        } else {
            self.notify(
                &format!(
                    "Invalid effort level \"{arg}\" — valid levels: {}",
                    levels.join(", ")
                ),
                NotifyLevel::Error,
            );
        }
    }

    pub(super) fn open_effort_selector(&mut self) {
        let levels = effort_levels_for_model(self.current_model.as_deref());
        self.effort_selector = Some(EffortSelector::new(levels, self.current_effort.as_deref()));
    }

    pub(super) fn handle_effort_selector_key(&mut self, key: &Key) {
        if let Some(selector) = &mut self.effort_selector {
            selector.handle_input(key);
            match selector.take_result() {
                EffortSelectorResult::Selected(level) => {
                    self.effort_selector = None;
                    self.send_set_effort(&level);
                }
                EffortSelectorResult::Dismissed => {
                    self.effort_selector = None;
                }
                EffortSelectorResult::Pending => {}
            }
        }
    }

    /// Send `set_effort` over the socket. The footer is NOT updated here —
    /// only a successful response switches it, so a rejected or failed
    /// switch visibly keeps the previous level.
    pub(super) fn send_set_effort(&mut self, effort: &str) {
        self.send_command(Command::SetEffort {
            id: Some("se".into()),
            effort: effort.to_string(),
        });
    }

    /// Apply a successful `set_effort` response: the agent echoes the level
    /// it actually applied in `data.effort`.
    pub(super) fn handle_set_effort_success(&mut self, data: Option<serde_json::Value>) {
        let Some(level) = data
            .as_ref()
            .and_then(|d| d.get("effort"))
            .and_then(|v| v.as_str())
            .map(crate::interface::ansi::sanitize_control)
        else {
            return;
        };
        self.notify(&format!("Effort set to {level}"), NotifyLevel::Success);
        self.set_current_effort(Some(level));
    }

    /// Track the session's active effort and mirror it onto the footer.
    pub(super) fn set_current_effort(&mut self, effort: Option<String>) {
        self.master_session.footer.set_effort(effort.clone());
        self.current_effort = effort;
    }
}
