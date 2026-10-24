//! The structured parts the board's artifacts share: their limits and the
//! checks they share (a finding's place is one rule for every artifact
//! that has findings), and a task's description in its sections (#2705
//! T2).
use super::super::value_objects::schema_error::{SchemaError, distinct, line};
use regex::Regex;
use std::ops::RangeInclusive;
use std::sync::LazyLock;

pub const MAX_ITEMS: usize = 128;
/// Criteria, evidence and dependencies per task item.
pub const MAX_PER_ITEM: usize = 16;
pub const MAX_LINE_CHARS: usize = 500;
pub const MAX_OWNER_CHARS: usize = 100;
/// The largest integer a JSON reader in any language keeps exactly.
pub const MAX_BUDGET_TOKENS: u64 = (1 << 53) - 1;
pub const MAX_FINDING_CHARS: usize = 4_000;

/// `path:line` or `path:line:col`, numbers from 1 without leading zeros.
/// The path is relative: `/`-separated segments, none empty, `.` or `..`
/// (a segment starts with a name character, or with one `.` followed by
/// one, as in `.github`), and the first does not start with `-` (an option
/// to a tool).
static LOCATION: LazyLock<Regex> = LazyLock::new(|| {
    let rest = r"[A-Za-z0-9._@+-]*";
    let first = format!(r"(?:[A-Za-z0-9_@+]{rest}|\.[A-Za-z0-9_@+-]{rest})");
    let segment = format!(r"(?:[A-Za-z0-9_@+-]{rest}|\.[A-Za-z0-9_@+-]{rest})");
    let pattern = format!(r"^{first}(?:/{segment})*:[1-9][0-9]{{0,8}}(?::[1-9][0-9]{{0,8}})?$");
    Regex::new(&pattern).expect("location")
});

/// Distinct single lines, at most `max`.
pub(super) fn lines(field: &str, entries: &[String], max: usize) -> Result<(), SchemaError> {
    distinct(field, entries, max)?;
    let mut indexed = entries.iter().enumerate();
    indexed.try_for_each(|(index, entry)| line(&format!("{field}/{index}"), entry, MAX_LINE_CHARS))
}

pub(super) fn shaped(
    field: &str,
    text: &str,
    max_bytes: usize,
    shape: &Regex,
) -> Result<(), SchemaError> {
    if text.len() <= max_bytes && shape.is_match(text) {
        Ok(())
    } else {
        Err(SchemaError::new(
            field,
            format!("{text:?} is not an allowed reference of at most {max_bytes} bytes"),
        ))
    }
}

/// A finding's place, `path:line` or `path:line:col`: one rule for the
/// task's findings and a review's.
pub(super) fn location(field: &str, text: &str) -> Result<(), SchemaError> {
    shaped(field, text, MAX_LINE_CHARS, &LOCATION)
}

/// What a task's title and description enter the board by (#2705 T2):
/// one table, so a rule change is a data edit. Reading stays lenient:
/// what is stored reads by the task's own schema, whatever these say.
pub struct EntryRules {
    /// Characters of the whole description, its headings included.
    pub max_description_chars: usize,
    /// Sentences of its goal.
    pub goal_sentences: RangeInclusive<usize>,
    /// Outcomes of its done-when checklist.
    pub done_when: RangeInclusive<usize>,
    /// Characters of a title.
    pub max_title_chars: usize,
}

pub const ENTRY_RULES: EntryRules = EntryRules {
    max_description_chars: 1_200,
    goal_sentences: 1..=3,
    done_when: 1..=8,
    max_title_chars: 70,
};

/// A section of a description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Goal,
    Why,
    DoneWhen,
    Links,
}

/// A section's heading, the field a refusal names it by, and what it
/// must hold when it is missing or empty.
pub struct PartRule {
    pub part: Part,
    pub heading: &'static str,
    pub field: &'static str,
    pub required: bool,
    pub wanted: &'static str,
}

/// The sections, in the order a description holds them.
pub const PARTS: [PartRule; 4] = [
    PartRule {
        part: Part::Goal,
        heading: "Goal",
        field: "goal",
        required: true,
        wanted: "is required: 1 to 3 sentences saying what the task achieves",
    },
    PartRule {
        part: Part::Why,
        heading: "Why",
        field: "why",
        required: false,
        wanted: "is empty: write one short paragraph, or leave it out",
    },
    PartRule {
        part: Part::DoneWhen,
        heading: "Done when",
        field: "done_when",
        required: true,
        wanted: "is required: 1 to 8 outcomes you would check, not steps",
    },
    PartRule {
        part: Part::Links,
        heading: "Links",
        field: "links",
        required: false,
        wanted: "is empty: list links, or leave it out",
    },
];

/// One outcome of the done-when checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub done: bool,
    pub text: String,
}

/// A task description in its sections: `## Goal`, `## Why` (optional),
/// `## Done when` (`- [ ] ` or `- [x] ` outcomes) and `## Links`
/// (optional, `- ` lines), in that order, one blank line between them.
/// Text in any other shape is legacy: it reads as it is, and needs a goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description {
    goal: String,
    why: Option<String>,
    done_when: Vec<Outcome>,
    links: Vec<String>,
}

impl Description {
    /// `text` in its sections, or `None` for legacy text.
    pub fn read(_text: &str) -> Option<Self> {
        None
    }

    /// `text` as it enters the board: `None` when it is empty.
    ///
    /// # Errors
    /// Text that is not normalised, not in its sections, or breaks
    /// [`ENTRY_RULES`].
    pub fn entered(_text: &str) -> Result<Option<Self>, SchemaError> {
        Ok(None)
    }

    /// The description the given sections make.
    pub fn compose(
        _goal: Option<&str>,
        _why: Option<&str>,
        _done_when: Option<&[String]>,
        _links: Option<&[String]>,
    ) -> String {
        String::new()
    }

    /// The description `sections` make, each a heading and its body.
    pub fn assemble<'a>(_sections: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
        String::new()
    }

    /// Each section it holds: its heading and body.
    pub fn sections(&self) -> Vec<(&'static str, String)> {
        vec![]
    }

    /// The description as text.
    pub fn markdown(&self) -> String {
        String::new()
    }

    pub fn goal(&self) -> &str {
        &self.goal
    }

    pub fn why(&self) -> Option<&str> {
        self.why.as_deref()
    }

    pub fn done_when(&self) -> &[Outcome] {
        &self.done_when
    }

    pub fn links(&self) -> &[String] {
        &self.links
    }

    /// Its goal on one line: a card's subtitle.
    pub fn subtitle(&self) -> String {
        String::new()
    }
}

/// A title as it enters the board: normalised, and at most
/// [`EntryRules::max_title_chars`] characters.
///
/// # Errors
/// A title never normalised, or longer.
pub fn entered_title(_field: &str, _title: &str) -> Result<(), SchemaError> {
    Ok(())
}

#[cfg(test)]
#[path = "task_parts_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "task_description_tests.rs"]
mod description_tests;
