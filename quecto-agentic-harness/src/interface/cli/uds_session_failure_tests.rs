use super::*;

#[cfg(test)]
#[test]
fn identical_failure_notifications_do_not_reopen_an_idle_turn() {
    let mut session = AgentSession::new("test".into());
    assert!(
        session
            .enqueue_subagent_notification(
                "worker".into(),
                1,
                "quota exhausted".into(),
                crate::interface::cli::uds_session::NoteClass::Failure
            )
            .is_retained()
    );
    assert_eq!(session.drain_pending().len(), 1);
    assert_eq!(
        session.enqueue_subagent_notification(
            "worker".into(),
            2,
            "quota exhausted".into(),
            crate::interface::cli::uds_session::NoteClass::Failure
        ),
        NotificationEnqueueOutcome::Duplicate
    );
    assert!(session.drain_pending().is_empty());
    assert!(
        session
            .enqueue_subagent_notification(
                "worker".into(),
                3,
                "different failure".into(),
                crate::interface::cli::uds_session::NoteClass::Failure
            )
            .is_retained()
    );
    session.drain_pending();
    assert!(
        session
            .enqueue_subagent_notification(
                "worker".into(),
                4,
                "recovered".into(),
                crate::interface::cli::uds_session::NoteClass::Completion
            )
            .is_retained()
    );
    session.drain_pending();
    assert!(
        session
            .enqueue_subagent_notification(
                "worker".into(),
                5,
                "different failure".into(),
                crate::interface::cli::uds_session::NoteClass::Failure
            )
            .is_retained()
    );
}

/// #2467: a coordinator's run state is news each time it is sent, even when
/// it repeats the last one word for word (idle, a decision, idle again).
#[test]
fn a_repeated_swarm_state_note_still_reopens_an_idle_turn() {
    use crate::interface::cli::uds_session::NoteClass;
    let mut session = AgentSession::new("test".into());
    let idle = "Swarm coordinator 'coord' reports its run running with nothing in flight";
    for sequence in 1..=2 {
        assert!(
            session
                .enqueue_subagent_notification(
                    "coord".into(),
                    sequence,
                    idle.into(),
                    NoteClass::State
                )
                .is_retained(),
            "state note {sequence}"
        );
        assert_eq!(session.drain_pending().len(), 1);
    }
}

/// #2467: run-state notes from two coordinators are never folded into the
/// generic "N sub-agents ended a turn" summary.
#[test]
fn swarm_state_notes_are_not_coalesced_with_turn_ends() {
    use crate::interface::cli::uds_session::NoteClass;
    let mut session = AgentSession::new("test".into());
    for (agent, class) in [
        ("a", NoteClass::State),
        ("b", NoteClass::State),
        ("c", NoteClass::Completion),
    ] {
        let text = format!("{agent} reports its run succeeded");
        assert!(
            session
                .enqueue_subagent_notification(agent.into(), 1, text, class)
                .is_retained()
        );
    }
    let drained: Vec<String> = coalesce_pending(session.drain_pending())
        .into_iter()
        .map(|message| format!("{message:?}"))
        .collect();
    assert_eq!(drained.len(), 3, "{drained:?}");
    for agent in ["a", "b"] {
        let text = format!("{agent} reports its run succeeded");
        assert!(drained.iter().any(|m| m.contains(&text)), "{drained:?}");
    }
}

/// #2467: each sub-agent note is queued in its class.
#[test]
fn each_note_is_queued_in_its_class() {
    use crate::infrastructure::tools::subagent_registry::{
        SequencedSubagentNotification as Note, SubagentNotification, SwarmNoteState,
    };
    use crate::interface::cli::uds_session::NoteClass;
    let agent_id = || "coord".to_owned();
    let cases = [
        (
            SubagentNotification::Completed {
                agent_id: agent_id(),
            },
            NoteClass::Completion,
        ),
        (
            SubagentNotification::SwarmState {
                agent_id: agent_id(),
                state: SwarmNoteState::Quiet { minutes: 30 },
            },
            NoteClass::State,
        ),
        (
            SubagentNotification::Errored {
                agent_id: agent_id(),
                error: "boom".into(),
            },
            NoteClass::Failure,
        ),
    ];
    for (note, class) in cases {
        assert_eq!(
            NoteClass::of(&Note::new(1, note.clone())),
            class,
            "{note:?}"
        );
    }
}
