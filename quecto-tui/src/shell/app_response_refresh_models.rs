//! The `refresh_models` success handler.
//!
//! Kept beside `app_response.rs` rather than inside it so that file stays within
//! the module size gate.

use crate::components::notification::NotifyLevel;

use super::App;

impl App {
    /// Report what a catalogue refresh actually did: which sources refreshed,
    /// which failed, and whether anything was queried at all.
    pub(in crate::shell::app) fn handle_refresh_models_success(
        &mut self,
        data: Option<serde_json::Value>,
    ) {
        // A refresh can succeed for some sources and fail for others;
        // reporting only the success would hide the part that did not.
        let failures = data
            .as_ref()
            .map(|d| {
                crate::protocol::model_payloads::parse_refresh_failures(
                    d,
                    &crate::components::ansi::sanitize_control,
                )
            })
            .unwrap_or_default();
        let (skipped, refreshed_any) = data
            .as_ref()
            .map(|d| {
                (
                    crate::protocol::model_payloads::parse_refresh_skipped(
                        d,
                        &crate::components::ansi::sanitize_control,
                    ),
                    crate::protocol::model_payloads::parse_refresh_refreshed_any(d),
                )
            })
            .unwrap_or_default();
        if failures.is_empty() && !refreshed_any && !skipped.is_empty() {
            // Nothing was queried: saying "complete" would claim work
            // that did not happen.
            self.notify(
                &format!("Nothing to refresh: {}", skipped.join("; ")),
                NotifyLevel::Info,
            );
        } else if failures.is_empty() {
            self.notify("Model catalogue refresh complete", NotifyLevel::Info);
        } else {
            self.notify(
                &format!("Model catalogue refreshed, except {}", failures.join("; ")),
                NotifyLevel::Warning,
            );
        }
        self.open_model_selector();
    }
}
