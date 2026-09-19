//! The resume decision dialog (#2011): the explicit choices the harness
//! offers for a session that cannot simply be restored here. Presentation
//! only — which actions exist and which are available is the harness's word;
//! an unavailable action is shown, explained and never sent.
use crate::components::{
    ansi::sanitize_control,
    component::Component,
    select_list::{SelectItem, SelectList, SelectResult},
    select_overlay::build_select_overlay,
    theme,
    utils::wrap_text,
};
use crate::protocol::resume_decision_payloads::{
    ResumeAction, ResumeActionOffer, ResumeDecision, ResumeDecisionKind, ResumeSelection,
};
use crate::shell::keys::Key;

#[derive(Debug, PartialEq, Eq)]
pub enum ResumeDecisionEvent {
    /// An available action: the stable identity, the action and the version
    /// the decision was made on.
    Chosen(ResumeSelection),
    /// An unavailable action: the reason to show; the dialog stays open.
    Unavailable(String),
    /// Cancel or Escape: close, send nothing.
    Cancelled,
    Pending,
}

pub struct ResumeDecisionDialog {
    decision: ResumeDecision,
    list: SelectList,
}

const FOOTER: &str = "Enter choose · Esc cancel — nothing is restored or linked";

/// No terminal controls, bounded length.
fn safe_text(value: &str) -> String {
    sanitize_control(value).chars().take(512).collect()
}

fn title(kind: ResumeDecisionKind) -> &'static str {
    match kind {
        ResumeDecisionKind::CrossFolder => "This session belongs to another folder",
        ResumeDecisionKind::HomeMissing => "This session's folder is missing or moved",
        ResumeDecisionKind::HomeChanged => "This session's folder changed its workspace",
        ResumeDecisionKind::HomeUnknown => "This session's folder record is unreadable",
        ResumeDecisionKind::LegacyUnscoped => "This session was never linked to a folder",
    }
}

fn label(action: ResumeAction) -> &'static str {
    match action {
        ResumeAction::OpenOriginal => "Open original folder",
        ResumeAction::ForkCurrent => "Fork into current folder",
        ResumeAction::Locate => "Locate folder",
        ResumeAction::Associate => "Associate with a folder",
        ResumeAction::Cancel => "Cancel",
    }
}

fn item(index: usize, offer: &ResumeActionOffer) -> SelectItem {
    let description = (!offer.available).then(|| "unavailable".to_string());
    SelectItem {
        value: index.to_string(),
        label: label(offer.action).to_string(),
        description,
    }
}

impl ResumeDecisionDialog {
    /// The decision's texts are untrusted metadata: made safe and bounded
    /// here, once, before anything is rendered.
    pub fn new(mut decision: ResumeDecision) -> Self {
        decision.session = safe_text(&decision.session);
        decision.execution_path = decision.execution_path.as_deref().map(safe_text);
        decision.detail = decision.detail.as_deref().map(safe_text);
        for offer in &mut decision.actions {
            offer.reason = offer.reason.as_deref().map(safe_text);
        }
        let items = decision
            .actions
            .iter()
            .enumerate()
            .map(|(index, offer)| item(index, offer))
            .collect();
        let rows = decision.actions.len().max(1);
        Self {
            decision,
            list: SelectList::new(items, rows),
        }
    }

    pub fn decision(&self) -> &ResumeDecision {
        &self.decision
    }

    pub fn handle_key(&mut self, key: &Key) -> ResumeDecisionEvent {
        self.list.handle_input(key);
        let chosen = match self.list.take_result() {
            SelectResult::Pending => return ResumeDecisionEvent::Pending,
            SelectResult::Dismissed => return ResumeDecisionEvent::Cancelled,
            SelectResult::Selected(value) => value,
        };
        let offer = chosen
            .parse::<usize>()
            .ok()
            .and_then(|index| self.decision.actions.get(index));
        match offer {
            None => ResumeDecisionEvent::Cancelled,
            Some(offer) if offer.action == ResumeAction::Cancel => ResumeDecisionEvent::Cancelled,
            Some(offer) if offer.available => ResumeDecisionEvent::Chosen(ResumeSelection {
                session: self.decision.session_key.clone(),
                action: Some(offer.action),
                expected_home_version: Some(self.decision.home_version.clone()),
            }),
            Some(offer) => ResumeDecisionEvent::Unavailable(format!(
                "{} is unavailable{}",
                label(offer.action),
                offer
                    .reason
                    .as_deref()
                    .map(|reason| format!(": {reason}"))
                    .unwrap_or_default()
            )),
        }
    }

    /// Title, the session and its recorded folder, the offers, and — for an
    /// unavailable offer under the cursor — the harness's reason, wrapped.
    pub fn render_overlay(&mut self, width: usize, height: usize) -> (Vec<String>, usize) {
        let place = self
            .decision
            .execution_path
            .as_deref()
            .or(self.decision.detail.as_deref())
            .unwrap_or("no folder recorded");
        let subject = format!("{} — {place}", self.decision.session);
        let reason = self
            .list
            .selected_item()
            .and_then(|item| item.value.parse::<usize>().ok())
            .and_then(|index| self.decision.actions.get(index))
            .filter(|offer| !offer.available)
            .map(|offer| offer.reason.clone().unwrap_or_default());
        let kind = self.decision.kind;
        let list = &mut self.list;
        build_select_overlay(width, height, |content_width| {
            let mut lines = vec![
                theme::bold(title(kind)),
                theme::dim(&subject),
                String::new(),
            ];
            lines.extend(list.render(content_width));
            lines.push(String::new());
            if let Some(reason) = reason {
                let text = format!("Unavailable: {reason}");
                lines.extend(
                    wrap_text(&text, content_width)
                        .iter()
                        .map(|l| theme::dim(l)),
                );
            }
            lines.push(theme::dim(FOOTER));
            lines
        })
    }
}

#[cfg(test)]
#[path = "resume_decision_tests.rs"]
mod tests;
