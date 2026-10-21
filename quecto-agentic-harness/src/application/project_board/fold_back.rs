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
//! - **Secrets:** the board is public, so a line in doubt is dropped
//!   whole (and counted), never rewritten further; an owner in doubt is
//!   left out. A part is in doubt when it is past its board limit in
//!   characters (never cut: a cut may strip the `@` that marks a secret);
//!   when it holds an invisible character (a format character or another
//!   default ignorable one: a soft hyphen, a direction mark, a zero-width
//!   space, a variation selector, a tag); when, folded (NFKC, lower case),
//!   it names a credential word (`CREDENTIAL_WORDS`: `authorization`,
//!   `bearer`, `password`, `token`, `secret`, … as any part of a word, or
//!   `basic` before anything but a letter);
//!   when an address in it has a fragment that is no short anchor (up to
//!   64 of `[A-Za-z0-9_.-]`), a `%25` in its path, or is a bare host whose
//!   word holds a `;`; or when, once secrets are redacted, it holds an at
//!   sign (`@`, fullwidth, small, or `%40` encoded once or more) outside an
//!   address, but in a `worker@1` or `HEAD@{1}` word. Otherwise its spaces
//!   are made plain, and every address (`scheme://…`, `//host…` or a bare
//!   `host.tld` before a `/`, `?` or `#`, in a code span or a link too)
//!   that holds no at sign and has an unambiguous authority (an ASCII host,
//!   perhaps a port of digits) keeps that authority, its path up to its
//!   first `?`, `#`, `;` or `%3F`, and a short anchor straight after it;
//!   any other is written `scheme://***`. Then every secret-shaped span is
//!   redacted.
//! - **Accepted costs:** a line that merely mentions a credential word
//!   ("basic CI-check passed", "token budget") is dropped, so an item about
//!   auth, token or credential code may show little or no evidence (drops
//!   are counted, not shown); a line holding an invisible character is
//!   dropped; a line past its
//!   limit is dropped, not shortened; an address with an at sign anywhere
//!   (an npm `/@scope`, vite's `/@vite/client`, `?to=a@b.co`), an IPv6 or
//!   a non-ASCII host is written `scheme://***`; a line holding an email, a
//!   `pkg@1.2.3`, an `@types/node`, a `git@host:repo` or any other at sign
//!   outside an address is dropped; an owner holding one is left out.
//! - **Known limits** (best effort, as the shared redactor is): a secret
//!   with no at sign and no credential word (`-u admin:hunter2`), or in an
//!   address's path (a Slack or Discord webhook's token), is kept; so is a
//!   credential word spelt with a lookalike from another script (a
//!   Cyrillic `е` in `sеcret`), which no fold reads as Latin, and `basic`
//!   glued to letters (`basicAuth=…`); a
//!   bare `host.com;sig=…` with no `/`, a bare host with no dot
//!   (`localhost/x?sig=…`), or one glued to a CJK letter or a `_`, is not
//!   read as an address, so it keeps what follows. A line split by a line
//!   break is never one board line, so it is dropped whole.
//! - **Owner:** the member id, as one board line within
//!   [`MAX_OWNER_CHARS`], never cut; none when in doubt or when the board
//!   cannot hold it (a scrub that lengthens it past its limit included).
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
use crate::domain::project_board::value_objects::schema_error::line;
use crate::domain::project_board::value_objects::telemetry::ItemCounts;
use crate::domain::project_board::value_objects::vocabulary::ItemState;
#[cfg(test)]
use scrub::{MAX_FRAGMENT_CHARS, address, balanced, linked_word};
use scrub::{linked, scrubbed};

#[path = "fold_back_scrub.rs"]
mod scrub;

/// Whether a fold-back keeps its evidence, or leaves it out (a run the
/// store judged too large with it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    Kept,
    Left,
}

pub use super::dto::{SwarmEnd, SwarmEvidence, SwarmItemState, SwarmItemView, SwarmRunView};

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

pub use crate::domain::project_board::value_objects::commit_token::{
    MIN_COMMIT_PREFIX, MIN_COMMIT_TOKEN, checks_commit, commit_token,
};

/// What the evidence of a run's recorded task items names (#2666): the
/// commit token of each entry (by [`commit_token`]: `None` for one naming
/// none), in the plan's order; and each token once, case-insensitively.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvidenceRevisions {
    pub ids: Vec<String>,
    pub entries: Vec<Option<String>>,
}

impl EvidenceRevisions {
    /// How many entries name no commit that a repository whose full ids
    /// are `id_len` hex digits checks ([`checks_commit`]).
    pub fn unchecked(&self, id_len: usize) -> usize {
        let checked = self
            .entries
            .iter()
            .filter(|token| {
                token
                    .as_deref()
                    .is_some_and(|token| checks_commit(token, id_len))
            })
            .count();
        self.entries.len() - checked
    }
}

/// What the evidence of the swarm items `plan`'s task items stand for (as
/// [`run_items`] matches them) names: what a run recorded from `view`
/// claims as its work, which the push rule checks. An extra item's
/// evidence is never recorded, so it is not among it.
pub fn evidence_revisions(plan: &Plan, view: &SwarmRunView) -> EvidenceRevisions {
    let mut named = EvidenceRevisions::default();
    let entries = plan
        .fields()
        .items
        .iter()
        .filter_map(|item| matched(item, view))
        .flat_map(|(_, swarm)| swarm.evidence.iter());
    for entry in entries {
        let token = commit_token(&entry.revision);
        if let Some(id) = &token {
            if named.ids.iter().all(|named| named != id) {
                named.ids.push(id.clone());
            }
        }
        named.entries.push(token);
    }
    named
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
/// board line, scrubbed ([`scrubbed`]) and its bare addresses linked.
/// Empty, so the line is dropped and counted, when either part is empty or
/// in doubt.
pub fn evidence_line(entry: &SwarmEvidence) -> String {
    let part = |raw: &str| scrubbed(raw, MAX_LINE_CHARS).map(|kept| linked(&kept));
    match (part(&entry.artifact), part(&entry.revision)) {
        (Some(artifact), Some(revision)) if !artifact.is_empty() && !revision.is_empty() => {
            format!("{artifact} at {revision}")
        }
        _ => String::new(),
    }
}

/// The tokens a run spent: what the swarm observed, at most what the board
/// spells.
pub fn cost_of(view: &SwarmRunView) -> u64 {
    view.observed_tokens.min(MAX_BUDGET_TOKENS)
}

/// Member id `raw` as an owner: scrubbed as an evidence part is
/// ([`scrubbed`], within [`MAX_OWNER_CHARS`]); none when in doubt (past
/// its limit, an at sign but `worker@1`'s, a credential word) or when the
/// board cannot hold what is left.
fn owner_of(raw: &str, cells: &dyn BoardTextCells) -> Option<String> {
    let owner = scrubbed(raw, MAX_OWNER_CHARS)?;
    held_line(&owner, MAX_OWNER_CHARS, cells).then_some(owner)
}

/// Whether the board holds `text` as one line of at most `max_chars`
/// characters: its schema's rule, and its codec's text cell.
fn held_line(text: &str, max_chars: usize, cells: &dyn BoardTextCells) -> bool {
    line("line", text, max_chars).is_ok() && cells.holds(text)
}

#[cfg(test)]
#[path = "fold_back_scrub_tests.rs"]
mod scrub_tests;
#[cfg(test)]
#[path = "fold_back_tests.rs"]
pub(super) mod tests;
