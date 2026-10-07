use quecto::application::swarm::ports::WorkerBoardRead;
use quecto::domain::swarm::RunStatus;
use quecto::domain::swarm::worker_wake::WorkerBoard;

/// #2471: the run's coordinator reads its live run, and no other member
/// reads anything.
#[tokio::test]
async fn only_the_run_s_coordinator_reads_its_worker_board() {
    let (_directory, context) = super::swarm_control_fixture::context();
    assert_eq!(
        context.worker_board().await.unwrap(),
        Some(WorkerBoard {
            status: RunStatus::Running,
            ready: 0,
            claimed_by: Vec::new(),
        })
    );
    let mut worker = context.clone();
    worker.member = "worker".into();
    assert_eq!(worker.worker_board().await.unwrap(), None);
}
