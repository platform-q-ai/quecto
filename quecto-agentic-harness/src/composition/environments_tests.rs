//! #2247 review F2: a session's restore prints nothing about another
//! session's environment — not even a kill in flight, which it cannot
//! verify — so no `warn` names another session's ref.
use std::sync::{Arc, Mutex};

use super::{build_environment_registry, build_environment_registry_store};
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
};

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<String>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap()
            .push_str(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLog {
    type Writer = CapturedLog;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn killing_record_of(created_by: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-C1".into(),
        environment_uuid: "uuid-C1".into(),
        name: None,
        workspace_path: "/state/env-C1/workspace".into(),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec!["exec".into()],
        retained_kill_argv: vec!["kill".into()],
        retained_cleanup_argv: vec!["cleanup".into()],
        retained_inspect_argv: vec!["inspect".into()],
        members: vec![],
        status: EnvironmentStatus::Killing,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: created_by.into(),
        created_at: Some(1),
    }
}

/// The warnings a session `session` logs restoring a store that holds one
/// killing record created by `created_by`.
fn restore_warnings(created_by: &str, session: &str) -> String {
    let base = tempfile::TempDir::new().unwrap();
    build_environment_registry_store(base.path())
        .record(&killing_record_of(created_by))
        .unwrap();
    let logs = CapturedLog::default();
    let sink = logs.0.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs)
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        build_environment_registry(base.path(), session, true);
    });
    sink.lock().unwrap().clone()
}

#[test]
fn another_sessions_kill_in_flight_is_not_warned_about() {
    let warnings = restore_warnings("cli:other", "cli:me");
    assert!(!warnings.contains("C1"), "{warnings}");
}

#[test]
fn a_sessions_own_kill_in_flight_is_warned_about() {
    let warnings = restore_warnings("cli:me", "cli:me");
    assert!(
        warnings.contains("C1") && warnings.contains("could not be verified"),
        "{warnings}"
    );
}
