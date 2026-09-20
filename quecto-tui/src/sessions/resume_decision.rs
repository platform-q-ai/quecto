//! Plain notice shown when the harness refuses a resume because the session
//! belongs to another folder (#2045). It offers nothing: it says where the
//! session lives and how to open it there, and any key dismisses it.
use crate::components::{
    ansi::sanitize_untrusted_label, select_overlay::build_select_overlay, theme,
};
use crate::protocol::resume_decision_payloads::{ResumeRefusal, ResumeRefusalCode};
use crate::shell::keys::Key;
#[path = "resume_decision_layout.rs"]
mod layout;
pub(super) use layout::bounded_ends;
use layout::{path_lines, wrap_bounded, wrap_exact};

const LIMIT: usize = 512;
/// A command is shown whole or not at all: never cut to a label's length.
const COMMAND_LIMIT: usize = 16 * 1024;
const FOOTER: &str = "Enter or Esc to close";

pub struct ResumeDecisionDialog {
    refusal: ResumeRefusal,
    picked_title: Option<String>,
}

impl ResumeDecisionDialog {
    pub fn new(mut refusal: ResumeRefusal, picked_title: Option<&str>) -> Self {
        refusal.kind = safe(&refusal.kind);
        refusal.execution_path = refusal.execution_path.as_deref().map(safe_path);
        refusal.detail = refusal.detail.as_deref().map(safe);
        refusal.command = refusal.command.as_deref().and_then(whole_or_nothing);
        refusal.resume = refusal.resume.as_deref().and_then(whole_or_nothing);
        let picked_title = picked_title.map(safe).filter(|title| !title.is_empty());
        Self {
            refusal,
            picked_title,
        }
    }

    /// This notice has no action: Enter, Escape, and Ctrl-C only dismiss it.
    pub fn handle_key(&mut self, key: &Key) -> bool {
        matches!(key, Key::Enter | Key::Escape | Key::Ctrl('c'))
    }

    pub fn render_overlay(&mut self, width: usize, height: usize) -> (Vec<String>, usize) {
        let budget = height.saturating_sub(6).max(1);
        build_select_overlay(width, height, |content| self.lines(content, budget))
    }

    /// Title, what was picked, the folder, the harness's detail, how to open
    /// it, the footer. When the panel is too short the explanatory lines go
    /// first — the way to open the session and the footer are kept whole.
    fn lines(&self, content: usize, budget: usize) -> Vec<String> {
        let refusal = &self.refusal;
        let mut context: Vec<String> = Vec::new();
        if let Some(title) = &self.picked_title {
            context.extend(wrap_bounded(title, content, 1));
        }
        if let Some(path) = &refusal.execution_path {
            context.extend(path_lines(path, content, 2).iter().map(|v| theme::dim(v)));
        }
        if let Some(detail) = &refusal.detail {
            context.extend(wrap_bounded(detail, content, 2));
        }
        let mut how: Vec<String> = Vec::new();
        match (&refusal.command, &refusal.resume) {
            (Some(command), resume) => {
                how.push("Open quecto there:".to_string());
                how.extend(wrap_exact(command, content));
                if let Some(resume) = resume {
                    how.push("then type:".to_string());
                    how.extend(wrap_exact(resume, content));
                }
            }
            (None, Some(resume)) => {
                how.push("Open quecto in that folder, then type:".to_string());
                how.extend(wrap_exact(resume, content));
            }
            // An older harness names no command and no step: still say what to do.
            (None, None) if refusal.execution_path.is_some() => {
                how.push("Open quecto in that folder and resume it there.".to_string());
            }
            (None, None) => {}
        }
        let fixed = 1 + how.len() + 1;
        context.truncate(budget.saturating_sub(fixed));
        let mut lines = vec![theme::bold(title(refusal.code))];
        lines.extend(context);
        lines.extend(how);
        lines.truncate(budget.saturating_sub(1).max(1));
        lines.push(theme::dim(FOOTER));
        lines
    }
}

fn title(code: ResumeRefusalCode) -> &'static str {
    match code {
        ResumeRefusalCode::BelongsElsewhere => "This session belongs to another folder",
        ResumeRefusalCode::HomeMissing => "This session's folder is missing or unreadable",
        ResumeRefusalCode::HomeChanged => "This session's folder has changed",
        ResumeRefusalCode::HomeUnknown => "This session's folder can't be read",
        ResumeRefusalCode::NoHomeRecorded => "No folder is recorded for this session",
    }
}

fn safe(value: &str) -> String {
    sanitize_untrusted_label(value, LIMIT)
}
fn safe_path(value: &str) -> String {
    bounded_ends(&sanitize_untrusted_label(value, usize::MAX), LIMIT)
}
/// A command the sanitiser would have to change is not the command the harness
/// meant: show none rather than one that runs differently from how it reads.
fn whole_or_nothing(value: &str) -> Option<String> {
    let shown = sanitize_untrusted_label(value, COMMAND_LIMIT);
    (shown == value && !shown.is_empty()).then_some(shown)
}

#[cfg(test)]
#[path = "resume_decision_tests.rs"]
mod tests;
