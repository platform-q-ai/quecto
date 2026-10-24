//! Each front end's entry policy (#2705 T2): the CLI (the Pocket relay's,
//! and people's) still takes free text and long titles as the board did
//! before, held as legacy text; given sections, every rule applies. The
//! agent tool takes sections only.
use super::*;
use crate::domain::project_board::entities::task_parts::FREE_TEXT;

#[test]
fn a_cli_create_with_free_text_and_a_long_title_is_held_as_legacy() {
    let title = "t".repeat(120);
    let input = json!({"task": "b", "title": title, "description": "Why.\r\nHow.\r\n"});
    let (code, answer, _) = cli(
        Some(memory_board),
        &["create", "--input", &input.to_string()],
    );
    assert_eq!(code, 0, "{answer}");
    let task = &answer["task"];
    // Master's answer, then the sections' fields: legacy text needs a goal.
    let expected = json!({
        "id": "b",
        "title": title,
        "kind": "task",
        "description": "Why.\nHow.",
        "status": "draft",
        "parent": null,
        "depends_on": [],
        "accepted_plan": null,
        "claim": null,
        "prs": [],
        "created": task["created"],
        "updated": task["updated"],
        "goal": null,
        "why": null,
        "done_when": [],
        "links": [],
        "needs_goal": true,
    });
    assert_eq!(*task, expected);
}

#[test]
fn a_cli_create_with_sections_keeps_every_rule() {
    let input = json!({"task": "b", "title": "t".repeat(71), "goal": "Ship it.",
                       "done_when": ["Shipped"]});
    let (code, answer, _) = cli(
        Some(memory_board),
        &["create", "--input", &input.to_string()],
    );
    assert_eq!(code, 1, "{answer}");
    assert_eq!(
        (&answer["error"]["code"], &answer["error"]["field"]),
        (&json!("invalid_task"), &json!("title")),
        "{answer}"
    );
    let input = json!({"task": "b", "title": "Ship it", "goal": "Ship it.",
                       "done_when": ["Shipped"]});
    let (code, answer, _) = cli(
        Some(memory_board),
        &["create", "--input", &input.to_string()],
    );
    assert_eq!(code, 0, "{answer}");
    assert_eq!(answer["task"]["needs_goal"], json!(false), "{answer}");
}

#[test]
fn a_tool_create_with_free_text_is_pointed_to_the_sections() {
    use crate::application::project_board::ports::ProjectBoardStore;
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let operations = assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .unwrap();
    let request = json!({"op": "create", "task": "b", "title": "B", "description": "Why."});
    // `run` answers as the agent tool does.
    let answer = run(&operations, &request.to_string());
    assert_eq!(
        (&answer["error"]["code"], &answer["error"]["field"]),
        (&json!("invalid_task"), &json!("description")),
        "{answer}"
    );
    assert_eq!(answer["error"]["problem"], json!(FREE_TEXT), "{answer}");
    // The same request from the command line is held as legacy text.
    let (code, answer, _) = cli(
        Some(memory_board),
        &["create", "--input", &request.to_string()],
    );
    assert_eq!(code, 0, "{answer}");
    assert_eq!(answer["task"]["needs_goal"], json!(true), "{answer}");
}

/// Review round 2: sections given only empty never clear a description,
/// from either front end: the goal is wanted.
#[test]
fn empty_sections_alone_are_refused_on_goal_from_both_front_ends() {
    for sections in [
        json!({"links": []}),
        json!({"goal": ""}),
        json!({"done_when": []}),
    ] {
        let mut request = json!({"op": "update_draft", "task": "a"});
        let mut input = json!({"task": "a"});
        for (key, value) in sections.as_object().unwrap() {
            request[key] = value.clone();
            input[key] = value.clone();
        }
        let (code, answer, _) = cli(
            Some(memory_board),
            &["update_draft", "--input", &input.to_string()],
        );
        assert_eq!(code, 1, "{answer}");
        assert_eq!(answer["error"]["field"], json!("goal"), "{answer}");
        let request = crate::interface::tools::project_board::request::parse(
            &request,
            crate::domain::project_board::entities::task_parts::EntryPolicy::Sectioned,
        );
        assert_eq!(request.unwrap_err().field, "goal");
    }
}
