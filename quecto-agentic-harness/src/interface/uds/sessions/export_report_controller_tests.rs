use super::*;
use crate::application::sessions::active_session::ActiveSessionState;
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;

fn controller(messages: &[Message]) -> ExportSessionReportController {
    let mut state = ActiveSessionState::new(SessionIdentity::from_persisted_key("cli:ctl"));
    state.publish(messages);
    let state = Arc::new(tokio::sync::RwLock::new(state));
    ExportSessionReportController::new(Arc::new(ExportSessionReport::new(state, None)))
}

#[tokio::test]
async fn report_maps_export_raw_onto_the_use_case() {
    let controller = controller(&[Message::assistant("done", vec![])]);
    let report = controller.report(false).await.unwrap();
    assert_eq!(report.report.unwrap().content, "done");
    assert!(report.raw_export.is_none());
    assert!(matches!(
        controller.report(true).await.unwrap_err(),
        ReportError::ExportUnavailable
    ));
    assert!(format!("{controller:?}").starts_with("ExportSessionReportController"));
}

#[tokio::test]
async fn admitted_exports_are_the_use_case_admissions() {
    let controller = controller(&[]);
    let first = controller.admit_raw_export().unwrap();
    let _second = controller.admit_raw_export().unwrap();
    assert!(matches!(
        controller.admit_raw_export().map(|_| ()).unwrap_err(),
        ReportError::ExportBusy
    ));
    assert!(matches!(
        first.await.unwrap_err(),
        ReportError::ExportUnavailable
    ));
}
