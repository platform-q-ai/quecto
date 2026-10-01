//! The durable messages' refusal sites (#2276), beside the site table
//! [`super::REFUSALS`] so its file stays within 750 lines: the same
//! `(file:function, text, kind)` rows, one per construction site, which
//! the table's test reads with it ([`super::refusals`]).

/// `(file:function, text, kind)` for each construction site of the
/// durable messages (`board_messages.rs` and the four message use cases)
/// and the wake notifications (`accept_wake.rs`), and the loss and death
/// records and the read models (#2277), and the completion use cases
/// (`amend_run_contract.rs`, `complete_run.rs`, `record_evidence.rs`,
/// `revalidate_task.rs`; moved here by #2394 to keep the table's file
/// within 750 lines).
pub(super) const MESSAGE_REFUSALS: &[(&str, &str, &str)] = &[
    (
        "src/application/swarm/use_cases/amend_run_contract.rs:AmendRunContract::execute",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/application/swarm/use_cases/complete_run.rs:CompleteRun::execute",
        "completion accepted a revision that is not text",
        "Internal",
    ),
    (
        "src/application/swarm/use_cases/record_evidence.rs:RecordEvidence::execute",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/application/swarm/use_cases/record_evidence.rs:RecordEvidence::execute",
        "evidence must match a configured criterion and kind",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/record_evidence.rs:RecordEvidence::execute",
        "evidence must match a configured criterion and kind",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/record_evidence.rs:RecordEvidence::execute",
        "the recorded evidence row is gone",
        "Store",
    ),
    (
        "src/application/swarm/use_cases/revalidate_task.rs:RevalidateTask::execute",
        "the board's task row has no evidence",
        "Store",
    ),
    (
        "src/application/swarm/use_cases/revalidate_task.rs:RevalidateTask::execute",
        "unknown task",
        "NotFound",
    ),
    (
        "src/application/swarm/board_read_models.rs:summary",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/application/swarm/board_read_models.rs:summary",
        "the gate admitted a member that is nobody",
        "Internal",
    ),
    (
        "src/application/swarm/use_cases/list_tasks.rs:ListTasks::execute",
        "task page requires nonnegative offset and limit 1 through 100",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/read_run_events.rs:ReadRunEvents::execute",
        "an event id is not an integer",
        "Store",
    ),
    (
        "src/application/swarm/use_cases/read_run_events.rs:ReadRunEvents::execute",
        "event page requires nonnegative cursor and limit 1 through 100",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/read_run_summary.rs:ReadRunSummary::execute",
        "summary cursor must be a nonnegative integer",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/confirm_member_dead.rs:exit_kind",
        "exit kind must be orderly or abrupt",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/lose_coordinator.rs:LoseCoordinator::execute",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/application/swarm/use_cases/accept_wake.rs:claim",
        "wake generation is ahead of the board",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/accept_wake.rs:wake_generation",
        "wake generation must be a nonnegative integer",
        "Invalid",
    ),
    (
        "src/application/swarm/board_messages.rs:message_id",
        "message id must be a positive integer",
        "Invalid",
    ),
    (
        "src/application/swarm/board_messages.rs:retire",
        "message {id} is already {}",
        "WrongState",
    ),
    (
        "src/application/swarm/board_messages.rs:retire",
        "only a message to the same recipient can be {status}",
        "WrongState",
    ),
    (
        "src/application/swarm/board_messages.rs:retire",
        "only your own message can be {status}",
        "NotOwner",
    ),
    (
        "src/application/swarm/use_cases/acknowledge_message.rs:AcknowledgeMessage::execute",
        "unknown message in own inbox",
        "NotFound",
    ),
    (
        "src/application/swarm/use_cases/send_message.rs:SendMessage::execute",
        "supersedes must be a message id",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/send_message.rs:SendMessage::send",
        "recipient inbox full (100 unconsumed messages)",
        "CapacityFull",
    ),
    (
        "src/application/swarm/use_cases/send_message.rs:SendMessage::send",
        "unknown or out-of-swarm recipient",
        "NotFound",
    ),
];
