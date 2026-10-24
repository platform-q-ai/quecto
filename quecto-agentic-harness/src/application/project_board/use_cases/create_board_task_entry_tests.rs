//! A create's entry rules (#2705 T2): a sectioned create keeps every
//! rule, a legacy one master's, and what a board file cannot hold is
//! refused on its own field as it enters.
use super::tests::create;
use crate::application::project_board::dto::{NewBoardTask, ProjectBoardError};
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::fixtures_tests::{SetClock, board, new_task};

/// #2705 T2: a sectioned create enters through the description's entry rules:
/// free text is refused with the field it lacks, and a long title too.
#[test]
fn free_text_or_a_long_title_never_enters() {
    let store = board();
    let made = |new: NewBoardTask| create(&store, SetClock::at("10:00:00")).execute(&new);
    let free = made(NewBoardTask {
        description: "Why and how.".into(),
        ..new_task("a")
    });
    assert!(
        matches!(&free, Err(ProjectBoardError::Invalid(error)) if error.field == "description"),
        "{free:?}"
    );
    let long = made(NewBoardTask {
        title: "t".repeat(71),
        ..new_task("a")
    });
    assert!(
        matches!(&long, Err(ProjectBoardError::Invalid(error)) if error.field == "title"),
        "{long:?}"
    );
    assert_eq!(store.read().unwrap().tasks, vec![]);
    // A legacy entry (the CLI's free text) takes both, as before T2.
    let legacy = made(NewBoardTask {
        title: "t".repeat(120),
        description: "Why and how.".into(),
        entry: crate::domain::project_board::entities::task_parts::Entry::Legacy,
        ..new_task("a")
    });
    assert_eq!(legacy.unwrap().fields().description, "Why and how.");
}

/// Review round 2: a goal or why the board file's prose cannot hold is
/// refused as it enters, on its own field, never at write.
#[test]
fn a_goal_or_why_the_file_cannot_hold_is_refused_on_entry() {
    let store = board();
    let made = |goal: &str, why: &str| {
        create(&store, SetClock::at("10:00:00")).execute(&NewBoardTask {
            description: format!("## Goal\n{goal}\n\n## Why\n{why}\n\n## Done when\n- [ ] Done"),
            ..new_task("a")
        })
    };
    for (goal, why, field) in [
        ("See https://x.io for it.", "Users.", "goal"),
        ("Make it <b>bold</b>.", "Users.", "goal"),
        ("Keep &amp; out.", "Users.", "goal"),
        ("Show ![a](https://x.io/a.png) here.", "Users.", "goal"),
        ("Read [x](http://x.io) first.", "Users.", "goal"),
        ("Ship.", "Hidden <!-- note --> here.", "why"),
        ("Ship.\n    indented", "Users.", "goal"),
    ] {
        let refused = made(goal, why);
        assert!(
            matches!(&refused, Err(ProjectBoardError::Invalid(error)) if error.field == field),
            "{goal} / {why}: {refused:?}"
        );
    }
    assert_eq!(store.read().unwrap().tasks, vec![]);
    let held = made("See [x.io](https://x.io) for it.", "Users `wait`.");
    assert!(held.is_ok(), "{held:?}");
}

/// Review round 1: an outcome or link a board file cannot hold is refused
/// as it enters, on its own field, never at write.
#[test]
fn an_outcome_or_link_the_file_cannot_hold_is_refused_on_entry() {
    let store = board();
    let made = |description: &str| {
        create(&store, SetClock::at("10:00:00")).execute(&NewBoardTask {
            description: description.into(),
            ..new_task("a")
        })
    };
    for (description, field) in [
        (
            "## Goal\nShip.\n\n## Done when\n- [ ] Done\n- [ ] see <b>x</b>",
            "done_when/1",
        ),
        (
            "## Goal\nShip.\n\n## Done when\n- [ ] see https://x.io",
            "done_when/0",
        ),
        (
            "## Goal\nShip.\n\n## Done when\n- [ ] Done\n\n## Links\n- [www.x.io](https://x.io)",
            "links/0",
        ),
    ] {
        let refused = made(description);
        assert!(
            matches!(&refused, Err(ProjectBoardError::Invalid(error)) if error.field == field),
            "{description}: {refused:?}"
        );
    }
    assert_eq!(store.read().unwrap().tasks, vec![]);
    let held = made(
        "## Goal\nShip.\n\n## Done when\n- [ ] `cargo test` passes\n\n## Links\n- \
                     [PR 1](https://github.com/o/r/pull/1)",
    );
    assert!(held.is_ok(), "{held:?}");
}
