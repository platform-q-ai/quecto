//! The resume decision dialog (#2011): the explicit choices the harness
//! offers for a session that cannot simply be restored here. Presentation
//! only — which actions exist and which are available is the harness's word;
//! an unavailable action is shown, explained and never sent.
use crate::components::{
    ansi::sanitize_untrusted_label,
    component::Component,
    select_list::{SelectList, SelectResult},
    select_overlay::build_select_overlay,
    theme,
    utils::visible_width,
};
#[path = "resume_decision_layout.rs"]
mod layout;
#[path = "resume_decision_wording.rs"]
mod wording;
use crate::protocol::resume_decision_payloads::{ResumeAction, ResumeDecision, ResumeSelection};
use crate::shell::keys::Key;
use layout::{bounded_ends, path_lines, wrap_bounded, wrap_words};
pub use wording::UNAVAILABLE_POINTER;
use wording::{FOOTER, FOOTERS, NO_FOLDER, NO_REASON, RowWording, titles};

#[derive(Debug, PartialEq, Eq)]
pub enum ResumeDecisionEvent {
    /// An available action: the stable identity, the action and the version
    /// the decision was made on.
    Chosen(ResumeSelection),
    /// An unavailable action: nothing is sent and the dialog stays open — its
    /// reason is already shown under the cursor.
    Unavailable,
    /// Cancel or Escape: close, send nothing.
    Cancelled,
    Pending,
}

pub struct ResumeDecisionDialog {
    decision: ResumeDecision,
    /// The title of the row the user picked, when the picker listed one.
    picked_title: Option<String>,
    list: SelectList,
}

/// The recorded folder never takes more lines than this.
const PATH_LINES: usize = 2;
/// Every text is bounded to this many characters before it is laid out.
const TEXT_CHARS: usize = 512;

/// No terminal controls, no invisible characters, bounded length.
fn safe_text(value: &str) -> String {
    sanitize_untrusted_label(value, TEXT_CHARS)
}

/// A folder is bounded from the WHOLE safe path, keeping both its ends.
fn safe_path(value: &str) -> String {
    bounded_ends(&sanitize_untrusted_label(value, usize::MAX), TEXT_CHARS)
}

impl ResumeDecisionDialog {
    /// The decision's texts are untrusted metadata: made safe and bounded
    /// here, once, before anything is rendered. `picked_title` is the title
    /// the picker showed for this session, if it listed it.
    pub fn new(mut decision: ResumeDecision, picked_title: Option<&str>) -> Self {
        decision.session = safe_text(&decision.session);
        decision.session_key = safe_text(&decision.session_key);
        decision.execution_path = decision.execution_path.as_deref().map(safe_path);
        decision.detail = decision.detail.as_deref().map(safe_text);
        for offer in &mut decision.actions {
            offer.reason = offer.reason.as_deref().map(safe_text);
        }
        let picked_title = picked_title
            .map(safe_text)
            .filter(|title| !title.is_empty() && *title != decision.session_key);
        let rows = decision.actions.len().max(1);
        let items = RowWording::WIDEST.items(&decision.actions, usize::MAX);
        let list = SelectList::new(items, rows);
        Self {
            decision,
            picked_title,
            list,
        }
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
            Some(_) => ResumeDecisionEvent::Unavailable,
        }
    }

    /// Title, the session (the title the user picked, its key beneath), its
    /// recorded folder on its own line(s) and the harness's detail, the offers
    /// — each unavailable one marked — the reason of the unavailable offer
    /// under the cursor, and the footer. Words wrap whole; the folder keeps
    /// both its ends; the reason takes the rows that are free; the least
    /// important lines are shed first, so the border survives a small terminal.
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
        let Self {
            decision,
            picked_title,
            list,
        } = self;
        build_select_overlay(width, height, |content| {
            let wording = RowWording::fitting(&decision.actions, content);
            list.sync_items(wording.items(&decision.actions, content));
            list.set_max_visible(decision.actions.len().max(1));
            let [long, short] = titles(decision.kind);
            let title = if wrap_words(long, content).len() <= 2 {
                long
            } else {
                short
            };
            let session = match picked_title {
                Some(title) => vec![title.clone(), decision.session_key.clone()],
                None => vec![decision.session.clone()],
            };
            let (place, detail) = match (&decision.execution_path, &decision.detail) {
                (Some(path), detail) => (Place::Path(path), detail.as_deref()),
                (None, Some(detail)) => (Place::Text(detail), None),
                (None, None) => (Place::Text(NO_FOLDER), None),
            };
            let reason = reason.map(|reason| reason.unwrap_or_else(|| NO_REASON.to_string()));
            let mut sections = Sections {
                width: content,
                title,
                title_lines: wrap_bounded(title, content, 2).len(),
                picked: picked_title.is_some(),
                session: (session.iter())
                    .flat_map(|line| wrap_bounded(line, content, 1))
                    .collect(),
                place_lines: place.lines(content, PATH_LINES).len(),
                place,
                detail: detail.map_or_else(Vec::new, |detail| wrap_bounded(detail, content, 1)),
                offers: list.render(content),
                reason: reason.map_or_else(Vec::new, |reason| wrap_words(&reason, content)),
                // Two lines only when they hold the whole footer.
                footer_lines: match wrap_words(FOOTER, content).len() {
                    lines @ 1..=2 => lines,
                    _ => 1,
                },
            };
            let spacing = sections.fit(rows);
            // Still too tall (many offers, a very low terminal): the list
            // scrolls in fewer rows, and last of all the footer goes — the
            // box keeps its border.
            let mut visible = decision.actions.len();
            while sections.height(spacing) > rows && visible > 1 {
                visible -= 1;
                list.set_max_visible(visible);
                sections.offers = list.render(content);
            }
            if sections.height(spacing) > rows {
                sections.footer_lines = 0;
                // One row: the cursor's, without the list's position line.
                sections.offers.truncate(rows);
            }
            sections.lines(spacing)
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

/// The dialog's lines by section. The reason takes the rows that are free —
/// never more than the rows left beside one line of title, of folder and of
/// footer and the offers. When the terminal is too small the least important
/// line goes first: the footer's second line, the spacing, the session's key
/// and title, the detail, the folder's and the title's second lines, the
/// reason beyond two lines, the folder, the rest of the reason, the title.
/// Whatever is cut is re-laid-out so the cut is marked.
struct Sections<'a> {
    width: usize,
    title: &'static str,
    title_lines: usize,
    /// Whether the first session line is the title the user picked.
    picked: bool,
    session: Vec<String>,
    place: Place<'a>,
    place_lines: usize,
    detail: Vec<String>,
    offers: Vec<String>,
    reason: Vec<String>,
    footer_lines: usize,
}

impl Sections<'_> {
    fn height(&self, spacing: usize) -> usize {
        let reason_gap = usize::from(!self.reason.is_empty() && spacing > 0);
        let fixed = self.title_lines + self.session.len() + self.place_lines + self.detail.len();
        fixed + self.offers.len() + self.reason.len() + self.footer_lines + spacing + reason_gap
    }

    /// Cut the reason to `lines`, marking the cut on its last line.
    fn cut_reason(&mut self, lines: usize) {
        if self.reason.len() <= lines {
            return;
        }
        self.reason.truncate(lines);
        if let Some(last) = self.reason.last_mut() {
            *last = wrap_bounded(&format!("{last} …"), self.width, 1).concat();
            if !last.ends_with('…') {
                last.push('…');
            }
        }
    }

    /// Shed one line; `false` when nothing sheddable is left.
    fn shed(&mut self, spacing: &mut usize) -> bool {
        if self.footer_lines > 1 {
            self.footer_lines -= 1;
        } else if *spacing > 0 {
            *spacing -= 1;
        } else if self.session.pop().is_some() || self.detail.pop().is_some() {
        } else if self.place_lines > 1 {
            self.place_lines -= 1;
        } else if self.title_lines > 1 {
            self.title_lines -= 1;
        } else if self.reason.len() > 2 {
            self.cut_reason(self.reason.len() - 1);
        } else if self.place_lines > 0 {
            self.place_lines -= 1;
        } else if !self.reason.is_empty() {
            self.cut_reason(self.reason.len() - 1);
        } else if self.title_lines > 0 {
            self.title_lines -= 1;
        } else {
            return false;
        }
        true
    }

    /// Fit `rows`; the spacing that is left.
    fn fit(&mut self, rows: usize) -> usize {
        let structure = 1 + self.place_lines.min(1) + self.offers.len() + 1;
        self.cut_reason(rows.saturating_sub(structure).max(1));
        let mut spacing = 2;
        while self.height(spacing) > rows && self.shed(&mut spacing) {}
        spacing
    }

    fn lines(self, spacing: usize) -> Vec<String> {
        let fits = |text: &&str| visible_width(text) <= self.width;
        let footer = match self.footer_lines {
            0 => Vec::new(),
            1 => {
                let line = [FOOTER].iter().chain(&FOOTERS).find(|text| fits(text));
                wrap_bounded(line.unwrap_or(&FOOTERS[2]), self.width, 1)
            }
            _ => wrap_bounded(FOOTER, self.width, 2),
        };
        let title = wrap_bounded(self.title, self.width, self.title_lines);
        let place = self.place.lines(self.width, self.place_lines);
        let blank = |wanted: bool| wanted.then(String::new);
        let mut lines: Vec<String> = title.iter().map(|l| theme::bold(l)).collect();
        // The title the user picked reads as text; its key, as metadata.
        let plain = usize::from(self.picked);
        lines.extend(self.session.iter().take(plain).cloned());
        let dimmed = self.session.iter().skip(plain);
        lines.extend(
            dimmed
                .chain(&place)
                .chain(&self.detail)
                .map(|l| theme::dim(l)),
        );
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
