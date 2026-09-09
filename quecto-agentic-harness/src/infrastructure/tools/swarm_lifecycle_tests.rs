use super::*;

#[test]
fn every_non_terminal_run_is_supervised_including_paused() {
    assert!(needs_supervision(RunStatus::Setup));
    assert!(needs_supervision(RunStatus::Running));
    assert!(
        needs_supervision(RunStatus::Paused),
        "a member joining a paused run still needs a watcher"
    );
    for terminal in [
        RunStatus::Succeeded,
        RunStatus::Blocked,
        RunStatus::Failed,
        RunStatus::Cancelled,
    ] {
        assert!(!needs_supervision(terminal), "{terminal:?}");
    }
}

/// #1715: a supervisor tick records participation from the run status, so a
/// member's shared handle follows the run it watches.
#[test]
fn a_supervisor_tick_records_participation_from_the_run() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    let participation = super::super::swarm_bridge::Participation::shared();
    let mut snapshot = context.snapshot().unwrap();
    assert!(!participation.participating());
    super::observe(&context, &mut snapshot, &participation);
    assert!(participation.participating(), "{snapshot:?}");
    assert_eq!(snapshot.status, crate::domain::swarm::RunStatus::Running);
}

/// A member endpoint that answers wake hints and records their generations.
fn wake_sink(
    socket: &std::path::Path,
) -> (
    tokio::task::JoinHandle<Vec<u64>>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let listener = tokio::net::UnixListener::bind(socket).unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = stop.clone();
    let task = tokio::spawn(async move {
        let mut generations = Vec::new();
        while !done.load(std::sync::atomic::Ordering::SeqCst) {
            let accept =
                tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept());
            let Ok(Ok((stream, _))) = accept.await else {
                continue;
            };
            let (read, mut write) = tokio::io::split(stream);
            let mut read = tokio::io::BufReader::new(read);
            let bytes =
                quecto_line_io::read_frame(&mut read, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                    .await
                    .unwrap()
                    .unwrap();
            let command: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(command["type"], "swarm_control");
            assert_eq!(command["action"], "wake");
            generations.push(command["generation"].as_u64().unwrap());
            let response = serde_json::json!({"type":"response","id":command["id"],"success":true});
            quecto_line_io::write_frame(
                &mut write,
                response.to_string().as_bytes(),
                quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
            )
            .await
            .unwrap();
        }
        generations
    });
    (task, stop)
}

/// #1721: a resume is not an actionable board change, so the notification
/// policy targets nobody for it; both resume paths (the control port used by
/// `swarm_control` and the `swarm` tool op) wake every live member with the
/// resume's control generation, so a provider-failure suspension re-arms.
#[tokio::test]
async fn a_resume_wakes_every_live_member_even_though_nothing_targets_them() {
    use crate::domain::swarm::{RunControlAction, SwarmRunControl};
    let (directory, context) = crate::swarm_control_fixture::context();
    let socket = directory.path().join("worker.sock");
    let (sink, stop) = wake_sink(&socket);
    context
        .call("_admit", serde_json::json!(["worker", "r"]))
        .unwrap();
    context
        .call(
            "_activate",
            serde_json::json!(["worker", "r", 123, "identity", socket]),
        )
        .unwrap();
    context.pause("member failed").unwrap();
    let receipt = context.apply(RunControlAction::Resume).await.unwrap();
    assert_eq!(receipt.status, crate::domain::swarm::RunStatus::Running);
    assert!(receipt.wake_warnings.is_empty(), "{receipt:?}");
    context.pause("again").unwrap();
    let tool =
        super::super::swarm_control::control(context.clone(), "resume", serde_json::json!({}))
            .await
            .unwrap();
    let tool_generation = tool["generation"].as_u64().unwrap();
    assert!(tool_generation > receipt.generation, "{tool}");
    assert!(tool.get("wake_warnings").is_none(), "{tool}");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let generations = sink.await.unwrap();
    assert_eq!(generations, [receipt.generation, tool_generation]);
    // A member that cannot be reached is reported, not fatal.
    std::fs::remove_file(&socket).unwrap();
    context.pause("once more").unwrap();
    let tool =
        super::super::swarm_control::control(context.clone(), "resume", serde_json::json!({}))
            .await
            .unwrap();
    assert_eq!(tool["status"], "running", "{tool}");
    assert_eq!(
        tool["wake_warnings"].as_array().map(Vec::len),
        Some(1),
        "{tool}"
    );
    context.pause("and again").unwrap();
    let receipt = context.apply(RunControlAction::Resume).await.unwrap();
    assert_eq!(receipt.wake_warnings.len(), 1, "{receipt:?}");
    assert!(receipt.wake_warnings[0].contains("worker"), "{receipt:?}");
}
