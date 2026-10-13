//! A swarm's run as the board keeps it once the swarm has ended (S4,
//! #2479): how its status, items, owners, evidence and spend map onto the
//! board's run of the plan it was briefed with.
//!
//! - **Outcome:** succeeded is succeeded; failed and budget-exhausted are
//!   failed; blocked and cancelled are stopped. Only those five statuses
//!   end a swarm, so only they make a [`SwarmEnd`], and a fold-back is only
//!   ever of an ended swarm.
//! - **Task items:** a swarm item is matched to the plan item whose brief
//!   title it carries ([`item_title`], `<slug>: <title>`), exactly, or,
//!   when none carries it, by its `<slug>: ` prefix (a slug holds no `:`,
//!   so a prefix names one plan item). Of several, the most progressed is
//!   taken (completed, submitted, claimed, ready, blocked), the newest of
//!   equals. Ready is todo, claimed doing, submitted review, completed
//!   done, blocked blocked. A plan item the swarm never seeded is todo
//!   (`unmatched`); a swarm item no plan item takes is counted (`extra`),
//!   never kept.
//! - **Secrets:** every `scheme://` address in an owner or evidence part,
//!   wherever it stands (bare, in a code span, in a link), keeps only its
//!   scheme, host, path and fragment: its userinfo is written `***` and
//!   its query (a signed URL's signature, a token) dropped. Then every
//!   secret-shaped span is redacted. The board is pushed, and read by every
//!   member.
//! - **Owner:** the member id, as one board line cut to
//!   [`MAX_OWNER_CHARS`]; none when the board cannot hold it.
//! - **Evidence:** one line per entry, `<artifact> at <revision>`, each
//!   word that is a whole `https://` address (its surrounding `<>`, `()`
//!   and trailing punctuation left outside) written as a `[link](…)`. A
//!   line the board cannot hold (its schema, its codec's text cell, a
//!   repeat, past [`MAX_PER_ITEM`] or past the run's [`MAX_RUN_TEXT_BYTES`],
//!   owners counted first) is dropped and counted, with no marker left:
//!   one bad line never sinks the fold-back.
//! - **Cost:** the swarm's observed tokens, within the board's bound.
use super::ports::BoardTextCells;
use super::swarm_brief::item_title;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::plan::PlanItem;
use crate::domain::project_board::entities::run::{MAX_RUN_TEXT_BYTES, RunItem};
use crate::domain::project_board::entities::task_parts::{
    MAX_BUDGET_TOKENS, MAX_LINE_CHARS, MAX_OWNER_CHARS, MAX_PER_ITEM,
};
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::schema_error::line;
use crate::domain::project_board::value_objects::telemetry::ItemCounts;
use crate::domain::project_board::value_objects::vocabulary::{ItemState, RunOutcome};
use crate::domain::redaction::redact_secrets;
use crate::domain::swarm::{RunState, TaskState};
use regex::{Captures, Regex};
use std::sync::LazyLock;

/// The scheme of the one address a board line links.
const HTTPS: &str = "https://";
/// The text of a link written for a bare address.
const LINK_TEXT: &str = "link";
/// What an address's userinfo is written as: dropped from a linked one.
const REDACTED_USER: &str = "***";
/// What may close a query and is given back to the text around it: a code
/// span's backtick, a link's or quote's close, punctuation.
const QUERY_CLOSERS: [char; 12] = ['`', ')', ']', '>', '"', '\'', '.', ',', ';', ':', '!', '?'];
/// A `scheme://` address anywhere in text (in a code span or a link
/// included): its scheme, then its authority, path, query and fragment,
/// each running to the next one's mark or a space.
static ADDRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?P<scheme>[A-Za-z][A-Za-z0-9+.\-]*://)(?P<authority>[^/?#\s]*)",
        r"(?P<path>[^?#\s]*)(?P<query>\?[^#\s]*)?(?P<fragment>#\S*)?",
    ))
    .expect("a valid pattern")
});
/// What may stand before a bare address and stay outside its link.
const OPENERS: [char; 2] = ['<', '('];
/// What may follow a bare address and stay outside its link.
const CLOSERS: [char; 8] = ['>', ')', '.', ',', ';', ':', '!', '?'];

/// Whether a fold-back keeps its evidence, or leaves it out (a run the
/// store judged too large with it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    Kept,
    Left,
}

/// How a swarm ended: one of the statuses that end it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwarmEnd {
    Succeeded,
    Failed,
    BudgetExhausted,
    Blocked,
    Cancelled,
}

impl SwarmEnd {
    pub const ALL: [Self; 5] = [
        Self::Succeeded,
        Self::Failed,
        Self::BudgetExhausted,
        Self::Blocked,
        Self::Cancelled,
    ];

    /// The end `status` is, when it ends the swarm (an allowlist: a
    /// swarm setting up, running or paused has not ended, nor has one
    /// whose status is any other text).
    pub fn of(status: &RunState) -> Option<Self> {
        match status.as_str() {
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "budget-exhausted" => Some(Self::BudgetExhausted),
            "blocked" => Some(Self::Blocked),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// The board's outcome of a run that ended so.
    pub fn outcome(self) -> RunOutcome {
        match self {
            Self::Succeeded => RunOutcome::Succeeded,
            Self::Failed | Self::BudgetExhausted => RunOutcome::Failed,
            Self::Blocked | Self::Cancelled => RunOutcome::Stopped,
        }
    }
}

/// Where a swarm item stood: one of the five statuses the swarm writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwarmItemState {
    Ready,
    Claimed,
    Submitted,
    Completed,
    Blocked,
}

impl SwarmItemState {
    /// The state `state` is, when the swarm writes it (an allowlist).
    pub fn of(state: &TaskState) -> Option<Self> {
        match state.as_str() {
            "ready" => Some(Self::Ready),
            "claimed" => Some(Self::Claimed),
            "submitted" => Some(Self::Submitted),
            "completed" => Some(Self::Completed),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }

    /// How far an item standing so got: completed, submitted, claimed,
    /// ready, then blocked (an item blocked and created again is taken by
    /// its new copy).
    pub fn progress(self) -> u8 {
        match self {
            Self::Blocked => 0,
            Self::Ready => 1,
            Self::Claimed => 2,
            Self::Submitted => 3,
            Self::Completed => 4,
        }
    }

    /// The board's state of a task item standing so.
    pub fn item_state(self) -> ItemState {
        match self {
            Self::Ready => ItemState::Todo,
            Self::Claimed => ItemState::Doing,
            Self::Submitted => ItemState::Review,
            Self::Completed => ItemState::Done,
            Self::Blocked => ItemState::Blocked,
        }
    }
}

/// One evidence entry a swarm item was submitted with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwarmEvidence {
    pub artifact: String,
    pub revision: String,
}

/// One item of an ended swarm's board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwarmItemView {
    pub title: String,
    pub state: SwarmItemState,
    /// The member holding it, by id.
    pub owner: Option<String>,
    pub evidence: Vec<SwarmEvidence>,
}

/// An ended swarm's board, as its live view (`get_swarm_board`) reads
/// it: how it ended, its items in the view's order, and the tokens it
/// was observed to spend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwarmRunView {
    pub end: SwarmEnd,
    pub items: Vec<SwarmItemView>,
    pub observed_tokens: u64,
}

/// What a fold-back keeps of a swarm's items: every plan item, in the
/// plan's order, and what was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldBackItems {
    pub items: Vec<RunItem>,
    /// Plan items no swarm item matched.
    pub unmatched: usize,
    /// Swarm items no plan item took.
    pub extra: usize,
    /// Evidence lines kept, and dropped.
    pub evidence: usize,
    pub dropped: usize,
}

impl FoldBackItems {
    /// The task items by the state each stood in.
    pub fn counts(&self) -> ItemCounts {
        let count = |state: ItemState| {
            let held = self.items.iter().filter(|item| item.state == state).count();
            u64::try_from(held).unwrap_or(u64::MAX)
        };
        ItemCounts {
            todo: count(ItemState::Todo),
            doing: count(ItemState::Doing),
            review: count(ItemState::Review),
            done: count(ItemState::Done),
            blocked: count(ItemState::Blocked),
        }
    }
}

/// Where each of `plan`'s task items stood when `view`'s swarm ended,
/// each line checked against `cells` as the board's codec holds it, and
/// its evidence kept or left out as `evidence` says.
pub fn run_items(
    plan: &Plan,
    view: &SwarmRunView,
    cells: &dyn BoardTextCells,
    evidence_kept: Evidence,
) -> FoldBackItems {
    let planned = &plan.fields().items;
    let found: Vec<Option<(usize, &SwarmItemView)>> =
        planned.iter().map(|item| matched(item, view)).collect();
    let at: Vec<usize> = found.iter().flatten().map(|(at, _)| *at).collect();
    debug_assert_eq!(
        at.iter().collect::<std::collections::BTreeSet<_>>().len(),
        at.len(),
        "a swarm item stands for one plan item at most (a slug holds no `:`)"
    );
    let matched: Vec<Option<&SwarmItemView>> = found
        .iter()
        .map(|found| found.map(|(_, swarm)| swarm))
        .collect();
    let taken = matched.iter().flatten().count();
    let owners: Vec<Option<String>> = matched
        .iter()
        .map(|swarm| swarm.and_then(|swarm| owner_of(swarm.owner.as_deref()?, cells)))
        .collect();
    // Owners first: what is left of the run's text is the evidence's.
    let mut room =
        MAX_RUN_TEXT_BYTES.saturating_sub(owners.iter().flatten().map(String::len).sum());
    let (mut evidence, mut dropped) = (0, 0);
    let items: Vec<RunItem> = planned
        .iter()
        .zip(matched.iter().zip(owners))
        .map(|(item, (swarm, owner))| {
            let lines = swarm.map_or(&[][..], |swarm| swarm.evidence.as_slice());
            let mut kept: Vec<String> = Vec::new();
            for entry in lines {
                let written = evidence_line(entry);
                let holds = evidence_kept == Evidence::Kept
                    && kept.len() < MAX_PER_ITEM
                    && written.len() <= room
                    && !kept.contains(&written)
                    && held_line(&written, MAX_LINE_CHARS, cells);
                match holds {
                    true => {
                        room -= written.len();
                        kept.push(written);
                    }
                    false => dropped += 1,
                }
            }
            evidence += kept.len();
            RunItem {
                id: item.id.clone(),
                state: swarm.map_or(ItemState::Todo, |swarm| swarm.state.item_state()),
                owner,
                evidence: kept,
            }
        })
        .collect();
    let folded = FoldBackItems {
        unmatched: planned.len() - taken,
        extra: view.items.len() - taken,
        items,
        evidence,
        dropped,
    };
    debug_assert_eq!(folded.items.len(), planned.len(), "every plan item, once");
    debug_assert_eq!(
        folded.evidence + folded.dropped,
        matched
            .iter()
            .flatten()
            .map(|swarm| swarm.evidence.len())
            .sum::<usize>(),
        "every evidence entry of a matched item is kept or dropped"
    );
    folded
}

/// The swarm item plan item `item` stands for, by its place in the view:
/// of those carrying its
/// title exactly, or else its `<slug>: ` prefix, the most progressed, the
/// newest of equals.
fn matched<'v>(item: &PlanItem, view: &'v SwarmRunView) -> Option<(usize, &'v SwarmItemView)> {
    let key = item_title(item);
    let prefix = format!("{}: ", item.id.as_str());
    debug_assert!(
        key.starts_with(&prefix),
        "a title key starts with its prefix"
    );
    let (key, prefix) = (key.as_str(), prefix.as_str());
    let under = |exact: bool| {
        view.items
            .iter()
            .enumerate()
            .filter(move |(_, swarm)| match exact {
                true => swarm.title == key,
                false => swarm.title.starts_with(prefix),
            })
    };
    let exact = under(true).next().is_some();
    under(exact).max_by_key(|(at, swarm)| (swarm.state.progress(), *at))
}

/// `entry` as one evidence line: `<artifact> at <revision>`, each part one
/// board line, its secrets redacted and its bare addresses linked. Empty
/// when either part is.
pub fn evidence_line(entry: &SwarmEvidence) -> String {
    let artifact = linked(&scrubbed(&entry.artifact));
    let revision = linked(&scrubbed(&entry.revision));
    match (artifact.is_empty(), revision.is_empty()) {
        (false, false) => format!("{artifact} at {revision}"),
        _ => String::new(),
    }
}

/// The tokens a run spent: what the swarm observed, at most what the board
/// spells.
pub fn cost_of(view: &SwarmRunView) -> u64 {
    view.observed_tokens.min(MAX_BUDGET_TOKENS)
}

/// `raw` from outside the board as one board line: each address in it
/// kept to what [`kept_address`] keeps, then every secret-shaped span
/// redacted.
fn scrubbed(raw: &str) -> String {
    let addressed = ADDRESS.replace_all(raw, kept_address);
    board_text::line(&redact_secrets(&addressed))
}

/// What an address in text keeps (an allowlist): its scheme, its host and
/// port (any userinfo before them written [`REDACTED_USER`]), its path and
/// its fragment; its query (where a signed URL's signature or a token
/// rides) is dropped, but for the [`QUERY_CLOSERS`] that end it, which
/// belong to the text around it (a code span's backtick, a link's close).
fn kept_address(found: &Captures<'_>) -> String {
    let part = |name: &str| found.name(name).map_or("", |part| part.as_str());
    let authority = part("authority");
    let host = match authority.rsplit_once('@') {
        Some((_, host)) => format!("{REDACTED_USER}@{host}"),
        None => authority.to_string(),
    };
    let query = part("query");
    let closing = &query[query.trim_end_matches(QUERY_CLOSERS).len()..];
    format!(
        "{}{host}{}{closing}{}",
        part("scheme"),
        part("path"),
        part("fragment")
    )
}

/// `text` with each word that is a whole bare address written as a link
/// to it ([`linked_word`]).
fn linked(text: &str) -> String {
    text.split(' ')
        .map(linked_word)
        .collect::<Vec<_>>()
        .join(" ")
}

/// `word` with the address it holds, once any of [`OPENERS`] before it
/// and [`CLOSERS`] after it are set apart, written as a link to that
/// address (angle brackets, an autolink's, dropped); `word` as it is when
/// what is left is no [`address`].
fn linked_word(word: &str) -> String {
    let opened = word.trim_start_matches(OPENERS);
    let core = opened.trim_end_matches(CLOSERS);
    let (lead, trail) = (&word[..word.len() - opened.len()], &opened[core.len()..]);
    match address(core) {
        Some(address) => format!(
            "{}[{LINK_TEXT}]({address}){}",
            lead.replace('<', ""),
            trail.replace('>', "")
        ),
        None => word.to_string(),
    }
}

/// `core` as the address a link holds: an `https://` address whose
/// authority (up to its first `/`, `?` or `#`) is a host, with at most the
/// [`REDACTED_USER`] userinfo [`kept_address`] writes (dropped here), its
/// query dropped, its parentheses percent-encoded, and every character one
/// the codec's address takes (an allowlist: its host, then its path and
/// fragment); none otherwise, so the word is left for the codec to judge.
fn address(core: &str) -> Option<String> {
    let rest = core.strip_prefix(HTTPS)?;
    let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    let host = match authority.split_once('@') {
        None => authority,
        Some((REDACTED_USER, host)) => host,
        Some(_) => return None,
    };
    let (before, fragment) = tail.split_once('#').unwrap_or((tail, ""));
    let path = before.split_once('?').map_or(before, |(path, _)| path);
    let kept = match fragment.is_empty() {
        true => path.to_string(),
        false => format!("{path}#{fragment}"),
    };
    let encoded = kept.replace('(', "%28").replace(')', "%29");
    let shaped = !host.is_empty() && host.chars().all(host_char) && encoded.chars().all(path_char);
    shaped.then(|| format!("{HTTPS}{host}{encoded}"))
}

/// A character the codec's address takes in its host part.
fn host_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-._~:!&'*+,;=%".contains(c)
}

/// A character the codec's address takes past its host.
fn path_char(c: char) -> bool {
    host_char(c) || "/?#@".contains(c)
}

/// Member id `raw` as an owner: one board line, its secrets redacted, cut
/// to [`MAX_OWNER_CHARS`] and trimmed again; none when the board cannot
/// hold what is left.
fn owner_of(raw: &str, cells: &dyn BoardTextCells) -> Option<String> {
    let cut: String = scrubbed(raw).chars().take(MAX_OWNER_CHARS).collect();
    let owner = board_text::line(&cut);
    held_line(&owner, MAX_OWNER_CHARS, cells).then_some(owner)
}

/// Whether the board holds `text` as one line of at most `max_chars`
/// characters: its schema's rule, and its codec's text cell.
fn held_line(text: &str, max_chars: usize, cells: &dyn BoardTextCells) -> bool {
    line("line", text, max_chars).is_ok() && cells.holds(text)
}

#[cfg(test)]
#[path = "fold_back_tests.rs"]
pub(super) mod tests;
