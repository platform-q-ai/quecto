//! The words of the resume decision dialog (#2011): titles, action labels,
//! the footer and the mark of an unavailable row — each in a long form and in
//! a short form for a narrow terminal, chosen so that nothing is clipped
//! silently. For people who have never heard of a "workspace" or a "record".
use crate::components::{
    select_list::SelectItem,
    utils::{truncate_to_width, visible_width},
};
use crate::protocol::resume_decision_payloads::{
    ResumeAction, ResumeActionOffer, ResumeDecisionKind,
};

pub(super) const FOOTER: &str = "Enter choose · Esc cancel — your current session is untouched";
/// What a single footer line says when the whole footer does not fit it.
pub(super) const FOOTERS: [&str; 3] = [
    "Enter choose · Esc cancel",
    "Enter · Esc cancel",
    "Esc cancel",
];
/// Under the cursor of an unavailable offer the harness gave no reason for.
pub(super) const NO_REASON: &str = "Not available in this version";
/// No folder and no detail came with the decision.
pub(super) const NO_FOLDER: &str = "No folder on record";
/// Enter on an unavailable offer: the reason is already under the cursor, so
/// the one-line toast only points at it.
pub const UNAVAILABLE_POINTER: &str = "Not available yet — see the reason below";

/// The long title, and the short one for a width the long one does not fit.
pub(super) fn titles(kind: ResumeDecisionKind) -> [&'static str; 2] {
    match kind {
        ResumeDecisionKind::CrossFolder => {
            ["This session belongs to another folder", "Other folder"]
        }
        ResumeDecisionKind::HomeMissing => [
            "This session's folder can't be opened (missing, moved or no permission)",
            "Folder missing",
        ],
        ResumeDecisionKind::HomeChanged => [
            "This folder is no longer the same project as when the session was saved",
            "Folder changed",
        ],
        ResumeDecisionKind::HomeUnknown => [
            "quecto can't read where this session was saved",
            "Folder unreadable",
        ],
        ResumeDecisionKind::LegacyUnscoped => [
            "This session was saved before quecto tracked folders",
            "No folder",
        ],
    }
}

fn labels(action: ResumeAction) -> [&'static str; 2] {
    match action {
        ResumeAction::OpenOriginal => ["Open original folder", "Open original"],
        ResumeAction::ForkCurrent => ["Copy into this folder as a new session", "Copy here"],
        ResumeAction::Locate => ["Locate folder", "Locate"],
        ResumeAction::Associate => ["Attach to a folder", "Attach"],
        ResumeAction::Cancel => ["Cancel", "Cancel"],
    }
}

/// How an unavailable row says so: spelled out, abbreviated, or as a leading
/// cross when the labels themselves barely fit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mark {
    Spelled,
    Short,
    Cross,
}

impl Mark {
    fn suffix(self) -> &'static str {
        match self {
            Self::Spelled => " — unavailable",
            Self::Short => " (n/a)",
            Self::Cross => "",
        }
    }
}

/// The widest wording every row has room for, so that EVERY unavailable row
/// carries a mark at every width: long labels before short ones, a spelled
/// mark before an abbreviated one, the cross last.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct RowWording {
    short: bool,
    mark: Mark,
}

impl RowWording {
    pub(super) const WIDEST: Self = Self {
        short: false,
        mark: Mark::Spelled,
    };

    pub(super) fn fitting(offers: &[ResumeActionOffer], width: usize) -> Self {
        let candidates = [
            Self::WIDEST,
            Self {
                short: false,
                mark: Mark::Short,
            },
            Self {
                short: true,
                mark: Mark::Spelled,
            },
            Self {
                short: true,
                mark: Mark::Short,
            },
        ];
        let fits = |wording: &Self| {
            offers
                .iter()
                .all(|offer| 2 + visible_width(&wording.text(offer)) <= width)
        };
        candidates.into_iter().find(fits).unwrap_or(Self {
            short: true,
            mark: Mark::Cross,
        })
    }

    fn text(self, offer: &ResumeActionOffer) -> String {
        let name = labels(offer.action)[usize::from(self.short)];
        match (offer.available, self.mark) {
            (true, _) => name.to_string(),
            (false, Mark::Cross) => format!("✗ {name}"),
            (false, mark) => format!("{name}{}", mark.suffix()),
        }
    }

    /// The rows of the list; a label the width still clips ends in "…". Two
    /// columns of every row are the list's selection marker.
    pub(super) fn items(self, offers: &[ResumeActionOffer], width: usize) -> Vec<SelectItem> {
        let room = width.saturating_sub(2).max(1);
        let rows = offers.iter().enumerate();
        rows.map(|(index, offer)| SelectItem {
            value: index.to_string(),
            label: truncate_to_width(&self.text(offer), room, Some("…")),
            description: None,
        })
        .collect()
    }
}
