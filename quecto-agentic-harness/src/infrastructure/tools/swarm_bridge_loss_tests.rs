//! Member loss and confirmed death over the packaged store (#1961).
use super::{context, create};
use crate::infrastructure::tools::swarm_bridge::{
    SwarmContext, process_confirmed_dead, process_start,
};
use serde_json::json;

#[tokio::test]
async fn ready_failure_confirms_the_rolled_back_member_dead_and_keeps_the_run_running() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 2);
    let mut reservation =
        crate::infrastructure::tools::swarm_admission::LaunchReservation::reserve(context.clone())
            .unwrap();
    let member = reservation.member().to_owned();
    let mut command = tokio::process::Command::new("sleep");
    command.arg("30");
    let mut prepared = crate::infrastructure::tools::spawn_container::PreparedChild::new_for_test(
        Some(command),
        None,
        None,
    )
    .await;
    reservation.launched(prepared.display_pid).unwrap();
    prepared.swarm_reservation = Some(reservation);
    assert_eq!(context.summary().unwrap()["usage"], 2);
    prepared.rollback_once().await;
    // The rollback observed the child's exit through the owned handle: that
    // is a confirmed death (#1961), never a lost-harness pause. The slot is
    // free again and the run keeps running.
    let summary = context.summary().unwrap();
    assert_eq!(summary["usage"], 1, "{summary}");
    assert_eq!(summary["status"], "running", "{summary}");
    let rolled_back = summary["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == member)
        .unwrap();
    assert_eq!(rolled_back["status"], "dead", "{summary}");
    let events = context.events(0, 100).unwrap();
    assert!(
        events["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "death_confirmed"),
        "{events}"
    );
}

#[test]
fn abrupt_harness_death_retains_ownership_while_orphan_writer_survives() {
    use std::io::BufRead;
    let directory = tempfile::tempdir().unwrap();
    let parent = context(&directory);
    create(&parent, 2);
    let mut harness = std::process::Command::new("python3")
        .args(["-c", "import subprocess,time; p=subprocess.Popen(['sh','-c','while true; do echo writing >> orphan-write; sleep 0.05; done'], start_new_session=True, stdout=subprocess.DEVNULL); print(p.pid,flush=True); time.sleep(30)"])
        .current_dir(directory.path()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    let mut line = String::new();
    std::io::BufReader::new(harness.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let writer_pid: u32 = line.trim().parse().unwrap();
    let writer_start = process_start(writer_pid).unwrap();
    let worker = SwarmContext {
        member: "worker".into(),
        ..parent.clone()
    };
    parent.call("_admit", json!(["worker", "r"])).unwrap();
    parent
        .call(
            "_activate",
            json!([
                "worker",
                "r",
                harness.id(),
                process_start(harness.id()),
                null
            ]),
        )
        .unwrap();
    let task = worker
        .call("task_create", json!(["t", "write file", ["pass"]]))
        .unwrap();
    let claim = worker.call("claim", json!([task["id"]])).unwrap();
    worker
        .call(
            "reserve",
            json!([task["id"], claim["token"], ["orphan-write"]]),
        )
        .unwrap();
    harness.kill().unwrap();
    harness.wait().unwrap();
    // The launcher's first observation of the vanished pid starts the loss
    // grace (#1961): its owned-handle reaper normally confirms the death
    // first, so nothing is recorded yet and the run keeps running.
    let observed = crate::infrastructure::tools::swarm_lifecycle::reconcile(&parent).unwrap();
    assert_eq!(observed["status"], "running", "{observed}");
    assert_eq!(observed["file_count"], 1);
    // Past the grace (the observation backdated), the loss is recorded.
    backdate_observations(&parent);
    let snapshot = crate::infrastructure::tools::swarm_lifecycle::reconcile(&parent).unwrap();
    let surviving = !process_confirmed_dead(writer_pid, &writer_start);
    crate::infrastructure::tools::swarm::cancel_job_process(writer_pid);
    assert!(surviving);
    assert_eq!(
        snapshot["file_count"], 1,
        "harness death cannot prove its execution scope stopped"
    );
    assert_eq!(snapshot["usage"], 2);
    assert_eq!(
        (snapshot["status"].as_str(), snapshot["outcome"].as_str()),
        (Some("paused"), Some("failed"))
    );
    assert!(parent.call("recover", json!([task["id"]])).is_err());
}

/// Move every `scope_observed` event past the loss grace.
fn backdate_observations(context: &SwarmContext) {
    let status = std::process::Command::new("python3")
        .args([
            "-c",
            "import sqlite3,sys; db=sqlite3.connect(sys.argv[1]); db.execute(\"UPDATE events SET time=time-60 WHERE action='scope_observed'\"); db.commit()",
        ])
        .arg(context.database())
        .status()
        .unwrap();
    assert!(status.success());
}
