//! A task's description in its sections (#2705 T2): what enters, what is
//! refused and with which field, and legacy text read as it is.
use super::*;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::services::drafts::{DraftEdit, edit_draft};
use crate::domain::project_board::services::fixtures_tests::{at, task};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

/// The proposal's own example: every section.
const SECTIONED: &str = "## Goal\nLand L12 (#2652): move the UDS presenters, mapping\nand loop \
     controllers into interface/uds roles.\n\n## Why\nPart of the layout epic #2357. #2648 and \
     #2650\nstack on it.\n\n## Done when\n- [ ] Current master merged in, moves only\n- [x] \
     Layout ratchet and placements still hold\n\n## Links\n- PR #2652 · issue #2370 · epic \
     #2357\n- Handoff: planning notes";

const LEGACY: &str = "Refresh existing PR #2652 after #2641. Merge current origin/master as a \
     merge commit; moves only.\n\n- keep the ratchet";

/// A goal, and `outcomes` outcomes.
fn with_outcomes(outcomes: usize) -> String {
    let items: Vec<String> = (1..=outcomes)
        .map(|n| format!("- [ ] Outcome {n}"))
        .collect();
    format!("## Goal\nShip it.\n\n## Done when\n{}", items.join("\n"))
}

fn refused_field(text: &str) -> String {
    Description::entered(text).expect_err(text).field
}

#[test]
fn a_description_without_a_goal_is_refused_with_its_field() {
    for text in [
        "## Why\nBecause.\n\n## Done when\n- [ ] Shipped",
        "## Done when\n- [ ] Shipped",
        LEGACY,
        "## Goal\n\n## Done when\n- [ ] Shipped",
    ] {
        assert_eq!(refused_field(text), "goal", "{text}");
    }
    assert_eq!(refused_field("## Goal\nShip it."), "done_when");
}

#[test]
fn a_description_over_1200_characters_is_refused() {
    let head = "## Goal\nShip it.\n\n## Done when\n- [ ] ";
    let fill = |chars: usize| format!("{head}{}", "x".repeat(chars - head.chars().count()));
    let at_cap = fill(1_200);
    assert_eq!(at_cap.chars().count(), 1_200);
    assert!(Description::entered(&at_cap).unwrap().is_some());
    let over = Description::entered(&fill(1_201)).unwrap_err();
    assert_eq!(over.field, "description");
    assert!(
        over.problem
            .ends_with("move details into the plan or a handoff"),
        "{}",
        over.problem
    );
}

#[test]
fn nine_outcomes_are_refused_and_eight_enter() {
    assert!(Description::entered(&with_outcomes(8)).unwrap().is_some());
    let nine = Description::entered(&with_outcomes(9)).unwrap_err();
    assert_eq!(nine.field, "done_when");
    assert!(
        nine.problem
            .contains("move details into the plan or a handoff")
    );
}

#[test]
fn a_goal_of_four_sentences_is_refused_and_three_enter() {
    let goal = |goal: &str| format!("## Goal\n{goal}\n\n## Done when\n- [ ] Done");
    let three = goal("Move it. See #2652, e.g. the presenters. Then merge.");
    assert!(Description::entered(&three).unwrap().is_some());
    assert_eq!(
        refused_field(&goal("One. Two. Three. Four.")),
        "goal",
        "four sentences"
    );
}

#[test]
fn a_misshapen_line_is_refused_at_its_place() {
    let cases = [
        (
            "## Goal\nShip.\n\n## Done when\n- [ ] One\n* [ ] Two",
            "done_when/1",
        ),
        (
            "## Goal\nShip.\n\n## Done when\n- [ ] One\n\n## Links\nPR #1",
            "links/0",
        ),
        (
            "## Goal\nShip.\n\n## Done when\n- [ ] One\n\n## Notes\nMore",
            "description",
        ),
        (
            "## Goal\nShip.\n\n## Done when\n- [ ] One\n\n## Why\nLate",
            "description",
        ),
        ("## Goal\nShip.\n## Done when\n- [ ] One", "goal"),
    ];
    for (text, field) in cases {
        assert_eq!(refused_field(text), field, "{text}");
    }
}

#[test]
fn an_empty_description_enters_as_none_and_unnormalised_text_is_refused() {
    assert_eq!(Description::entered(""), Ok(None));
    assert_eq!(refused_field(&format!("{SECTIONED}\n")), "description");
}

#[test]
fn sectioned_text_reads_back_byte_for_byte() {
    let description = Description::read(SECTIONED).expect("sectioned");
    assert_eq!(description.markdown(), SECTIONED);
    assert_eq!(
        description.goal(),
        "Land L12 (#2652): move the UDS presenters, mapping\nand loop controllers into \
         interface/uds roles."
    );
    assert_eq!(
        description.why(),
        Some("Part of the layout epic #2357. #2648 and #2650\nstack on it.")
    );
    assert_eq!(
        description.done_when(),
        [
            Outcome {
                done: false,
                text: "Current master merged in, moves only".into()
            },
            Outcome {
                done: true,
                text: "Layout ratchet and placements still hold".into()
            },
        ]
    );
    assert_eq!(
        description.links(),
        [
            "PR #2652 · issue #2370 · epic #2357",
            "Handoff: planning notes"
        ]
    );
    assert_eq!(
        description.subtitle(),
        "Land L12 (#2652): move the UDS presenters, mapping and loop controllers into \
         interface/uds roles."
    );
    let sections = description.sections();
    let pairs = sections
        .iter()
        .map(|(heading, body)| (*heading, body.as_str()));
    assert_eq!(Description::assemble(pairs), SECTIONED);
    assert_eq!(Description::entered(SECTIONED), Ok(Some(description)));
}

#[test]
fn legacy_text_reads_as_legacy() {
    for text in [
        LEGACY,
        "",
        "## Goal\nShip.",
        "## Goal\nShip.\n\n## Done when\nnot a box",
    ] {
        assert_eq!(Description::read(text), None, "{text}");
    }
}

#[test]
fn composed_sections_are_normalised_and_enter() {
    let outcomes = vec!["  Merged \u{a0}".to_string(), "CI green".to_string()];
    let links = vec!["PR #1".to_string()];
    let text = Description::compose(Some("Ship it.\r\n"), None, Some(&outcomes), Some(&links));
    assert_eq!(
        text,
        "## Goal\nShip it.\n\n## Done when\n- [ ] Merged\n- [ ] CI green\n\n## Links\n- PR #1"
    );
    assert!(Description::entered(&text).unwrap().is_some());
    // What is not given, or given empty, is left out.
    assert_eq!(
        Description::compose(Some("Ship."), Some(" "), Some(&[]), None),
        "## Goal\nShip."
    );
}

#[test]
fn a_title_of_71_characters_is_refused_on_entry() {
    assert_eq!(entered_title("title", &"t".repeat(70)), Ok(()));
    let refused = entered_title("title", &"t".repeat(71)).unwrap_err();
    assert_eq!(refused.field, "title");
    assert!(entered_title("title", " padded").is_err());
}

fn draft(description: &str, title: &str) -> TaskFields {
    let mut fields = task(TaskStatus::Draft).into_fields();
    fields.description = description.into();
    fields.title = title.into();
    fields
}

#[test]
fn a_task_enters_only_with_a_sectioned_or_empty_description_and_a_short_title() {
    assert!(Task::entered(draft(SECTIONED, "Land L12")).is_ok());
    assert!(Task::entered(draft("", "Land L12")).is_ok());
    let legacy = Task::entered(draft(LEGACY, "Land L12")).unwrap_err();
    assert_eq!(legacy.field, "goal");
    let long = Task::entered(draft(SECTIONED, &"t".repeat(71))).unwrap_err();
    assert_eq!(long.field, "title");
    // Reading back stays lenient: legacy text and a long title still read.
    assert!(Task::new(draft(LEGACY, &"t".repeat(200))).is_ok());
}

#[test]
fn editing_a_legacy_drafts_other_fields_never_rewrites_its_description() {
    let legacy = Task::new(draft(LEGACY, &"t".repeat(200))).unwrap();
    let retitle = DraftEdit {
        title: Some("Land L12".into()),
        ..DraftEdit::default()
    };
    let edited = edit_draft(&legacy, &retitle, &at("10:00:00")).unwrap();
    assert_eq!(edited.fields().description, LEGACY);
    let describe = |text: &str| DraftEdit {
        description: Some(text.into()),
        ..DraftEdit::default()
    };
    // Sent again as it is held, legacy text enters nothing: a resend of
    // the draft's own title and description changes nothing.
    let resent = DraftEdit {
        title: Some(legacy.fields().title.clone()),
        description: Some(LEGACY.into()),
        ..DraftEdit::default()
    };
    assert_eq!(
        edit_draft(&legacy, &resent, &at("10:00:00")),
        Err(BoardRuleError::NoEdit)
    );
    assert_eq!(
        edit_draft(&legacy, &describe("Other free text."), &at("10:00:00")),
        Err(BoardRuleError::Invalid(SchemaError::new(
            "goal",
            PARTS[0].wanted
        )))
    );
    let sectioned = edit_draft(&legacy, &describe(SECTIONED), &at("10:00:00")).unwrap();
    assert_eq!(sectioned.fields().description, SECTIONED);
    let long = DraftEdit {
        title: Some("t".repeat(71)),
        ..DraftEdit::default()
    };
    assert!(edit_draft(&legacy, &long, &at("10:00:00")).is_err());
}
