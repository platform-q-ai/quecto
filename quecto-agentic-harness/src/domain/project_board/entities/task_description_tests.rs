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
     Layout ratchet and placements still hold\n\n## Links\n- [PR #2652](https://github.com/o/r/pull/2652)\n- \
     [Handoff: planning notes](https://example.com/handoff)";

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
        "## Goal\n\n## Done when\n- [ ] Shipped",
    ] {
        assert_eq!(refused_field(text), "goal", "{text}");
    }
    assert_eq!(refused_field("## Goal\nShip it."), "done_when");
    // Free text, opening with no heading, is pointed to the sections.
    assert_eq!(
        Description::entered(LEGACY),
        Err(SchemaError::new("description", FREE_TEXT))
    );
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
        (
            "## Goal\nShip.\n# Heading\n\n## Done when\n- [ ] One",
            "goal",
        ),
        (
            "## Goal\nShip.\n\n## Why\n   ###\n\n## Done when\n- [ ] One",
            "why",
        ),
        ("## Goal\nShip.\n\n## Done when\n- [ ]  One", "done_when/0"),
    ];
    for (text, field) in cases {
        assert_eq!(refused_field(text), field, "{text}");
    }
}

/// A line opening with a reference, or a `#` a reader never takes for a
/// heading, is prose.
#[test]
fn a_line_opening_with_a_reference_is_prose() {
    for goal in [
        "Land it.\n#2652 goes first.",
        "Ship.\n    # indented code",
        "Ship.\n#######",
    ] {
        let text = format!("## Goal\n{goal}\n\n## Done when\n- [ ] Done");
        assert!(Description::entered(&text).unwrap().is_some(), "{goal}");
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
                exception: None,
                done: false,
                text: "Current master merged in, moves only".into()
            },
            Outcome {
                exception: None,
                done: true,
                text: "Layout ratchet and placements still hold".into()
            },
        ]
    );
    assert_eq!(
        description.links(),
        [
            "[PR #2652](https://github.com/o/r/pull/2652)",
            "[Handoff: planning notes](https://example.com/handoff)"
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
    let links = vec![
        "[PR #1](https://github.com/o/r/pull/1)".to_string(),
        "https://www.example.com/a?b=1".to_string(),
    ];
    let text =
        Description::compose(Some("Ship it.\r\n"), None, Some(&outcomes), Some(&links)).unwrap();
    assert_eq!(
        text,
        "## Goal\nShip it.\n\n## Done when\n- [ ] Merged\n- [ ] CI green\n\n## Links\n- [PR \
         #1](https://github.com/o/r/pull/1)\n- [example.com/a?b=1](https://www.example.com/a?b=1)"
    );
    assert!(Description::entered(&text).unwrap().is_some());
    // What is not given, or given empty, is left out.
    assert_eq!(
        Description::compose(Some("Ship."), Some(" "), Some(&[]), None),
        Ok("## Goal\nShip.".into())
    );
    // Review round 2: sections given only empty never clear a description.
    let empty: &[String] = &[];
    for (goal, done_when, links) in [
        (None, None, Some(empty)),
        (Some(""), None, None),
        (None, Some(empty), None),
        (Some(" "), Some(empty), Some(empty)),
    ] {
        assert_eq!(
            Description::compose(goal, None, done_when, links),
            Err(SchemaError::new("goal", PARTS[0].wanted))
        );
    }
    // Review round 3: a goal or why is kept as written, code and addresses
    // in it included: never rewritten.
    let done = ["Done".to_string()];
    for (goal, why) in CODE {
        let text = Description::compose(Some(goal), Some(why), Some(&done), None).unwrap();
        assert_eq!(
            text,
            format!("## Goal\n{goal}\n\n## Why\n{why}\n\n## Done when\n- [ ] Done")
        );
    }
}

/// Goals and whys holding addresses in code (review round 3): a span, a
/// double-backtick span, a fence, and a span in a why.
const CODE: [(&str, &str); 4] = [
    ("Run `curl https://x.io` first.", "Users."),
    ("Run ``curl https://x.io/`a` `` first.", "Users."),
    ("```sh\ncurl https://x.io\n```", "Users."),
    ("Ship.", "It fails on `curl https://x.io` today."),
];

/// #2705 T2 review round 1: each section is checked on its own field
/// before it is composed, so a line break in one never reaches another's
/// structure, and an item is named by its own index.
#[test]
fn a_section_given_as_a_field_is_refused_on_its_own_field() {
    let refused = |goal: &str, done_when: &[&str]| {
        let items: Vec<String> = done_when.iter().map(|item| (*item).to_string()).collect();
        Description::compose(Some(goal), None, Some(&items), None)
            .unwrap_err()
            .field
    };
    assert_eq!(refused("a\n\n## Done when\n- [ ] injected", &["x"]), "goal");
    assert_eq!(refused("a\n\nb", &["x"]), "goal");
    assert_eq!(refused("Ship.", &["x\n- [ ] y"]), "done_when/0");
    assert_eq!(refused("Ship.", &["x\ny"]), "done_when/0");
    assert_eq!(refused("Ship.", &["ok", "   "]), "done_when/1");
    let why = Description::compose(Some("Ship."), Some("w\n# h"), None, None);
    assert_eq!(why.unwrap_err().field, "why");
    let link = |links: &[&str]| {
        let items: Vec<String> = links.iter().map(|item| (*item).to_string()).collect();
        let text = Description::compose(Some("Ship."), None, Some(&["x".into()]), Some(&items));
        text.and_then(|text| Description::entered(&text).map(|_| ()))
    };
    assert_eq!(link(&["x\ny"]).unwrap_err().field, "links/0");
    // Only `[text](https://…)` or a bare https address enters as a link.
    for refused in ["<https://x.io>", "PR #2", "http://x.io", "[x](http://x.io)"] {
        assert_eq!(
            link(&["https://x.io", refused]).unwrap_err().field,
            "links/1"
        );
    }
    assert_eq!(link(&["https://github.com/o/r/pull/1"]), Ok(()));
}

/// #2705 T2 review round 1: an abbreviation or an initial ends no
/// sentence, so the count falls short, never over.
#[test]
fn abbreviations_and_initials_end_no_sentence() {
    for goal in [
        "Fix Dr. Who. Fix Mr. X. Done.",
        "Use e.g. The parser, i.e. The codec, etc. Then stop.",
        "Cf. St. James vs. Ms. Lee and Mrs. Doe. Ship.",
        "Fix the U.S. Bank export. Ship it. Tell ops.",
        "Run it at 5 p.m. Monday. Then stop.",
        "Pay Acme Inc. Payroll and Bob Jr. Smith. Done.",
    ] {
        let text = format!("## Goal\n{goal}\n\n## Done when\n- [ ] Done");
        assert!(Description::entered(&text).unwrap().is_some(), "{goal}");
    }
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
    let sectioned = |fields: TaskFields| Task::entered(fields, Entry::Sectioned);
    assert!(sectioned(draft(SECTIONED, "Land L12")).is_ok());
    assert!(sectioned(draft("", "Land L12")).is_ok());
    let legacy = sectioned(draft(LEGACY, "Land L12")).unwrap_err();
    assert_eq!(legacy, SchemaError::new("description", FREE_TEXT));
    let long = sectioned(draft(SECTIONED, &"t".repeat(71))).unwrap_err();
    assert_eq!(long.field, "title");
    // Reading back stays lenient: legacy text and a long title still read.
    assert!(Task::new(draft(LEGACY, &"t".repeat(200))).is_ok());
}

/// A legacy entry (a front end still writing free text) takes what the
/// board took before T2: free text and a title to the schema's limit,
/// normalised; it never checks the sections.
#[test]
fn a_legacy_entry_takes_free_text_and_a_long_title() {
    let legacy = |fields: TaskFields| Task::entered(fields, Entry::Legacy);
    assert!(legacy(draft(LEGACY, &"t".repeat(120))).is_ok());
    assert!(legacy(draft(SECTIONED, "Land L12")).is_ok());
    assert_eq!(
        legacy(draft(&format!("{LEGACY}\n"), "Land L12"))
            .unwrap_err()
            .field,
        "description"
    );
    assert_eq!(legacy(draft(LEGACY, " padded")).unwrap_err().field, "title");
    let edit = DraftEdit {
        title: Some("t".repeat(200)),
        description: Some("Other free text.".into()),
        entry: Entry::Legacy,
        ..DraftEdit::default()
    };
    let draft = Task::new(draft(SECTIONED, "Land L12")).unwrap();
    assert!(edit_draft(&draft, &edit, &at("10:00:00")).is_ok());
    // Each front end's policy resolves the entry of its requests.
    assert_eq!(EntryPolicy::Sectioned.entry(false), Entry::Sectioned);
    assert_eq!(EntryPolicy::SectionedOrLegacy.entry(true), Entry::Sectioned);
    assert_eq!(EntryPolicy::SectionedOrLegacy.entry(false), Entry::Legacy);
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
            "description",
            FREE_TEXT
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

/// Review round 1 (HIGH): the form, never the text's shape, says how a
/// description is kept. Legacy text in the sections' shape stays legacy
/// on every write; only a description entered as sections is kept in them,
/// and the form is never the caller's to set.
#[test]
fn only_a_description_entered_as_sections_is_kept_in_them() {
    let shaped = "## Goal\nShip it.\n\n## Done when\n- [ ] Merged";
    let mut forged = draft(shaped, "Ship it");
    forged.description_form = DescriptionForm::Sectioned;
    let legacy_entry = Task::entered(forged.clone(), Entry::Legacy).unwrap();
    assert_eq!(
        legacy_entry.fields().description_form,
        DescriptionForm::Legacy
    );
    let sectioned = Task::entered(forged, Entry::Sectioned).unwrap();
    assert_eq!(
        sectioned.fields().description_form,
        DescriptionForm::Sectioned
    );
    let empty = Task::entered(draft("", "Ship it"), Entry::Sectioned).unwrap();
    assert_eq!(empty.fields().description_form, DescriptionForm::Legacy);
    // A legacy task in the sections' shape keeps its form when its title
    // changes; a description given as sections makes it sectioned.
    let legacy = Task::new(draft(shaped, "Ship it")).unwrap();
    let retitle = DraftEdit {
        title: Some("Ship it now".into()),
        ..DraftEdit::default()
    };
    let retitled = edit_draft(&legacy, &retitle, &at("10:00:00")).unwrap();
    assert_eq!(retitled.fields().description_form, DescriptionForm::Legacy);
    let describe = |entry: Entry| DraftEdit {
        description: Some(SECTIONED.into()),
        entry,
        ..DraftEdit::default()
    };
    let as_sections = edit_draft(&legacy, &describe(Entry::Sectioned), &at("10:00:00"));
    assert_eq!(
        as_sections.unwrap().fields().description_form,
        DescriptionForm::Sectioned
    );
    let as_text = edit_draft(&legacy, &describe(Entry::Legacy), &at("10:00:00"));
    assert_eq!(
        as_text.unwrap().fields().description_form,
        DescriptionForm::Legacy
    );
    // A form that says sections holds text in them.
    let mut broken = draft(LEGACY, "Ship it");
    broken.description_form = DescriptionForm::Sectioned;
    assert_eq!(Task::new(broken).unwrap_err().field, "description");
}
