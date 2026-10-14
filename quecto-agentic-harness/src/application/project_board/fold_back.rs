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
//!   whole (and counted), never rewritten further. Each owner and evidence
//!   part is cut to its board limit first, and its spaces made plain, so an
//!   address ends where a reader sees it end. A part is in doubt when it
//!   holds an `Authorization:` header or a credential after `basic` or
//!   `bearer`; an address whose fragment is no short anchor (up to 64 of
//!   `[A-Za-z0-9_.-]`), whose path holds `%25`, or a bare host whose word
//!   holds a `;`; or an at sign (`@`, fullwidth or small) outside an
//!   address, but in a `worker@1` or `HEAD@{1}` word. Otherwise every
//!   address (`scheme://…`, `//host…` or a bare `host.tld` before a `/`, `?`
//!   or `#`, in a code span or a link too) that holds no at sign and has an
//!   unambiguous authority (an ASCII host, perhaps a port of digits) keeps
//!   that authority, its path up to its first `?`, `#`, `;` or `%3F`, and a
//!   short anchor straight after it; any other is written `scheme://***`.
//!   Then every secret-shaped span is redacted.
//! - **Accepted costs:** an address with an at sign anywhere (an npm
//!   `/@scope`, vite's `/@vite/client`, `?to=a@b.co`), an IPv6 or a
//!   non-ASCII host is written `scheme://***`; a line holding an email, a
//!   `pkg@1.2.3`, an `@types/node`, a `git@host:repo` or any other at sign
//!   outside an address is dropped, as is one holding `basic` or `bearer`
//!   before a credential-like word; an owner holding one is left out.
//! - **Known limits** (best effort, as the shared redactor is): a secret in
//!   an address's path (a Slack or Discord webhook's token) is kept, since
//!   no pattern of the redactor names one; a bare host with no dot
//!   (`localhost/x?sig=…`), or one glued to a CJK letter or a `_`, is not
//!   read as an address, so it keeps its query; and a secret that is no
//!   address, holds no at sign and matches no redactor pattern (a bare
//!   `sig=…` pair) is kept. A line split by a line break is never one board
//!   line, so it is dropped whole.
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
use std::cell::Cell;
use std::sync::LazyLock;

/// The scheme of the one address a board line links.
const HTTPS: &str = "https://";
/// The text of a link written for a bare address.
const LINK_TEXT: &str = "link";
/// What may close a query and is given back to the text around it: a code
/// span's backtick, a link's or quote's close, punctuation.
const QUERY_CLOSERS: [char; 12] = ['`', ')', ']', '>', '"', '\'', '.', ',', ';', ':', '!', '?'];
/// An address anywhere in text (in a code span or a link included), to
/// the next space: a `scheme://` one (`scheme`, then `rest`); a
/// scheme-relative `//host…` one after the start, a space or an opening
/// mark (`lead`, then `relative`); or a bare `host.tld[:port]` one
/// followed by a `/`, `?` or `#` (`bare`).
static ADDRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?P<scheme>[A-Za-z][A-Za-z0-9+.\-]*://)(?P<rest>\S*)",
        r#"|(?P<lead>^|[\s`(\[<"'])(?P<relative>//[^\s/]\S*)"#,
        r"|\b(?P<bare>[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)+(?::[0-9]+)?[/?#]\S*)",
    ))
    .expect("a valid pattern")
});
/// What an address holding an `@`, or whose authority is ambiguous, is
/// written as past its scheme.
const SCRUBBED: &str = "***";
/// The at signs: ASCII, fullwidth and small.
const AT_SIGNS: [char; 3] = ['@', '\u{ff20}', '\u{fe6b}'];
/// The only words outside an address that may hold an at sign: a name and
/// a number (`worker@1`), or a reflog reference (`HEAD@{1}`).
static AT_EXEMPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[A-Za-z0-9._\-]+@[0-9]+|[A-Za-z0-9._/\-]*@\{[0-9]+\})$")
        .expect("a valid pattern")
});
/// A credential's scheme before a credential-like word (`Basic dXNl…`,
/// `bearer eyJ…`), in any case.
static CREDENTIAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:basic|bearer)\s+(?P<word>\S{8,})").expect("a valid pattern")
});
/// The longest fragment an address keeps: a short anchor.
const MAX_FRAGMENT_CHARS: usize = 64;
/// What may open a word, set apart before the word is judged.
const WORD_OPENERS: [char; 6] = ['`', '(', '[', '<', '"', '\''];
/// The line breaks [`board_text::line`] reads as one: kept by [`spaced`],
/// so a line they break is still refused as two.
const LINE_BREAKS: [char; 6] = ['\n', '\r', '\u{2028}', '\u{2029}', '\u{85}', '\u{b}'];
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

/// `raw` from outside the board as one board line, or none when in doubt
/// (the board is public: a doubtful line is dropped, never rewritten
/// further). `raw` is cut to `cap` characters first (its board limit, so
/// no work grows past it) and its spaces made plain ([`spaced`]). In doubt:
/// an `Authorization:` header or a credential after `basic` or `bearer`
/// ([`credentialed`]); an address [`kept`] doubts; or an at sign left
/// outside an address that is no [`AT_EXEMPT`] word. Otherwise each
/// address is kept to what [`kept`] keeps, and every secret-shaped span
/// redacted.
fn scrubbed(raw: &str, cap: usize) -> Option<String> {
    let capped: String = raw.chars().take(cap).collect();
    let spaced = spaced(&capped);
    match credentialed(&spaced) {
        false => {}
        true => return None,
    }
    let doubt = Cell::new(false);
    let addressed = ADDRESS.replace_all(&spaced, |found: &Captures<'_>| {
        kept_address(found).unwrap_or_else(|| {
            doubt.set(true);
            found[0].to_string()
        })
    });
    match (doubt.get(), at_signs_exempt(&addressed)) {
        (false, true) => {}
        _ => return None,
    }
    let redacted = redact_secrets(&addressed);
    debug_assert!(
        at_signs_exempt(&redacted)
            && redacted
                .split(' ')
                .filter(|word| word.contains("://"))
                .all(|word| !word.contains(AT_SIGNS)),
        "no userinfo and no at sign but an exempt word survive: {redacted:?}"
    );
    Some(board_text::line(&redacted))
}

/// Whether `text` holds an `Authorization:` header, or `basic` or
/// `bearer` before a credential-like word (one that is no prose: not
/// letters alone, lower case past the first), in any case.
fn credentialed(text: &str) -> bool {
    let prose = |word: &str| {
        word.chars().all(|c| c.is_ascii_alphabetic())
            && word.chars().skip(1).all(|c| c.is_ascii_lowercase())
    };
    text.to_lowercase().contains("authorization:")
        || CREDENTIAL
            .captures_iter(text)
            .any(|found| !prose(&found["word"]))
}

/// Whether every word of `text` that holds an at sign, its opening and
/// closing marks set apart, is an [`AT_EXEMPT`] one (an allowlist).
fn at_signs_exempt(text: &str) -> bool {
    text.split(' ')
        .map(|word| {
            word.trim_start_matches(WORD_OPENERS)
                .trim_end_matches(QUERY_CLOSERS)
        })
        .all(|word| !word.contains(AT_SIGNS) || AT_EXEMPT.is_match(word))
}

/// `raw` with every space but a line break a plain one, so an address ends
/// where a reader sees it end (a no-break space cannot hide the rest of a
/// password past it).
fn spaced(raw: &str) -> String {
    raw.chars()
        .map(|c| match c.is_whitespace() && !LINE_BREAKS.contains(&c) {
            true => ' ',
            false => c,
        })
        .collect()
}

/// What an address in text keeps of itself ([`kept`]), its scheme, its
/// `//` or the start of its bare host as they were, and the text before
/// it (`lead`) left alone; none when [`kept`] doubts it.
fn kept_address(found: &Captures<'_>) -> Option<String> {
    let part = |name: &str| found.name(name).map(|part| part.as_str());
    match (part("scheme"), part("relative"), part("bare")) {
        (Some(scheme), _, _) => kept(scheme, part("rest").unwrap_or(""), Form::Named),
        (None, Some(relative), _) => {
            let rest = relative.strip_prefix("//").unwrap_or(relative);
            let kept = kept("//", rest, Form::Named)?;
            Some(format!("{}{kept}", part("lead").unwrap_or("")))
        }
        (None, None, Some(bare)) => kept("", bare, Form::Bare),
        (None, None, None) => unreachable!("ADDRESS matches one of its forms"),
    }
}

/// How an address was written: with a scheme or `//`, or as a bare host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Named,
    Bare,
}

/// What came of judging an address.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Judged {
    /// Kept: its unambiguous authority, path and anchor.
    Kept(String),
    /// Written [`SCRUBBED`]: it holds an at sign, or an ambiguous
    /// authority.
    Scrubbed,
    /// In doubt: its line is dropped.
    Doubt,
}

/// What an address keeps of `span` (all of it past `prefix`, up to the
/// next space), an allowlist ([`judged`]): kept, written `prefix***`, or
/// none when in doubt. The [`QUERY_CLOSERS`] that end the span belong to
/// the text around it (a code span's backtick, a link's close) and are
/// given back, but for a parenthesis or bracket that closes one opened in
/// what was dropped.
fn kept(prefix: &str, span: &str, form: Form) -> Option<String> {
    let core = span.trim_end_matches(QUERY_CLOSERS);
    let address = match judged(core, form) {
        Judged::Kept(address) => address,
        Judged::Scrubbed => SCRUBBED.to_string(),
        Judged::Doubt => return None,
    };
    let closing = balanced(core, &address, &span[core.len()..]);
    Some(format!("{prefix}{address}{closing}"))
}

/// What `core` (an address past its scheme) comes to. Kept only when it
/// holds no at sign and its authority is unambiguous ([`unambiguous`]):
/// that authority, its path up to its first `?`, `#`, `;` or `%3F`, and a
/// short anchor fragment straight after that path; a query or matrix
/// parameters (and any fragment after them) are dropped. An at sign may
/// end a userinfo anywhere (a password may hold `/`, `?` or `#`), so one
/// anywhere, or an ambiguous authority, is [`Judged::Scrubbed`]. In doubt:
/// a fragment that is no short anchor ([`anchor_shaped`]), a `%25` (a
/// doubly encoded mark) in its path, or a `;` in a bare host's word.
fn judged(core: &str, form: Form) -> Judged {
    let (authority, rest) = core.split_at(core.find(['/', '?', '#']).unwrap_or(core.len()));
    let at_free = core.chars().all(|c| !AT_SIGNS.contains(&c));
    match (at_free, unambiguous(authority)) {
        (true, true) => {}
        _ => return Judged::Scrubbed,
    }
    let (path, after) = rest.split_at(path_end(rest));
    let fragment = core.split_once('#').map(|(_, fragment)| fragment);
    let plain = !path.to_ascii_lowercase().contains("%25")
        && fragment.is_none_or(anchor_shaped)
        && (form == Form::Named || !core.contains(';'));
    match plain {
        true => {}
        false => return Judged::Doubt,
    }
    Judged::Kept(match after.strip_prefix('#') {
        Some(fragment) => format!("{authority}{path}#{fragment}"),
        None => format!("{authority}{path}"),
    })
}

/// Where a path ends in `rest` (what follows an authority): at its first
/// `?`, `#` or `;`, or an encoded `?` (`%3F`, either case).
fn path_end(rest: &str) -> usize {
    let marked = rest.find(['?', '#', ';']).unwrap_or(rest.len());
    let encoded = rest.to_ascii_lowercase().find("%3f").unwrap_or(rest.len());
    marked.min(encoded)
}

/// `closing`, the marks after an address, less each `)` or `]` that
/// closes a `(` or `[` opened in what was dropped of `core` to keep
/// `kept`: the link or text around it stays balanced. One pass.
fn balanced(core: &str, kept: &str, closing: &str) -> String {
    let count = |text: &str, mark: char| text.matches(mark).count();
    let unclosed = |open: char, close: char| {
        let opened = count(core, open).saturating_sub(count(kept, open));
        let closed = count(core, close).saturating_sub(count(kept, close));
        opened.saturating_sub(closed)
    };
    let (mut parens, mut brackets) = (unclosed('(', ')'), unclosed('[', ']'));
    closing
        .chars()
        .filter(|c| match (c, parens, brackets) {
            (')', 1.., _) => {
                parens -= 1;
                false
            }
            (']', _, 1..) => {
                brackets -= 1;
                false
            }
            _ => true,
        })
        .collect()
}

/// Whether `host`, an authority, is unambiguous: an ASCII host name and,
/// perhaps, a port of one or more digits, and nothing else (no userinfo,
/// no IPv6 or non-ASCII host, which are scrubbed rather than read).
fn unambiguous(host: &str) -> bool {
    let (name, port) = host.split_once(':').unwrap_or((host, "0"));
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c))
        && !port.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
}

/// Whether `fragment` is a short anchor: 1 to [`MAX_FRAGMENT_CHARS`]
/// letters, digits, `_`, `.` and `-` (`L10-L20`, `readme.md`), and nothing
/// else.
fn anchor_shaped(fragment: &str) -> bool {
    (1..=MAX_FRAGMENT_CHARS).contains(&fragment.chars().count())
        && fragment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
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
    let trimmed = opened.trim_end_matches(CLOSERS);
    // A close that balances an open inside the address is the address's
    // (`…/Rust_(programming_language)`): counted once, taken in one step.
    let unclosed = trimmed
        .matches('(')
        .count()
        .saturating_sub(trimmed.matches(')').count());
    let closes = opened[trimmed.len()..]
        .chars()
        .take_while(|c| *c == ')')
        .count();
    let core = &opened[..trimmed.len() + unclosed.min(closes)];
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

/// `core` as the address a link holds: an `https://` address holding no
/// `@`, whose authority (up to its first `/`, `?` or `#`) is unambiguous;
/// its path to its first `?`, `#`, `;` or `%3F`, and an anchor fragment
/// straight after it, kept; its parentheses percent-encoded; and every
/// character one the codec's address takes (an allowlist). None
/// otherwise, so the word is left for the codec to judge.
fn address(core: &str) -> Option<String> {
    let rest = core.strip_prefix(HTTPS)?;
    let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    match !rest.contains('@') && unambiguous(authority) {
        true => {}
        false => return None,
    }
    let (path, after) = tail.split_at(path_end(tail));
    let kept = match after
        .strip_prefix('#')
        .filter(|fragment| anchor_shaped(fragment))
    {
        Some(fragment) => format!("{path}#{fragment}"),
        None => path.to_string(),
    };
    let encoded = kept.replace('(', "%28").replace(')', "%29");
    encoded
        .chars()
        .all(path_char)
        .then(|| format!("{HTTPS}{authority}{encoded}"))
}

/// A character the codec's address takes in its host part.
fn host_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-._~:!&'*+,;=%".contains(c)
}

/// A character the codec's address takes past its host, but an `@`
/// (refused in any address here).
fn path_char(c: char) -> bool {
    host_char(c) || "/?#".contains(c)
}

/// Member id `raw` as an owner: scrubbed as an evidence part is
/// ([`scrubbed`], cut to [`MAX_OWNER_CHARS`] first), cut again and trimmed;
/// none when in doubt (an at sign but `worker@1`'s, a credential) or when
/// the board cannot hold what is left.
fn owner_of(raw: &str, cells: &dyn BoardTextCells) -> Option<String> {
    let cut: String = scrubbed(raw, MAX_OWNER_CHARS)?
        .chars()
        .take(MAX_OWNER_CHARS)
        .collect();
    let owner = board_text::line(&cut);
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
