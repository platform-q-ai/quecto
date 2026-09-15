use super::super::sessions::{SessionLoopInputs, build_session_handles};
use super::*;
use crate::domain::message::Message;

#[tokio::test]
async fn the_composed_loop_exports_under_the_artifacts_directory_of_its_base() {
    let tmp = tempfile::tempdir().unwrap();
    let handles = build_session_handles(SessionLoopInputs {
        base_dir: tmp.path().to_path_buf(),
        store: None,
        session_key: "cli:export".into(),
        ephemeral: false,
        system_prompt: String::new(),
        spill_store: None,
        durable_prefix: crate::application::durable_prefix::DurablePrefixLatch::shared(),
        workflow_state: None,
        subagent_registry: None,
    });
    let message = Message::assistant("the report", vec![]);
    handles
        .active_session
        .write()
        .await
        .publish(std::slice::from_ref(&message));
    let report = handles.export_report.report(true).await.unwrap();
    let receipt = report.raw_export.unwrap();
    assert!(
        receipt.records_path.starts_with(
            tmp.path()
                .canonicalize()
                .unwrap()
                .join("artifacts/session-exports")
        ),
        "{}",
        receipt.records_path.display()
    );
    assert!(
        std::fs::read_to_string(&receipt.records_path)
            .unwrap()
            .contains("the report")
    );
    assert!(Arc::ptr_eq(
        &handles.read_handles().export_report,
        &handles.export_report
    ));
}

#[tokio::test]
async fn a_loop_composed_without_an_exporter_refuses_raw_exports() {
    let state = Arc::new(tokio::sync::RwLock::new(
        crate::application::sessions::active_session::ActiveSessionState::new(
            crate::domain::session_identity::SessionIdentity::from_persisted_key("cli:none"),
        ),
    ));
    let controller = build_export_report(state, None);
    assert!(controller.report(false).await.unwrap().report.is_none());
    assert_eq!(
        controller.report(true).await.unwrap_err().to_string(),
        "session export directory unavailable"
    );
}
