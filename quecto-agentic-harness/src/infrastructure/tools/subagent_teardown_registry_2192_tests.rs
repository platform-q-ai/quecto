//! #2192: a child that ends on its own is announced with why it ended —
//! the tool it was running, its exit status and the panic it recorded.
use std::path::PathBuf;

use super::*;
use crate::domain::crash_record::{CrashRecord, PanicReport};
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::LaunchGeneration;
use crate::infrastructure::persistence::crash_record::Armed;
use crate::infrastructure::tools::subagent_registry::{
    new_exit_signal_channel, new_notification_channel, new_registry,
};

pub(super) const UUID: &str = "abc-123";

pub(super) const REASON: &str = "ended unexpectedly (signal 6) while tool call 'edit' was running \
                      (child-supplied): panicked; the child's recorded panic message: \"byte index 9 is not a \
                      char boundary\" at \"src/edit.rs:3:9\" (child-supplied, unverified)";

async fn exit_note(
    base: &std::path::Path,
    exit: Option<ExitSignal>,
    ended: bool,
    cause: TerminationCause,
) -> Option<SubagentNotification> {
    exit_note_of(
        base,
        exit,
        ended,
        cause,
        crate::domain::child_end::ChildOrigin::Launched,
    )
    .await
}

async fn exit_note_of(
    base: &std::path::Path,
    exit: Option<ExitSignal>,
    ended: bool,
    cause: TerminationCause,
    origin: crate::domain::child_end::ChildOrigin,
) -> Option<SubagentNotification> {
    let registry = new_registry();
    let (exit_tx, _exit_rx) = new_exit_signal_channel();
    exit_tx.send_replace(exit);
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new(UUID),
        "alpha".into(),
        PathBuf::from("/tmp/a.sock"),
        // Launched by this harness as pid 7: the pid its crash record names.
        7,
    );
    entry.origin = origin;
    entry.launch_generation = Some(LaunchGeneration::new(1));
    entry.exit_signal_tx = Some(exit_tx);
    registry.lock().unwrap().insert(UUID.into(), entry);
    let (notify_tx, mut notify_rx) = new_notification_channel();
    let port = RegistryDelegatedAgents::new(
        registry,
        None,
        Some(notify_tx),
        crate::composition::environments::build_member_finalizer,
    )
    .with_ended_child(
        ended.then(|| crate::composition::subagent_lifecycle::build_ended_child_inspection(base)),
    );
    let target = DelegatedAgentIdentity::new(UUID, LaunchGeneration::new(1));
    assert_eq!(port.claim_terminal(&target), TerminalClaim::Claimed);
    port.compensate(&target, cause).await;
    notify_rx.try_recv().ok().map(|note| note.notification)
}

pub(super) fn crash(base: &std::path::Path) {
    let record = CrashRecord::new(
        PanicReport::new(
            "byte index 9 is not a char boundary",
            Some("src/edit.rs:3:9"),
        ),
        7,
        1,
    )
    .running(vec!["edit".into()]);
    Armed::new(base, Some(&format!("cli:{UUID}")), None).record_fatal(&record, "panic", 0);
}

pub(super) fn signal(number: i32) -> Option<ExitSignal> {
    Some(ExitSignal {
        exit_code: None,
        signal: Some(number),
        kind: ExitSignalKind::ProcessExit,
    })
}

#[tokio::test]
async fn a_crashed_child_is_announced_with_its_tool_status_and_panic() {
    let base = tempfile::tempdir().unwrap();
    crash(base.path());
    let note = exit_note(
        base.path(),
        signal(6),
        true,
        TerminationCause::Exit(ExitObservation::ProcessExited),
    )
    .await
    .expect("an exited note");
    let SubagentNotification::Exited { detail, reason, .. } = &note else {
        panic!("{note:?}")
    };
    assert_eq!(reason.as_deref(), Some("process_exit"));
    assert_eq!(
        detail.as_deref(),
        Some(format!("{REASON} (process_exit)").as_str())
    );
    // No transcript in this store: none is offered, and how the end was
    // observed stays in the note (#2192 review).
    assert_eq!(
        note.to_message(),
        format!("Sub-agent 'alpha' {REASON} (process_exit).")
    );
}

#[tokio::test]
async fn an_exit_status_alone_is_still_announced() {
    let base = tempfile::tempdir().unwrap();
    let note = exit_note(
        base.path(),
        signal(9),
        true,
        TerminationCause::Exit(ExitObservation::ProcessExited),
    )
    .await
    .unwrap();
    assert!(
        matches!(&note, SubagentNotification::Exited { detail: Some(d), .. } if d == "ended unexpectedly (signal 9) (process_exit)"),
        "{note:?}"
    );
}

#[tokio::test]
async fn nothing_known_keeps_the_plain_note() {
    let base = tempfile::tempdir().unwrap();
    let note = exit_note(
        base.path(),
        None,
        true,
        TerminationCause::Exit(ExitObservation::ConnectionClosed),
    )
    .await
    .unwrap();
    assert!(
        matches!(&note, SubagentNotification::Exited { detail: None, .. }),
        "{note:?}"
    );
    assert_eq!(
        note.to_message(),
        "Sub-agent 'alpha' ended; no exit status or crash record was observed (connection_closed)."
    );
}

#[tokio::test]
async fn without_an_inspection_the_note_names_only_the_observation() {
    let base = tempfile::tempdir().unwrap();
    crash(base.path());
    let note = exit_note(
        base.path(),
        signal(6),
        false,
        TerminationCause::Exit(ExitObservation::ProcessExited),
    )
    .await
    .unwrap();
    assert!(
        matches!(&note, SubagentNotification::Exited { detail: None, .. }),
        "{note:?}"
    );
}

#[tokio::test]
async fn a_clean_exit_is_announced_in_the_same_words_as_everywhere_else() {
    let base = tempfile::tempdir().unwrap();
    let note = exit_note(
        base.path(),
        Some(ExitSignal {
            exit_code: Some(0),
            signal: None,
            kind: ExitSignalKind::ProcessExit,
        }),
        true,
        TerminationCause::Exit(ExitObservation::ProcessExited),
    )
    .await
    .unwrap();
    assert_eq!(
        note.to_message(),
        "Sub-agent 'alpha' ended normally (exit code 0) (process_exit)."
    );
}

/// #2192 review round 5 (L2): the note offers the transcript only when it
/// can be read — a launched child whose session this store holds — never
/// for a row a child reported, even with a session under its name.
#[tokio::test]
async fn the_transcript_is_offered_only_when_it_can_be_read() {
    use crate::application::sessions::ports::SessionStore;
    use crate::domain::child_end::ChildOrigin;
    let base = tempfile::tempdir().unwrap();
    let store = crate::composition::sessions::build_file_session_store(base.path());
    let mut session = crate::domain::session::Session::new(
        crate::domain::session_identity::SessionIdentity::named_cli(UUID).unwrap(),
    );
    session.messages = vec![crate::domain::message::Message::user("the work")];
    store.save(&session).await.unwrap();
    let clean = || {
        Some(ExitSignal {
            exit_code: Some(0),
            signal: None,
            kind: ExitSignalKind::ProcessExit,
        })
    };
    let cause = || TerminationCause::Exit(ExitObservation::ProcessExited);
    let launched = exit_note_of(base.path(), clean(), true, cause(), ChildOrigin::Launched)
        .await
        .unwrap();
    assert_eq!(
        launched.to_message(),
        "Sub-agent 'alpha' ended normally (exit code 0) (process_exit). Its transcript up to \
         the end stays readable with agent_cmd get_messages."
    );
    for origin in [ChildOrigin::Reported, ChildOrigin::Unverified] {
        let note = exit_note_of(base.path(), clean(), true, cause(), origin)
            .await
            .unwrap();
        assert!(
            !note.to_message().contains("stays readable"),
            "{origin:?}: {}",
            note.to_message()
        );
    }
}
