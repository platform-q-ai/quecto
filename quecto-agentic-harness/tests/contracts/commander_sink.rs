//! Contract for `CommanderSink` (Agent Commander spike): observing never
//! blocks the caller or fails it, whatever the event, and the dry-run
//! implementation is absent unless its switch is on.
use quecto::application::agent_commander::ports::{CommanderEvent, CommanderSink};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct Recording(Mutex<Vec<String>>);

impl CommanderSink for Recording {
    fn observe(&self, session_key: &str, _model: &str, event: CommanderEvent) {
        self.0
            .lock()
            .unwrap()
            .push(format!("{session_key}:{event:?}"));
    }
}

fn every_event() -> Vec<CommanderEvent> {
    vec![
        CommanderEvent::TurnEnd {
            turn: 1,
            prompt: "p".into(),
            final_text: "done".into(),
            stop_reason: Some("end_turn".into()),
            tool_rounds: 0,
            ended_by: "final_response".into(),
            output_tokens: Some(1),
            max_tokens: 10,
        },
        CommanderEvent::ProviderFailure {
            turn: 1,
            provider: "openai".into(),
            class: "client".into(),
            http_status: Some(400),
            error: "bad".into(),
            outcome: "terminal".into(),
            attempt: 0,
        },
        CommanderEvent::ToolError {
            turn: 1,
            tool: "bash".into(),
            arguments: "{}".into(),
            result: "exit 1".into(),
        },
        CommanderEvent::SubagentNotice {
            child: "c".into(),
            child_uuid: None,
            notice: "ended a turn".into(),
            detail: None,
        },
    ]
}

#[test]
fn a_sink_accepts_every_event_kind_without_blocking() {
    let sink: Arc<dyn CommanderSink> = Arc::new(Recording::default());
    let started = std::time::Instant::now();
    for event in every_event() {
        sink.observe("s", "m", event);
    }
    assert!(started.elapsed() < std::time::Duration::from_millis(100));
}

#[test]
fn the_dry_run_commander_is_off_without_its_switch() {
    if std::env::var_os("QUECTO_AGENT_COMMANDER").is_some() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    assert!(
        quecto::infrastructure::agent_commander::DryRunCommander::from_env(
            dir.path(),
            quecto::infrastructure::agent_commander::AgentRole::Root
        )
        .is_none()
    );
}
