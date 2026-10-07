use quecto::application::swarm::ports::WorkerBoardRead;
use quecto::application::tools::ports::Tool;
use quecto::domain::swarm::RunStatus;

/// #2471: the run's coordinator reads its live run (every page of tasks and
/// each claimed task's owner), and no other member reads anything.
#[tokio::test]
async fn only_the_run_s_coordinator_reads_its_worker_board() {
    let (_directory, context) = super::swarm_control_fixture::context();
    let tool =
        quecto::infrastructure::tools::swarm::SwarmTool::new().with_context(Some(context.clone()));
    for task in 1..=102 {
        let created = tool
            .execute(
                &serde_json::json!({"op": "task_create", "request": format!("r{task}"), "title": format!("t{task}"), "acceptance": ["done"]})
                    .to_string(),
            )
            .await
            .unwrap();
        assert!(!created.is_error, "{}", created.content);
    }
    let claimed = tool
        .execute(&serde_json::json!({"op": "claim", "task_id": 102}).to_string())
        .await
        .unwrap();
    assert!(!claimed.is_error, "{}", claimed.content);
    let board = context
        .worker_board()
        .await
        .unwrap()
        .expect("the coordinator reads it");
    assert_eq!(board.status, RunStatus::Running);
    assert_eq!(board.ready, 101);
    assert_eq!(board.claimed_by, vec!["coordinator".to_owned()]);
    assert_eq!(board.coordinator, "coordinator");
    let mut worker = context.clone();
    worker.member = "worker".into();
    assert_eq!(worker.worker_board().await.unwrap(), None);
}
