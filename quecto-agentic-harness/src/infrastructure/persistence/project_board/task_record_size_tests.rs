//! Every task the domain admits is written: the field limits and the text
//! budget never add up past the file cap.
use super::*;
use crate::domain::project_board::entities::task::{
    MAX_DEPENDENCIES, MAX_DESCRIPTION_CHARS, MAX_PRS, MAX_TEXT_BYTES, MAX_TITLE_CHARS,
};
use crate::domain::project_board::entities::task_parts::{
    MAX_BUDGET_TOKENS, MAX_ITEMS, MAX_LINE_CHARS, MAX_MEMBERS, MAX_PER_ITEM, MAX_REVIEWS,
    MAX_ROLES, MAX_RUNS, MAX_STEPS,
};

fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

/// A 64-byte id, distinct for each `(tag, n)`.
fn wide_id(tag: char, n: usize) -> Slug {
    Slug::parse(&format!("{tag}{n:0>63}")).unwrap()
}

fn person() -> Identity {
    Identity {
        name: "a".into(),
        email: "a@b".into(),
    }
}

/// One-character criteria, distinct within their list.
fn short_lines() -> Vec<String> {
    (0..MAX_PER_ITEM).map(|n| format!("{n:x}")).collect()
}

/// The widest structure a task can have: every list full, every optional
/// key set, every id 64 bytes, every number and word its longest, and
/// each text a single character.
fn widest_structure() -> TaskFields {
    let item = |n: usize| Item {
        id: wide_id('i', n),
        title: "a".into(),
        criteria: short_lines(),
        // Each item depends on up to 16 earlier ones: no cycle.
        depends_on: (n.saturating_sub(MAX_PER_ITEM)..n)
            .map(|earlier| wide_id('i', earlier))
            .collect(),
        state: ItemState::Blocked,
        owner: Some("a".into()),
        evidence: short_lines(),
    };
    let role = |n: usize| Role {
        name: wide_id('r', n),
        model: "a".into(),
        effort: Effort::Medium,
        limits: RoleLimits {
            members: MAX_MEMBERS,
            turns: Some(u32::MAX),
        },
    };
    let run = |n: usize| Run {
        id: wide_id('u', n),
        started: at("09:00:00"),
        ended: (n + 1 < MAX_RUNS).then(|| at("09:00:00")),
        outcome: (n + 1 < MAX_RUNS).then_some(RunOutcome::Succeeded),
    };
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
        plan: (0..MAX_STEPS)
            .map(|_| PlanStep {
                title: "a".into(),
                criteria: short_lines(),
            })
            .collect(),
        items: (0..MAX_ITEMS).map(item).collect(),
        team: Some(Team {
            roles: (0..MAX_ROLES).map(role).collect(),
            budget: Some(Budget {
                tokens: MAX_BUDGET_TOKENS,
            }),
            deadline: Some(at("23:59:59")),
            image: Some("a".into()),
        }),
        claim: Some(Claim {
            holder: person(),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        runs: (0..MAX_RUNS).map(run).collect(),
        reviews: (0..MAX_REVIEWS).map(review).collect(),
        prs: (0..MAX_PRS as u64).map(|n| u64::MAX - n).collect(),
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
    for item in &mut fields.items {
        for criterion in &mut item.criteria {
            left -= widen(criterion, MAX_LINE_CHARS, left);
        }
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
    fields.plan[0].title.push('x');
    let error = Task::new(fields).expect_err("one byte over the budget");
    assert_eq!(error.field, "text", "{error}");
}
