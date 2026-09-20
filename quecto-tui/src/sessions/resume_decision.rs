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
use layout::{path_lines, wrap_bounded, wrap_exact, wrap_words};

const LIMIT: usize = 512;
/// A command is shown whole or not at all: never cut to a label's length.
const COMMAND_LIMIT: usize = 16 * 1024;
const FOOTER: &str = "Enter or Esc to close";
/// The footer on a panel too narrow for it: never a clipped sentence.
const FOOTER_NARROW: &str = "Esc closes";

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

    /// Title, what was picked, the folder, the harness's detail, what to do,
    /// the footer. When the panel is too short the explanatory lines go first;
    /// what to do is then shown in the fullest form that fits WHOLE — a command
    /// is never shown in part (#2056 review): a cut command is not the command.
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
        // Nothing here is wider than the panel: the overlay would clip it with
        // an ellipsis, and a clipped label reads as a broken one.
        let heading = wrap_bounded(title(refusal.code), content, 3);
        let footer = [FOOTER, FOOTER_NARROW, "Esc"]
            .into_iter()
            .find(|text| text.chars().count() <= content)
            .unwrap_or("Esc");
        // Title and footer are always shown; `room` is what is left between them.
        let room = budget.saturating_sub(heading.len() + 1);
        let how = self
            .what_to_do(content)
            .into_iter()
            .find(|form| form.len() <= room)
            .unwrap_or_default();
        context.truncate(room - how.len());
        let mut lines: Vec<String> = heading.iter().map(|line| theme::bold(line)).collect();
        lines.extend(context);
        lines.extend(how);
        lines.push(theme::dim(footer));
        lines
    }

    /// What to do, fullest form first. Going to the recorded folder resumes
    /// ONE kind — a session that lives in another folder — so only that kind
    /// is ever shown a command or a resume step, whatever the harness sent.
    fn what_to_do(&self, content: usize) -> Vec<Vec<String>> {
        let refusal = &self.refusal;
        if refusal.code != ResumeRefusalCode::BelongsElsewhere {
            let why = match refusal.code {
                ResumeRefusalCode::HomeMissing => {
                    "A session resumes only in the folder it was saved in. Bring that folder back to resume it."
                }
                ResumeRefusalCode::HomeChanged => {
                    "That folder is a different project now than when this was saved, so it can't be resumed."
                }
                ResumeRefusalCode::HomeUnknown => {
                    "Its folder record can't be read, so it can't be resumed."
                }
                _ => "It was saved before quecto tracked folders, so it can't be resumed.",
            };
            return vec![wrap_bounded(why, content, 3)];
        }
        // A label wraps on words; what is to be typed is cut at the column only.
        let step = |label: &str, text: &str| {
            let mut lines = wrap_words(label, content);
            lines.extend(wrap_exact(text, content));
            lines
        };
        let mut forms = Vec::new();
        match (&refusal.command, &refusal.resume) {
            (Some(command), Some(resume)) => {
                let mut whole = step("Open quecto there:", command);
                whole.extend(step("then type:", resume));
                forms.push(whole);
                forms.push(step("Open quecto in that folder, then type:", resume));
            }
            (Some(command), None) => forms.push(step("Open quecto there:", command)),
            (None, Some(resume)) => {
                forms.push(step("Open quecto in that folder, then type:", resume));
            }
            (None, None) => {}
        }
        // An older harness names neither; and the last form that always fits.
        forms.push(wrap_words(
            "Open quecto in that folder and resume it there.",
            content,
        ));
        forms
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
