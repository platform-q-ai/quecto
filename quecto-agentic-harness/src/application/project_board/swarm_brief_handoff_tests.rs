//! A plan's handoff in its coordinator's brief (#2705 T3): verbatim, as
//! the data's `handoff`, counted in the brief's size; one the brief has
//! no room for whole is cut to fit with [`CUT_MARKER`] (the pointer says
//! where the rest is), so a start never fails on its handoff's length.
//! A plan without a handoff briefs as before.
use super::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{MAX_HANDOFF_CHARS, Plan, PlanFields};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::services::starts::start;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskKind, TaskStatus};
use serde_json::{Value, json};

fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn person(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

/// The brief of accepted plan v1 of a ready task, its handoff `handoff`.
fn brief(handoff: Option<&str>) -> Result<SwarmBrief, BriefRefusal> {
    let task = Task::new(TaskFields {
        id: slug("board-store"),
        title: "Board store".into(),
        kind: TaskKind::Task,
        description: String::new(),
        description_form: Default::default(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: Some(1),
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at("08:00:00"),
        updated: at("09:00:00"),
    })
    .unwrap();
    let plan = Plan::new(PlanFields {
        task: slug("board-store"),
        version: 1,
        state: PlanState::Accepted,
        approach: "Spike, then build.".into(),
        handoff: handoff.map(str::to_owned),
        items: vec![PlanItem {
            id: slug("schema"),
            title: "Build the schema".into(),
            criteria: vec![],
            depends_on: vec![],
        }],
        team: None,
        proposed_by: person("Ada"),
        edited_by: None,
        accepted: Some((person("Ada"), at("08:30:00"))),
        created: at("08:30:00"),
        updated: at("08:30:00"),
    })
    .unwrap();
    let now = at("10:00:00");
    let started = start(&task, &[], &[], Some(&plan), &person("Bob"), &now).unwrap();
    brief_of(&started)
}

/// The one line of data between the rendered brief's fences, parsed.
fn data(rendered: &str) -> Value {
    let lines: Vec<&str> = rendered.lines().collect();
    let begin = lines
        .iter()
        .position(|line| line.starts_with("=== BEGIN PLAN DATA "))
        .expect("a begin fence");
    serde_json::from_str(lines[begin + 1]).expect("the data is JSON")
}

#[test]
fn a_brief_includes_the_handoff_verbatim() {
    let handoff = "Scope: the store only.\n\n- run `cargo test -p store`\n- \"quote\" \\ and é";
    let brief = brief(Some(handoff)).unwrap();
    assert_eq!(brief.handoff.as_deref(), Some(handoff));
    assert!(!brief.handoff_cut);
    let rendered = render(&brief);
    assert_eq!(data(&rendered)["handoff"], json!(handoff), "{rendered}");
    assert!(rendered.contains(HANDOFF_NOTE), "{rendered}");
    assert!(
        rendered.len() <= brief.bytes(),
        "the size counts the handoff"
    );
}

#[test]
fn a_plan_without_a_handoff_still_briefs_as_before() {
    let brief = brief(None).unwrap();
    assert_eq!((brief.handoff.as_deref(), brief.handoff_cut), (None, false));
    let rendered = render(&brief);
    assert_eq!(data(&rendered).get("handoff"), None, "{rendered}");
    assert!(!rendered.contains("handoff"), "{rendered}");
}

/// The longest handoff, in four-byte letters, takes four times the
/// brief's room: it is cut to fit, never refused.
#[test]
fn a_handoff_past_the_briefs_room_is_cut_to_fit_with_a_marker() {
    let handoff = "\u{1d538}".repeat(MAX_HANDOFF_CHARS);
    let brief = brief(Some(&handoff)).unwrap();
    assert!(brief.handoff_cut);
    assert!(brief.bytes() <= MAX_BRIEF_BYTES, "{}", brief.bytes());
    let shown = brief.handoff.as_deref().unwrap();
    let kept = shown.strip_suffix(CUT_MARKER).expect("ended by the marker");
    assert!(
        handoff.starts_with(kept) && kept.len() > 32 * 1024,
        "{}",
        kept.len()
    );
    let rendered = render(&brief);
    assert!(rendered.len() <= brief.bytes(), "{}", rendered.len());
    assert_eq!(data(&rendered)["handoff"], json!(shown));
    // Quotes and line breaks render escaped, two bytes each: counted so.
    let escaped = format!("a{}", "\"\n".repeat(MAX_HANDOFF_CHARS / 2 - 1));
    let brief = self::brief(Some(&escaped)).unwrap();
    assert!(brief.handoff_cut && brief.bytes() <= MAX_BRIEF_BYTES);
    assert!(render(&brief).len() <= brief.bytes());
}
