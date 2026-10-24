use super::*;
use crate::domain::project_board::entities::task_parts::DescriptionForm;
use crate::domain::project_board::services::fixtures_tests::{at, task};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus::*;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

#[test]
fn a_draft_takes_a_new_title_description_and_references() {
    let draft = task(Draft);
    let edit = DraftEdit {
        title: Some("Board store, again".into()),
        description: Some("## Goal\nWhy.\n\n## Done when\n- [ ] Done".into()),
        parent: Some(Some(slug("epic"))),
        depends_on: Some(vec![slug("schema")]),
        entry: Default::default(),
    };
    let edited = edit_draft(&draft, &edit, &at("10:00:00")).unwrap();
    let mut expected = draft.fields().clone();
    expected.title = "Board store, again".into();
    expected.description = "## Goal\nWhy.\n\n## Done when\n- [ ] Done".into();
    expected.description_form = DescriptionForm::Sectioned;
    expected.parent = Some(slug("epic"));
    expected.depends_on = vec![slug("schema")];
    expected.updated = at("10:00:00");
    assert_eq!(edited.into_fields(), expected);
}

#[test]
fn only_what_the_edit_sets_changes_and_a_parent_can_be_taken_away() {
    let mut fields = task(Draft).into_fields();
    fields.parent = Some(slug("epic"));
    let draft = Task::new(fields).unwrap();
    let edit = DraftEdit {
        parent: Some(None),
        ..DraftEdit::default()
    };
    let edited = edit_draft(&draft, &edit, &at("10:00:00")).unwrap();
    assert_eq!(edited.fields().parent, None);
    assert_eq!(edited.fields().title, draft.fields().title);
    assert_eq!(edited.fields().description, draft.fields().description);
}

#[test]
fn only_a_draft_is_edited_and_only_by_an_edit_that_changes_something() {
    let retitle = DraftEdit {
        title: Some("New".into()),
        ..DraftEdit::default()
    };
    for status in [Ready, Claimed, InProgress, Review, Blocked, Done, Archived] {
        assert_eq!(
            edit_draft(&task(status), &retitle, &at("10:00:00")),
            Err(BoardRuleError::NotDraft(status))
        );
    }
    assert_eq!(
        edit_draft(&task(Draft), &DraftEdit::default(), &at("10:00:00")),
        Err(BoardRuleError::NoEdit)
    );
    let same = DraftEdit {
        title: Some(task(Draft).fields().title.clone()),
        parent: Some(None),
        depends_on: Some(vec![]),
        ..DraftEdit::default()
    };
    assert_eq!(
        edit_draft(&task(Draft), &same, &at("10:00:00")),
        Err(BoardRuleError::NoEdit),
        "setting what it already holds changes nothing"
    );
}

#[test]
fn an_edit_the_schema_refuses_and_a_stamp_from_a_slow_clock() {
    let own = task(Draft).fields().id.clone();
    let self_parent = DraftEdit {
        parent: Some(Some(own)),
        ..DraftEdit::default()
    };
    assert!(matches!(
        edit_draft(&task(Draft), &self_parent, &at("10:00:00")),
        Err(BoardRuleError::Invalid(_))
    ));
    let retitle = DraftEdit {
        title: Some("x".repeat(257)),
        ..DraftEdit::default()
    };
    assert!(matches!(
        edit_draft(&task(Draft), &retitle, &at("10:00:00")),
        Err(BoardRuleError::Invalid(_))
    ));
    let renamed = DraftEdit {
        title: Some("New".into()),
        ..DraftEdit::default()
    };
    let before_last_write = edit_draft(&task(Draft), &renamed, &at("08:30:00")).unwrap();
    assert_eq!(
        before_last_write.fields().updated,
        at("09:00:01"),
        "never backwards"
    );
}

/// An edit's text is board text as it enters (normalised by whoever took
/// it in); one that is not was never normalised, and is refused.
#[test]
fn an_edit_whose_text_was_never_normalised_is_refused() {
    for (field, edit) in [
        (
            "title",
            DraftEdit {
                title: Some(" padded ".into()),
                description: None,
                parent: None,
                depends_on: None,
                entry: Default::default(),
            },
        ),
        (
            "description",
            DraftEdit {
                title: None,
                description: Some("ends\n".into()),
                parent: None,
                depends_on: None,
                entry: Default::default(),
            },
        ),
    ] {
        let refused = edit_draft(&task(Draft), &edit, &at("10:00:00"));
        assert!(
            matches!(&refused, Err(BoardRuleError::Invalid(error)) if error.field == field),
            "{refused:?}"
        );
    }
}
