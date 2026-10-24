//! The structured parts the board's artifacts share: their limits and the
//! checks they share (a finding's place is one rule for every artifact
//! that has findings), and a task's description in its sections (#2705
//! T2).
use super::super::value_objects::board_text;
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
/// one table with [`PARTS`], so a rule change is a data edit. Reading
/// stays lenient: what is stored reads by the task's own schema.
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

/// Where a refused size sends the detail (owner-approved design, #2705).
pub const MOVE_DETAILS: &str = "move details into the plan or a handoff";
/// Why a title over [`EntryRules::max_title_chars`] is refused.
const SHORT_TITLE: &str = "write the outcome, with no instructions";
/// Why text that is not its sections is refused.
const NOT_SECTIONED: &str = "must be its sections in order (## Goal, ## Why, ## Done when, \
     ## Links), one blank line between them";

/// How a section's lines are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Sentences, on one or more lines, none a heading.
    Paragraph,
    /// `- [ ] ` or `- [x] ` and an outcome, a line each.
    Checklist,
    /// `- ` and a link, a line each.
    List,
}

/// A section: its heading, the field a refusal names it by, whether a
/// description needs it, how its lines are written, and why it is refused
/// when it is missing or empty and when a line is misshapen.
pub struct PartRule {
    pub heading: &'static str,
    pub field: &'static str,
    pub required: bool,
    pub shape: Shape,
    pub wanted: &'static str,
    pub misshapen: &'static str,
}

/// The sections, in the order a description holds them: goal, why, done
/// when, links.
pub const PARTS: [PartRule; 4] = [
    PartRule {
        heading: "Goal",
        field: "goal",
        required: true,
        shape: Shape::Paragraph,
        wanted: "is required: 1 to 3 sentences saying what the task achieves",
        misshapen: "must be plain sentences, with no heading or blank line",
    },
    PartRule {
        heading: "Why",
        field: "why",
        required: false,
        shape: Shape::Paragraph,
        wanted: "is empty: write one short paragraph, or leave it out",
        misshapen: "must be one short paragraph, with no heading or blank line",
    },
    PartRule {
        heading: "Done when",
        field: "done_when",
        required: true,
        shape: Shape::Checklist,
        wanted: "is required: 1 to 8 outcomes you would check, not steps",
        misshapen: "must be one outcome, written \"- [ ] \" then the outcome",
    },
    PartRule {
        heading: "Links",
        field: "links",
        required: false,
        shape: Shape::List,
        wanted: "is empty: list links, or leave it out",
        misshapen: "must be one link, written \"- \" then the link",
    },
];

const HEADING: &str = "## ";
const BETWEEN: &str = "\n\n";
const OPEN: &str = "- [ ] ";
const DONE: &str = "- [x] ";
const ITEM: &str = "- ";

/// One outcome of the done-when checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub done: bool,
    pub text: String,
}

/// A task description in its sections ([`PARTS`]): `## Goal`, `## Why`
/// (optional), `## Done when` and `## Links` (optional), in that order,
/// one blank line between them. Its text is always its sections exactly
/// ([`Self::markdown`]), so reading it back is byte for byte. Text in any
/// other shape is legacy: it reads as it is, and needs a goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description {
    goal: String,
    why: Option<String>,
    done_when: Vec<Outcome>,
    links: Vec<String>,
}

impl Description {
    /// `text` in its sections, or `None` for legacy text. Lenient: the
    /// entry rules are not checked.
    pub fn read(text: &str) -> Option<Self> {
        sectioned(text).ok()
    }

    /// `text` as it enters the board: `None` when it is empty (nothing
    /// written yet; it needs a goal).
    ///
    /// # Errors
    /// Text that is not normalised board text, not its sections, or
    /// breaks [`ENTRY_RULES`], with the field it breaks.
    pub fn entered(text: &str) -> Result<Option<Self>, SchemaError> {
        board_text::entered_text("description", text)?;
        if text.is_empty() {
            return Ok(None);
        }
        let description = sectioned(text)?;
        description.within_rules(text)?;
        Ok(Some(description))
    }

    /// The description the sections given make, each normalised as board
    /// text: a section not given, or given empty, is left out (and a
    /// required one is then refused on entry).
    pub fn compose(
        goal: Option<&str>,
        why: Option<&str>,
        done_when: Option<&[String]>,
        links: Option<&[String]>,
    ) -> String {
        let paragraph = |text: Option<&str>| text.map(board_text::text);
        let listed = |prefix: &str, items: Option<&[String]>| {
            items.map(|items| {
                let lines: Vec<String> = items
                    .iter()
                    .map(|item| format!("{prefix}{}", board_text::line(item)))
                    .collect();
                lines.join("\n")
            })
        };
        let bodies = [
            paragraph(goal),
            paragraph(why),
            listed(OPEN, done_when),
            listed(ITEM, links),
        ];
        Self::assemble(PARTS.iter().zip(&bodies).filter_map(|(rule, body)| {
            let body = body.as_deref().filter(|body| !body.is_empty())?;
            Some((rule.heading, body))
        }))
    }

    /// The text `sections` make, each a heading and its body, in order.
    pub fn assemble<'a>(sections: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
        let blocks: Vec<String> = sections
            .into_iter()
            .map(|(heading, body)| format!("{HEADING}{heading}\n{body}"))
            .collect();
        blocks.join(BETWEEN)
    }

    /// Each section it holds, in order: its heading and body.
    pub fn sections(&self) -> Vec<(&'static str, String)> {
        let [goal, why, done_when, links] = &PARTS;
        let listed: Vec<String> = self
            .links
            .iter()
            .map(|link| format!("{ITEM}{link}"))
            .collect();
        let mut sections = vec![(goal.heading, self.goal.clone())];
        sections.extend(self.why.clone().map(|text| (why.heading, text)));
        sections.push((done_when.heading, Self::checklist(&self.done_when)));
        // Links are optional: a section only while it holds one.
        if let [_, ..] = listed.as_slice() {
            sections.push((links.heading, listed.join("\n")));
        }
        sections
    }

    /// The done-when section's text `outcomes` make: a line each.
    pub fn checklist(outcomes: &[Outcome]) -> String {
        let lines: Vec<String> = outcomes
            .iter()
            .map(|outcome| match outcome.done {
                true => format!("{DONE}{}", outcome.text),
                false => format!("{OPEN}{}", outcome.text),
            })
            .collect();
        lines.join("\n")
    }

    /// The description as text: its sections.
    pub fn markdown(&self) -> String {
        let sections = self.sections();
        Self::assemble(
            sections
                .iter()
                .map(|(heading, body)| (*heading, body.as_str())),
        )
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
        self.goal.split('\n').collect::<Vec<_>>().join(" ")
    }

    /// The entry rules' sizes: the goal's sentences, the outcomes, and
    /// the whole text's characters.
    fn within_rules(&self, text: &str) -> Result<(), SchemaError> {
        let rules = &ENTRY_RULES;
        let [goal, _, done_when, _] = &PARTS;
        let sized =
            |field: &str, count: usize, what: &str, range: &RangeInclusive<usize>| match range
                .contains(&count)
            {
                true => Ok(()),
                false => Err(SchemaError::new(
                    field,
                    format!(
                        "has {count} {what}; it takes {} to {}: {MOVE_DETAILS}",
                        range.start(),
                        range.end()
                    ),
                )),
            };
        sized(
            goal.field,
            sentences(&self.goal),
            "sentences",
            &rules.goal_sentences,
        )?;
        let outcomes = self.done_when.len();
        sized(done_when.field, outcomes, "outcomes", &rules.done_when)?;
        let chars = text.chars().count();
        let max = rules.max_description_chars;
        match chars <= max {
            true => Ok(()),
            false => Err(SchemaError::new(
                "description",
                format!("is {chars} characters; the most is {max}: {MOVE_DETAILS}"),
            )),
        }
    }
}

/// `text` as its sections, or the first field it breaks: a section that
/// is missing, out of order, empty or misshapen (each body checked as it
/// is met, so the first broken section is named).
fn sectioned(text: &str) -> Result<Description, SchemaError> {
    let wanted = |rule: &PartRule| SchemaError::new(rule.field, rule.wanted);
    let mut bodies: [Option<&str>; 4] = [None; 4];
    let mut next = 0;
    for block in text.split(BETWEEN) {
        let (head, body) = block.split_once('\n').unwrap_or((block, ""));
        let rest = &PARTS[next..];
        let found = head
            .strip_prefix(HEADING)
            .and_then(|name| rest.iter().position(|rule| rule.heading == name));
        let Some(offset) = found else {
            return Err(match next {
                0 => wanted(&PARTS[0]),
                _ => SchemaError::new("description", NOT_SECTIONED),
            });
        };
        if let Some(skipped) = rest[..offset].iter().find(|rule| rule.required) {
            return Err(wanted(skipped));
        }
        let at = next + offset;
        if body.is_empty() {
            return Err(wanted(&PARTS[at]));
        }
        shaped_lines(&PARTS[at], body)?;
        bodies[at] = Some(body);
        next = at + 1;
    }
    if let Some(skipped) = PARTS[next..].iter().find(|rule| rule.required) {
        return Err(wanted(skipped));
    }
    let [goal, why, done_when, links] = bodies;
    let items = |body: Option<&str>, prefixes: &[&str]| -> Vec<(bool, String)> {
        body.map_or(vec![], |body| {
            body.split('\n')
                .map(|line| item(line, prefixes).expect("each line was checked"))
                .collect()
        })
    };
    let description = Description {
        goal: goal.expect("a goal is required").to_string(),
        why: why.map(str::to_string),
        done_when: items(done_when, &[OPEN, DONE])
            .into_iter()
            .map(|(done, text)| Outcome { done, text })
            .collect(),
        links: items(links, &[ITEM])
            .into_iter()
            .map(|(_, text)| text)
            .collect(),
    };
    assert_eq!(
        description.markdown(),
        text,
        "a description is its sections exactly"
    );
    Ok(description)
}

/// Each line of `body` is written as `rule` says.
fn shaped_lines(rule: &PartRule, body: &str) -> Result<(), SchemaError> {
    let misshapen = body.split('\n').position(|line| match rule.shape {
        Shape::Paragraph => line.trim().is_empty() || line.starts_with('#'),
        Shape::Checklist => item(line, &[OPEN, DONE]).is_none(),
        Shape::List => item(line, &[ITEM]).is_none(),
    });
    match (misshapen, rule.shape) {
        (None, _) => Ok(()),
        (Some(_), Shape::Paragraph) => Err(SchemaError::new(rule.field, rule.misshapen)),
        (Some(index), Shape::Checklist | Shape::List) => Err(SchemaError::new(
            format!("{}/{index}", rule.field),
            rule.misshapen,
        )),
    }
}

/// `line` after the first of `prefixes` it starts with (whether that was
/// the done box, [`DONE`]), when something shows after it with no space
/// at its edges (a file holds an outcome in a table cell, which keeps
/// none).
fn item(line: &str, prefixes: &[&str]) -> Option<(bool, String)> {
    prefixes.iter().find_map(|prefix| {
        let text = line.strip_prefix(prefix)?;
        let shows = !text.is_empty() && text.trim() == text;
        shows.then(|| (*prefix == DONE, text.to_string()))
    })
}

/// The sentences of `goal`: one, and one more after each word that ends
/// with `.`, `!` or `?` (past closing quotes and brackets) and comes
/// before a word starting with a capital (so "e.g. the" is one sentence).
fn sentences(goal: &str) -> usize {
    let words: Vec<&str> = goal.split_whitespace().collect();
    let ends = |word: &str| {
        word.trim_end_matches(['"', '\'', ')', ']', '\u{2019}', '\u{201d}'])
            .ends_with(['.', '!', '?'])
    };
    let starts = |word: &str| word.chars().next().is_some_and(char::is_uppercase);
    let boundaries = words
        .windows(2)
        .filter(|pair| ends(pair[0]) && starts(pair[1]))
        .count();
    boundaries + 1
}

/// A title as it enters the board: normalised, and at most
/// [`EntryRules::max_title_chars`] characters. A title read back may be
/// longer (to the schema's [`MAX_TITLE_CHARS`](super::task::MAX_TITLE_CHARS)).
///
/// # Errors
/// A title never normalised, or longer.
pub fn entered_title(field: &str, title: &str) -> Result<(), SchemaError> {
    board_text::entered_line(field, title)?;
    let chars = title.chars().count();
    let max = ENTRY_RULES.max_title_chars;
    match chars <= max {
        true => Ok(()),
        false => Err(SchemaError::new(
            field,
            format!("is {chars} characters; the most is {max}: {SHORT_TITLE}"),
        )),
    }
}

#[cfg(test)]
#[path = "task_parts_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "task_description_tests.rs"]
mod description_tests;
