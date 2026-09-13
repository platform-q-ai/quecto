//! Member loss and confirmed death over the packaged store (#1961).
use super::{context, create};
use crate::domain::swarm::MemberExit;
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
    // The rollback concluded the child through the owned handle (a fallback
    // signal to its whole group): that is a confirmed orderly death (#1961),
    // never a lost-harness pause. The slot is free again and the run keeps
    // running.
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
    let confirmed = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "death_confirmed")
        .unwrap_or_else(|| panic!("{events}"));
    let detail: serde_json::Value =
        serde_json::from_str(confirmed["detail"].as_str().unwrap()).unwrap();
    assert_eq!(detail["exit"], "orderly", "{detail}");
}

/// A launched member's harness ended abruptly (SIGKILL from outside) while
/// a Bash-style child in its own process group keeps writing a reserved
/// path: the reaper's confirmation marks it dead and blocks its work, but
/// keeps the reservation until the coordinator frees it explicitly. A
/// delegated kill (orderly) releases it.
#[test]
fn an_abrupt_exit_keeps_reservations_an_orderly_one_releases_them() {
    use std::io::BufRead;
    let directory = tempfile::tempdir().unwrap();
    let parent = context(&directory);
    create(&parent, 3);
    let mut harness = std::process::Command::new("python3")
        .args(["-c", "import subprocess,time; p=subprocess.Popen(['sh','-c','while true; do echo writing >> orphan-write; sleep 0.05; done'], start_new_session=True, stdout=subprocess.DEVNULL); print(p.pid,flush=True); time.sleep(30)"])
        .current_dir(directory.path()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    let mut line = String::new();
    std::io::BufReader::new(harness.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let writer_pid: u32 = line.trim().parse().unwrap();
    let writer_start = process_start(writer_pid).unwrap();
    let launch = |member: &str, pid: u32| {
        parent
            .call("_admit", json!([member, format!("r-{member}")]))
            .unwrap();
        parent
            .call(
                "_activate",
                json!([member, format!("r-{member}"), pid, process_start(pid), null]),
            )
            .unwrap();
        let context = SwarmContext {
            member: member.into(),
            ..parent.clone()
        };
        let task = context
            .call("task_create", json!([member, "write file", ["pass"]]))
            .unwrap();
        let claim = context.call("claim", json!([task["id"]])).unwrap();
        context
            .call(
                "reserve",
                json!([task["id"], claim["token"], [format!("{member}-write")]]),
            )
            .unwrap();
        task["id"].as_u64().unwrap()
    };
    let abrupt_task = launch("abrupt", harness.id());
    harness.kill().unwrap();
    harness.wait().unwrap();
    // The reaper's observation: killed by a signal this harness never sent.
    let after_abrupt = crate::infrastructure::tools::swarm_lifecycle::member_exited(
        &parent,
        "abrupt",
        MemberExit::Abrupt,
    )
    .unwrap();
    let surviving = !process_confirmed_dead(writer_pid, &writer_start);
    crate::infrastructure::tools::swarm::cancel_job_process(writer_pid);
    assert!(surviving, "the orphaned writer outlived its harness");
    assert_eq!(after_abrupt["status"], "running", "{after_abrupt}");
    assert_eq!(after_abrupt["file_count"], 1, "reservation retained");
    assert_eq!(after_abrupt["tasks"][0]["status"], "blocked");
    assert!(
        after_abrupt["tasks"][0]["blocker"]
            .as_str()
            .unwrap()
            .contains("reservations retained"),
        "{after_abrupt}"
    );
    let refused = parent
        .call("recover", json!([abrupt_task]))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("release_files=True"), "{refused}");
    parent.call("recover", json!([abrupt_task, true])).unwrap();
    assert_eq!(parent.summary().unwrap()["file_count"], 0);
    // A member ended by delegation (orderly) has its reservation released.
    let mut tidy = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let tidy_task = launch("tidy", tidy.id());
    tidy.kill().unwrap();
    tidy.wait().unwrap();
    let after_orderly = crate::infrastructure::tools::swarm_lifecycle::member_exited(
        &parent,
        "tidy",
        MemberExit::Orderly,
    )
    .unwrap();
    assert_eq!(after_orderly["status"], "running", "{after_orderly}");
    assert_eq!(after_orderly["file_count"], 0, "reservation released");
    parent.call("recover", json!([tidy_task])).unwrap();
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
