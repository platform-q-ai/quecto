//! Every task the domain admits is written: the field limits and the text
//! budget never add up past the file cap.
use super::*;
use crate::domain::project_board::entities::claim::{MAX_EMAIL_BYTES, MAX_NAME_CHARS};
use crate::domain::project_board::entities::plan::MAX_PLAN_VERSION;
use crate::domain::project_board::entities::task::{
    MAX_DEPENDENCIES, MAX_DESCRIPTION_CHARS, MAX_PRS, MAX_REFERENCES, MAX_SUPERSEDED_BY,
    MAX_TEXT_BYTES, MAX_TITLE_CHARS,
};
use crate::domain::project_board::entities::task_parts::{MAX_FINDING_CHARS, MAX_REVIEWS};
use crate::domain::project_board::value_objects::reference::{
    IssueNumber, MAX_ISSUE_NUMBER, Reference,
};
use crate::domain::project_board::value_objects::slug::MAX_SLUG_BYTES;

fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

/// An id of the most bytes, distinct for each `(tag, n)`.
fn wide_id(tag: char, n: usize) -> Slug {
    let width = MAX_SLUG_BYTES - 1;
    Slug::parse(&format!("{tag}{n:0>width$}")).unwrap()
}

/// A holder at its widest in JSON: four-byte characters to the name's
/// limit and an email of escaped `"`s to its limit. A holder is structure,
/// not text, so it may be this wide at any text budget.
fn widest_holder() -> Identity {
    Identity {
        name: "\u{1d11e}".repeat(MAX_NAME_CHARS),
        email: format!("{}@x", "\"".repeat(MAX_EMAIL_BYTES - 2)),
    }
}

fn person() -> Identity {
    Identity {
        name: "a".into(),
        email: "a@b".into(),
    }
}

/// The widest structure a task can have, from the caps: every list full,
/// every optional key set, every id, number and word its longest, the
/// holder at its widest, and each text a single character. A new field fails to build here until it
/// is set.
fn widest_structure() -> TaskFields {
    let review = |n: usize| Review {
        id: wide_id('v', n),
        round: u8::MAX,
        reviewer: person(),
        at: at("09:00:00"),
        severity: Severity::Critical,
        location: Some("a:1".into()),
        finding: "a".into(),
        verdict: Verdict::WontFix,
        proving_test: Some("a".into()),
    };
    TaskFields {
        id: wide_id('t', 0),
        title: "a".into(),
        kind: TaskKind::Chore,
        description: "a".into(),
        status: TaskStatus::InProgress,
        parent: Some(wide_id('t', 1)),
        depends_on: (0..MAX_DEPENDENCIES).map(|n| wide_id('d', n)).collect(),
        accepted_plan: Some(MAX_PLAN_VERSION),
        claim: Some(Claim {
            holder: widest_holder(),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        reviews: (0..MAX_REVIEWS).map(review).collect(),
        prs: (0..MAX_PRS as u64).map(|n| u64::MAX - n).collect(),
        github_issue: Some(IssueNumber::new(MAX_ISSUE_NUMBER).unwrap()),
        // A resolved task is done or archived, so it holds no claim; the
        // claim is the wider of the two.
        resolution: None,
        superseded_by: (0..MAX_SUPERSEDED_BY).map(|n| wide_id('s', n)).collect(),
        // References are text: each its shortest here, distinct.
        references: (1..=MAX_REFERENCES)
            .map(|n| Reference::parse(&format!("#{n}")).unwrap())
            .collect(),
        created: at("08:00:00"),
        updated: at("09:00:00"),
    }
}

/// Appends `"` (the widest escape in JSON) to `text`, up to `max_chars`,
/// taking at most `left` bytes; returns how many it took.
fn widen(text: &mut String, max_chars: usize, left: usize) -> usize {
    let room = max_chars.saturating_sub(text.chars().count()).min(left);
    text.push_str(&"\"".repeat(room));
    room
}

#[test]
fn the_widest_structure_with_its_whole_text_budget_escaped_fits_the_file_cap() {
    let fields = widest_structure();
    let text = fields.text_bytes();
    let task = Task::new(fields).expect("the widest structure is valid");
    let structure = encode(&task).expect("written").len() - text;
    // In JSON a text byte takes at most two (`"`, `\`, `\n` and `\t`
    // escape; nothing else the schema admits does).
    let widest = structure + 2 * MAX_TEXT_BYTES;
    assert!(
        widest <= MAX_TASK_FILE_BYTES,
        "{structure} + 2 × {MAX_TEXT_BYTES} = {widest} > {MAX_TASK_FILE_BYTES}"
    );
}

/// Spends what is left of the text budget on `"`s.
fn fill_text_budget(fields: &mut TaskFields) {
    let mut left = MAX_TEXT_BYTES - fields.text_bytes();
    left -= widen(&mut fields.title, MAX_TITLE_CHARS, left);
    left -= widen(&mut fields.description, MAX_DESCRIPTION_CHARS, left);
    for review in &mut fields.reviews {
        left -= widen(&mut review.finding, MAX_FINDING_CHARS, left);
    }
    assert_eq!(left, 0, "the budget is spent");
}

#[test]
fn a_task_filled_to_its_text_budget_with_quotes_is_written_and_read_back() {
    let mut fields = widest_structure();
    fill_text_budget(&mut fields);
    assert_eq!(fields.text_bytes(), MAX_TEXT_BYTES);
    let task = Task::new(fields).expect("exactly the text budget");
    let bytes = encode(&task).expect("a valid task is always written");
    assert!(bytes.len() <= MAX_TASK_FILE_BYTES, "{} bytes", bytes.len());
    assert_eq!(decode(&bytes), Ok(task));
}

#[test]
fn a_task_over_its_text_budget_is_refused_by_the_domain() {
    let mut fields = widest_structure();
    fill_text_budget(&mut fields);
    let short = fields
        .reviews
        .iter_mut()
        .find(|review| review.finding.chars().count() < MAX_FINDING_CHARS)
        .expect("a finding with room left");
    short.finding.push('x');
    let error = Task::new(fields).expect_err("one byte over the budget");
    assert_eq!(error.field, "text", "{error}");
}

#[test]
fn a_task_at_its_text_budget_takes_a_holder_at_its_widest() {
    let mut fields = widest_structure();
    fields.claim.as_mut().unwrap().holder = person();
    fill_text_budget(&mut fields);
    fields.claim.as_mut().unwrap().holder = widest_holder();
    let task = Task::new(fields).expect("a holder is not text");
    let bytes = encode(&task).expect("a valid task is always written");
    assert!(bytes.len() <= MAX_TASK_FILE_BYTES, "{} bytes", bytes.len());
}
