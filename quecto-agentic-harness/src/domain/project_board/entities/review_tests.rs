use super::*;
use crate::domain::project_board::entities::fixtures_tests::{at, commit, person, slug};

/// One change that must make valid fields invalid.
type Change_ = Box<dyn Fn(&mut ReviewFields)>;

fn finding(text: &str) -> Finding {
    Finding {
        location: Some("src/store.rs:42".into()),
        severity: Severity::High,
        text: text.into(),
    }
}

fn round(at_time: &str, verdict: ReviewVerdict) -> Round {
    Round {
        commit: commit('a'),
        reviewer: person("Bob"),
        at: at(at_time),
        verdict,
        findings: vec![finding("Lease not checked")],
    }
}

fn fields() -> ReviewFields {
    ReviewFields {
        task: slug("board-store"),
        change: 1,
        rounds: vec![
            round("09:10:00", ReviewVerdict::ChangesRequested),
            round("09:20:00", ReviewVerdict::Approved),
        ],
        created: at("09:10:00"),
        updated: at("09:20:00"),
    }
}

#[test]
fn a_reviews_state_is_its_last_rounds_verdict() {
    let review = Review::new(fields()).unwrap();
    assert_eq!(review.state(), ReviewVerdict::Approved);
    let mut asked = fields();
    asked.rounds.reverse();
    asked.rounds[0].at = at("09:10:00");
    asked.rounds[1].at = at("09:20:00");
    assert_eq!(
        Review::new(asked).unwrap().state(),
        ReviewVerdict::ChangesRequested
    );
}

#[test]
fn fields_that_break_a_rule_build_no_review() {
    let cases: Vec<(&str, Change_)> = vec![
        ("updated", Box::new(|r| r.updated = at("09:00:00"))),
        ("updated", Box::new(|r| r.created = at("09:30:00"))),
        ("rounds", Box::new(|r| r.rounds.clear())),
        (
            "rounds/0/reviewer/email",
            Box::new(|r| r.rounds[0].reviewer.email = "bob".into()),
        ),
        ("rounds/1/at", Box::new(|r| r.rounds[1].at = at("09:05:00"))),
        ("rounds/0/at", Box::new(|r| r.rounds[0].at = at("09:00:00"))),
        ("rounds/1/at", Box::new(|r| r.rounds[1].at = at("09:30:00"))),
        ("rounds/0/at", Box::new(|r| r.created = at("09:05:00"))),
        ("updated", Box::new(|r| r.updated = at("09:25:00"))),
        (
            "rounds/0/findings/0/location",
            Box::new(|r| r.rounds[0].findings[0].location = Some("../etc:1".into())),
        ),
        (
            "rounds/0/findings/0/text",
            Box::new(|r| r.rounds[0].findings[0].text = " ".into()),
        ),
        (
            "rounds/0/findings/0/text",
            Box::new(|r| r.rounds[0].findings[0].text = "two\nlines".into()),
        ),
        (
            "rounds/0/findings/0/text",
            Box::new(|r| r.rounds[0].findings[0].text = "x".repeat(MAX_FINDING_CHARS + 1)),
        ),
        (
            "rounds/0/findings",
            Box::new(|r| r.rounds[0].findings = vec![finding("x"); MAX_FINDINGS + 1]),
        ),
        (
            "rounds/0/findings/0/location",
            Box::new(|r| r.rounds[0].findings[0].location = Some("src/store.rs".into())),
        ),
        (
            "rounds",
            Box::new(|r| {
                let last = r.rounds[1].clone();
                r.rounds = vec![last; MAX_ROUNDS + 1];
            }),
        ),
    ];
    for (field, change) in cases {
        let mut broken = fields();
        change(&mut broken);
        assert_eq!(Review::new(broken).unwrap_err().field, field);
    }
    let mut widest = fields();
    widest.rounds[0].findings = vec![finding("x"); MAX_FINDINGS];
    let last = widest.rounds[1].clone();
    widest.rounds.resize(MAX_ROUNDS, last);
    widest.rounds[1].findings.clear();
    assert!(Review::new(widest).is_ok(), "the most rounds and findings");
}

/// A review's text is bounded: its findings and places together, exactly
/// to the budget and not a byte past it, over as many rounds as the
/// findings fill.
#[test]
fn a_reviews_text_is_bounded_by_its_budget() {
    let full = filled();
    assert!(
        full.rounds.len() > 2,
        "the findings fill more than two rounds"
    );
    assert!(
        full.rounds[..full.rounds.len() - 1]
            .iter()
            .all(|round| round.findings.len() == MAX_FINDINGS)
    );
    assert_eq!(full.text_bytes(), MAX_REVIEW_TEXT_BYTES);
    assert!(Review::new(full.clone()).is_ok(), "exactly the budget");
    let mut over = full.clone();
    let last = over.rounds.len() - 1;
    over.rounds[last].findings.push(finding("x"));
    assert_eq!(Review::new(over).unwrap_err().field, "text");
    let mut placed = full;
    placed.rounds[0].findings[0].location = Some("a:1".into());
    assert_eq!(
        Review::new(placed).unwrap_err().field,
        "text",
        "a finding's place alone takes it past the budget"
    );
}

/// A review at its text budget, its findings without places, 1,000 bytes
/// each so they fill more than two rounds of [`MAX_FINDINGS`].
fn filled() -> ReviewFields {
    let mut full = fields();
    full.rounds.truncate(1);
    full.rounds[0].findings.clear();
    full.updated = at("09:10:00");
    let mut left = MAX_REVIEW_TEXT_BYTES;
    while left > 0 {
        let last = full.rounds.len() - 1;
        if full.rounds[last].findings.len() == MAX_FINDINGS {
            let mut next = full.rounds[0].clone();
            next.findings.clear();
            full.rounds.push(next);
        }
        let len = left.min(1_000);
        let mut entry = finding(&"x".repeat(len));
        entry.location = None;
        let last = full.rounds.len() - 1;
        full.rounds[last].findings.push(entry);
        left -= len;
    }
    full
}
