//! Plain notice shown when the harness refuses a resume.
use crate::components::{
    ansi::sanitize_untrusted_label, select_overlay::build_select_overlay, theme,
};
use crate::protocol::resume_decision_payloads::{ResumeRefusal, ResumeRefusalCode};
use crate::shell::keys::Key;
#[path = "resume_decision_layout.rs"]
mod layout;
pub(super) use layout::bounded_ends;
use layout::{path_lines, wrap_bounded};

const LIMIT: usize = 512;

pub struct ResumeDecisionDialog {
    refusal: ResumeRefusal,
}

impl ResumeDecisionDialog {
    pub fn new(mut refusal: ResumeRefusal, _picked_title: Option<&str>) -> Self {
        refusal.kind = safe(&refusal.kind);
        refusal.execution_path = refusal.execution_path.as_deref().map(safe_path);
        refusal.detail = refusal.detail.as_deref().map(safe);
        refusal.command = refusal.command.as_deref().map(safe);
        Self { refusal }
    }

    /// This notice has no action: Enter, Escape, and Ctrl-C only dismiss it.
    pub fn handle_key(&mut self, key: &Key) -> bool {
        matches!(key, Key::Enter | Key::Escape | Key::Ctrl('c'))
    }

    pub fn render_overlay(&mut self, width: usize, height: usize) -> (Vec<String>, usize) {
        let refusal = &self.refusal;
        build_select_overlay(width, height, |content| {
            let mut lines = vec![theme::bold(title(refusal.code))];
            if let Some(path) = &refusal.execution_path {
                lines.extend(
                    path_lines(path, content, 2)
                        .into_iter()
                        .map(|v| theme::dim(&v)),
                );
            }
            if let Some(detail) = &refusal.detail {
                lines.extend(wrap_bounded(detail, content, 2));
            }
            if let Some(command) = &refusal.command {
                lines.extend(
                    wrap_bounded(command, content, 2)
                        .into_iter()
                        .map(|v| theme::dim(&v)),
                );
            }
            lines.push(theme::dim("Enter or Esc to dismiss"));
            lines.truncate(height.saturating_sub(6).max(1));
            lines
        })
    }
}

fn title(code: ResumeRefusalCode) -> &'static str {
    match code {
        ResumeRefusalCode::BelongsElsewhere => "This session is in another folder",
        ResumeRefusalCode::HomeMissing => "This session's folder is unavailable",
        ResumeRefusalCode::HomeChanged => "This session's folder has changed",
        ResumeRefusalCode::HomeUnknown => "This session's folder is unknown",
        ResumeRefusalCode::NoHomeRecorded => "No folder was recorded for this session",
    }
}

fn safe(value: &str) -> String {
    sanitize_untrusted_label(value, LIMIT)
}
fn safe_path(value: &str) -> String {
    bounded_ends(&sanitize_untrusted_label(value, usize::MAX), LIMIT)
}
