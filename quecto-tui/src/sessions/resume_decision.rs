//! The resume decision dialog (#2011): the explicit choices the harness
//! offers for a session that cannot simply be restored here. Presentation
//! only — which actions exist and which are available is the harness's word;
//! an unavailable action is shown, explained and never sent.
use crate::components::{
    ansi::sanitize_untrusted_label,
    component::Component,
    select_list::{SelectItem, SelectList, SelectResult},
    select_overlay::build_select_overlay,
    theme,
    utils::visible_width,
};
#[path = "resume_decision_layout.rs"]
mod layout;
use crate::protocol::resume_decision_payloads::{
    ResumeAction, ResumeActionOffer, ResumeDecision, ResumeDecisionKind, ResumeSelection,
};
use crate::shell::keys::Key;
use layout::{path_lines, wrap_bounded};

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
/// What a single footer line says when the whole footer does not fit it.
const FOOTERS: [&str; 3] = [
    "Enter choose · Esc cancel",
    "Enter · Esc cancel",
    "Esc cancel",
];
/// The recorded folder and the reason never take more lines than these.
const PATH_LINES: usize = 2;
const REASON_LINES: usize = 4;
/// Narrower than this, the dialog says less rather than breaking its words.
const NARROW: usize = 20;

/// No terminal controls, no bidi or zero-width characters, bounded length.
fn safe_text(value: &str) -> String {
    sanitize_untrusted_label(value, 512)
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

/// How an unavailable row says so: the widest mark every row has room for,
/// so that EVERY unavailable row carries one at every width — spelled out,
/// abbreviated, or as a leading cross when the labels themselves barely fit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Spelled,
    Short,
    Cross,
}

impl Mark {
    /// Two columns of every row are the list's selection marker.
    fn fitting(offers: &[ResumeActionOffer], width: usize) -> Self {
        let widest = offers
            .iter()
            .filter(|offer| !offer.available)
            .map(|offer| visible_width(label(offer.action)))
            .max()
            .unwrap_or(0);
        [Self::Spelled, Self::Short]
            .into_iter()
            .find(|mark| 2 + widest + visible_width(mark.suffix()) <= width)
            .unwrap_or(Self::Cross)
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Spelled => " — unavailable",
            Self::Short => " (n/a)",
            Self::Cross => "",
        }
    }
}

fn item(index: usize, offer: &ResumeActionOffer, mark: Mark) -> SelectItem {
    let name = label(offer.action);
    let label = match (offer.available, mark) {
        (true, _) => name.to_string(),
        (false, Mark::Cross) => format!("✗ {name}"),
        (false, mark) => format!("{name}{}", mark.suffix()),
    };
    SelectItem {
        value: index.to_string(),
        label,
        description: None,
    }
}

fn items(offers: &[ResumeActionOffer], mark: Mark) -> Vec<SelectItem> {
    let rows = offers.iter().enumerate();
    rows.map(|(index, offer)| item(index, offer, mark))
        .collect()
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
        let rows = decision.actions.len().max(1);
        let list = SelectList::new(items(&decision.actions, Mark::Spelled), rows);
        Self { decision, list }
    }

    #[cfg(test)]
    pub(crate) fn decision(&self) -> &ResumeDecision {
        &self.decision
    }

    /// Ctrl-C leaves the dialog exactly as Escape does: nothing is sent. (The
    /// dialog owns the keyboard, so the key cannot mean "interrupt" here.)
    pub fn handle_key(&mut self, key: &Key) -> ResumeDecisionEvent {
        if matches!(key, Key::Ctrl('c')) {
            return ResumeDecisionEvent::Cancelled;
        }
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

    /// Title, the session, its recorded folder on its own line(s), the offers
    /// — each unavailable one marked — the reason of the unavailable offer
    /// under the cursor, and the footer. Words wrap whole; the folder keeps
    /// both its ends; the sections are bounded and the least important lines
    /// are shed first, so the footer and the border survive a small terminal.
    pub fn render_overlay(&mut self, width: usize, height: usize) -> (Vec<String>, usize) {
        let reason = self
            .list
            .selected_item()
            .and_then(|item| item.value.parse::<usize>().ok())
            .and_then(|index| self.decision.actions.get(index))
            .filter(|offer| !offer.available)
            .map(|offer| offer.reason.clone().filter(|reason| !reason.is_empty()));
        // Two border rows; the overlay builder keeps `height - 4` rows in all.
        let rows = height.saturating_sub(6).max(1);
        let Self { decision, list } = self;
        build_select_overlay(width, height, |content| {
            list.sync_items(items(
                &decision.actions,
                Mark::fitting(&decision.actions, content),
            ));
            let place = match (&decision.execution_path, &decision.detail) {
                (Some(path), _) => Place::Path(path),
                (None, Some(detail)) => Place::Text(detail),
                (None, None) => Place::Text("no folder recorded"),
            };
            let sections = Sections {
                width: content,
                title: title(decision.kind),
                title_lines: wrap_bounded(title(decision.kind), content, 2).len(),
                session: wrap_bounded(&decision.session, content, 1),
                place_lines: place.lines(content, PATH_LINES).len(),
                place,
                offers: list.render(content),
                reason: reason.map_or_else(Vec::new, |reason| {
                    // The heading is dropped where it would not fit a line whole.
                    let text = match reason {
                        Some(reason) if content >= NARROW => format!("Unavailable: {reason}"),
                        Some(reason) => reason,
                        None => "Unavailable in this version".to_string(),
                    };
                    wrap_bounded(&text, content, REASON_LINES)
                }),
                // Two lines only when they hold the whole footer.
                footer_lines: match layout::wrap_words(FOOTER, content).len() {
                    lines @ 1..=2 => lines,
                    _ => 1,
                },
            };
            sections.fitted(rows)
        })
    }
}

/// Where the session was saved: a path keeps both its ends when it is cut
/// (re-elided for the lines it is given); any other text wraps by word.
enum Place<'a> {
    Path(&'a str),
    Text(&'a str),
}

impl Place<'_> {
    fn lines(&self, width: usize, max_lines: usize) -> Vec<String> {
        match self {
            _ if max_lines == 0 => Vec::new(),
            Self::Path(path) => path_lines(path, width, max_lines),
            Self::Text(text) => wrap_bounded(text, width, max_lines),
        }
    }
}

/// The dialog's lines by section. When the terminal is too small the least
/// important line goes first: the reason beyond two lines, the spacing, the
/// rest of the reason, the session, the folder's and the footer's and the
/// title's second lines, the folder, the title. The offers and one footer
/// line always stay; whatever is cut is re-laid-out so the cut is marked.
struct Sections<'a> {
    width: usize,
    title: &'static str,
    title_lines: usize,
    session: Vec<String>,
    place: Place<'a>,
    place_lines: usize,
    offers: Vec<String>,
    reason: Vec<String>,
    footer_lines: usize,
}

impl Sections<'_> {
    fn height(&self, spacing: usize) -> usize {
        let reason_gap = usize::from(!self.reason.is_empty() && spacing > 0);
        let fixed = self.title_lines + self.session.len() + self.place_lines;
        fixed + self.offers.len() + self.reason.len() + self.footer_lines + spacing + reason_gap
    }

    /// Shed one line; `false` when nothing sheddable is left.
    fn shed(&mut self, spacing: &mut usize) -> bool {
        if self.reason.len() > 2 || (*spacing == 0 && self.reason.len() > 1) {
            self.reason.pop();
            let last = self.reason.last_mut().expect("one line is left");
            if !last.ends_with('…') {
                *last = wrap_bounded(&format!("{last} …"), self.width, 1).concat();
            }
        } else if *spacing > 0 {
            *spacing -= 1;
        } else if self.reason.pop().is_some() || self.session.pop().is_some() {
        } else if self.place_lines > 1 {
            self.place_lines -= 1;
        } else if self.footer_lines > 1 {
            self.footer_lines -= 1;
        } else if self.title_lines > 1 {
            self.title_lines -= 1;
        } else if self.place_lines > 0 {
            self.place_lines -= 1;
        } else if self.title_lines > 0 {
            self.title_lines -= 1;
        } else {
            return false;
        }
        true
    }

    fn fitted(mut self, rows: usize) -> Vec<String> {
        let mut spacing = 2;
        while self.height(spacing) > rows && self.shed(&mut spacing) {}
        let fits = |text: &&str| visible_width(text) <= self.width;
        let footer = match self.footer_lines {
            2.. => wrap_bounded(FOOTER, self.width, 2),
            _ => {
                let line = [FOOTER].iter().chain(&FOOTERS).find(|text| fits(text));
                wrap_bounded(line.unwrap_or(&FOOTERS[2]), self.width, 1)
            }
        };
        let title = wrap_bounded(self.title, self.width, self.title_lines);
        let place = self.place.lines(self.width, self.place_lines);
        let blank = |wanted: bool| wanted.then(String::new);
        let mut lines: Vec<String> = title.iter().map(|l| theme::bold(l)).collect();
        lines.extend(self.session.iter().chain(&place).map(|l| theme::dim(l)));
        lines.extend(blank(spacing > 0));
        lines.extend(self.offers);
        lines.extend(blank(spacing > 1));
        lines.extend(self.reason.iter().map(|l| theme::dim(l)));
        lines.extend(blank(!self.reason.is_empty() && spacing > 0));
        lines.extend(footer.iter().map(|l| theme::dim(l)));
        lines
    }
}

#[cfg(test)]
#[path = "resume_decision_tests.rs"]
mod tests;
