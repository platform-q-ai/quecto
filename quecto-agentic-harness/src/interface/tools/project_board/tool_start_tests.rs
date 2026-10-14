//! S4c: the tool starts a task with its session's launcher, moved beside
//! `tool_tests.rs` as it grew past its room.
use std::sync::Arc;

use serde_json::{Value, json};

use super::super::site::{BuildError, ProjectBoardEnvironment, ProjectBoardHandles};
use super::BoardSite;
use super::tests::{recorded_tool, site, tool};
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings::QueryListings;
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::QueryBoard;
use crate::application::tools::ports::Tool;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::recording::tests::Records;

/// S4c: the board a test's launcher starts tasks on: task `a` ready with
/// version 1 of its plan accepted (a coordinator and a builder, one item).
fn launching_board(
    environment: &ProjectBoardEnvironment,
) -> Result<ProjectBoardHandles, BuildError> {
    use crate::application::project_board::operations::query_listings::assembled_launching;
    use crate::application::project_board::ports::BoardArtifact;
    use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
    use crate::domain::project_board::entities::task::{Task, TaskFields};
    use crate::domain::project_board::entities::team::{Role, RoleLimits, Team};
    use crate::domain::project_board::value_objects::slug::Slug;
    use crate::domain::project_board::value_objects::timestamp::Timestamp;
    use crate::domain::project_board::value_objects::vocabulary::{Effort, PlanState};
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let slug = |text: &str| Slug::parse(text).unwrap();
    let at = Timestamp::parse("2026-10-10T09:00:00Z").unwrap();
    let ada = crate::domain::project_board::entities::claim::Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    };
    let role = |name: &str, members| Role {
        name: slug(name),
        // The coordinator's model names its provider, as spawn takes it.
        model: match name {
            "coordinator" => "openai-oauth/gpt-5.5".into(),
            _ => "gpt-5.5".into(),
        },
        effort: Effort::High,
        limits: RoleLimits {
            members,
            turns: None,
        },
    };
    let plan = Plan::new(PlanFields {
        task: slug("a"),
        version: 1,
        state: PlanState::Accepted,
        approach: "Build it.".into(),
        items: vec![PlanItem {
            id: slug("schema"),
            title: "Schema".into(),
            criteria: vec!["migrates".into()],
            depends_on: vec![],
        }],
        team: Some(Team {
            roles: vec![role("coordinator", 1), role("builder", 2)],
            budget: None,
            deadline: None,
            image: None,
        }),
        proposed_by: ada.clone(),
        edited_by: None,
        accepted: Some((ada, at.clone())),
        created: at.clone(),
        updated: at.clone(),
    })
    .unwrap();
    let task = Task::new(TaskFields {
        id: slug("a"),
        title: "A".into(),
        kind: TaskKind::Task,
        description: String::new(),
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
        created: at.clone(),
        updated: at,
    })
    .unwrap();
    let head = store.read().unwrap().head;
    let seed = [BoardArtifact::Task(task), BoardArtifact::Plan(plan)];
    store.write(&head, &seed, "seed").unwrap();
    let operations = assembled_launching(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
        environment.launcher.clone(),
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    Ok(ProjectBoardHandles { operations })
}

/// A launcher that launches every coordinator it is asked to.
struct Launching;

impl crate::application::project_board::ports::CoordinatorLaunch for Launching {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        request: &crate::application::project_board::ports::CoordinatorRequest,
    ) -> Result<
        crate::application::project_board::ports::LaunchedCoordinator,
        crate::application::project_board::ports::LaunchRefused,
    > {
        Ok(
            crate::application::project_board::ports::LaunchedCoordinator {
                agent: format!("agent-{}", request.task.as_str()),
                environment_ref: "C3".into(),
            },
        )
    }
}

/// S4c: the tool starts a task with its session's launcher, answers the
/// run and the coordinator, and records the start as a `board_op` and a
/// `board_start` of counts and kinds; with no launcher (a session that may
/// not spawn) it starts nothing.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_starts_a_task_with_its_sessions_launcher() {
    let records = Arc::new(Records::default());
    let launching = BoardSite {
        launcher: Some(Arc::new(Launching)),
        ..site(Some(launching_board))
    };
    let tool = recorded_tool(launching, &records);
    let request = json!({"op": "start", "task": "a", "plan_version": 1});
    let started = tool.execute(&request.to_string()).await.unwrap();
    let answer: Value = serde_json::from_str(&started.content).unwrap();
    assert!(!started.is_error, "{answer}");
    assert_eq!(
        (
            &answer["task"]["status"],
            &answer["run"]["number"],
            &answer["run"]["state"],
            &answer["coordinator"],
            &answer["claim"]
        ),
        (
            &json!("in_progress"),
            &json!(1),
            &json!("running"),
            &json!({"agent": "agent-a", "environment_ref": "C3"}),
            &json!("fresh")
        ),
        "{answer}"
    );
    assert_eq!(
        answer["not_applied"],
        json!([{"role": "builder", "model": "gpt-5.5", "effort": "high"}])
    );
    let ops = records.taken();
    assert_eq!(ops.len(), 1);
    assert_eq!(
        (ops[0].op.as_str(), ops[0].plan_version),
        ("start", Some(1))
    );
    let starts = records.starts();
    assert_eq!(starts.len(), 1);
    let line = serde_json::to_value(crate::domain::audit::AuditEvent::BoardStart(Box::new(
        starts[0].clone(),
    )))
    .unwrap();
    assert_eq!(
        [
            &line["task"],
            &line["run"],
            &line["plan_version"],
            &line["members"],
            &line["items"],
            &line["claim"]
        ],
        [
            &json!("a"),
            &json!(1),
            &json!(1),
            &json!(3),
            &json!(1),
            &json!("fresh")
        ],
        "{line}"
    );
    assert!(line.get("launch_us").is_some(), "{line}");
    let unlaunched = tool_without_launcher()
        .execute(&request.to_string())
        .await
        .unwrap();
    let answer: Value = serde_json::from_str(&unlaunched.content).unwrap();
    assert_eq!(answer["error"]["code"], json!("no_launcher"), "{answer}");
}

fn tool_without_launcher() -> super::BoardTool {
    tool(site(Some(launching_board)))
}
