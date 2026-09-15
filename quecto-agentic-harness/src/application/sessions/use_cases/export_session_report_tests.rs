use super::*;
use crate::application::sessions::active_session::ActiveSessionState;
use crate::application::sessions::ports::SpillIndexList;
use crate::domain::error::DomainError;
use crate::domain::message::ToolCall;
use crate::domain::session::{SpillEntry, SpillIndex};
use std::sync::Mutex;

/// A writer that records what it was asked to write, or fails.
#[derive(Default)]
struct FakeExport {
    writes: Mutex<Vec<(Vec<ExportRecord>, ExportManifest)>>,
    fail: bool,
}

impl SessionExportPort for FakeExport {
    fn write_export(
        &self,
        records: Vec<ExportRecord>,
        manifest: ExportManifest,
    ) -> crate::application::sessions::ports::export::ExportOutcome<'_> {
        Box::pin(async move {
            if self.fail {
                return Err(DomainError::Tool("session export: disk full".into()));
            }
            let bytes = records.len() as u64;
            self.writes.lock().unwrap().push((records, manifest));
            Ok(RawExportReceipt {
                records_path: "/exports/x/records.jsonl".into(),
                manifest_path: "/exports/x/manifest.json".into(),
                sha256: "abc".into(),
                bytes,
            })
        })
    }
}

/// What the retention store does when listed and recalled.
#[derive(Clone, Copy)]
enum SpillBehaviour {
    Entries,
    IndexUnavailable,
    RecallUnavailable,
    Disappears,
    /// Replaces the session (a new epoch) while the index is being read.
    ReplacesSessionWhileListing,
    /// Takes long enough for a second admission to be observed.
    Slow,
}

struct FakeSpill {
    behaviour: SpillBehaviour,
    state: Option<ActiveSessionHandle>,
}

impl ContextSpillStore for FakeSpill {
    fn append(
        &self,
        _: &SessionIdentity,
        _: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
    fn recall(
        &self,
        _: &SessionIdentity,
        id: &SpillId,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let id = id.as_str().to_string();
        Box::pin(async move {
            match self.behaviour {
                SpillBehaviour::RecallUnavailable => {
                    Err(DomainError::Tool("recall unavailable".into()))
                }
                SpillBehaviour::Disappears => Ok(None),
                _ => Ok(Some(SpillEntry {
                    id: id.clone(),
                    tool: "bash".into(),
                    input_preview: "ls".into(),
                    tokens: 3,
                    content: format!("content of {id}"),
                })),
            }
        })
    }
    fn list_entries(&self, _: &SessionIdentity) -> SpillIndexList<'_> {
        Box::pin(async move {
            match self.behaviour {
                SpillBehaviour::IndexUnavailable => {
                    return Err(DomainError::Tool("index unavailable".into()));
                }
                SpillBehaviour::ReplacesSessionWhileListing => {
                    self.state.as_ref().unwrap().write().await.clear();
                }
                SpillBehaviour::Slow => {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                _ => {}
            }
            Ok(Arc::new(vec![
                SpillIndex {
                    id: "spill-1".into(),
                    tool: "bash".into(),
                    input_preview: "ls".into(),
                    tokens: 3,
                },
                SpillIndex {
                    id: "spill-2".into(),
                    tool: "grep".into(),
                    input_preview: "x".into(),
                    tokens: 4,
                },
            ]))
        })
    }
    fn clear(
        &self,
        _: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

fn state_with(messages: &[Message], spill: Option<SpillBehaviour>) -> ActiveSessionHandle {
    let mut state = ActiveSessionState::new(SessionIdentity::from_persisted_key("cli:report"));
    state.publish(messages);
    let handle: ActiveSessionHandle = Arc::new(tokio::sync::RwLock::new(state));
    if let Some(behaviour) = spill {
        let store = FakeSpill {
            behaviour,
            state: Some(handle.clone()),
        };
        handle
            .try_write()
            .unwrap()
            .set_spill_store(Some(Arc::new(store)));
    }
    handle
}

fn use_case(
    state: ActiveSessionHandle,
    export: Option<Arc<FakeExport>>,
) -> Arc<ExportSessionReport> {
    Arc::new(ExportSessionReport::new(
        state,
        export.map(|export| export as Arc<dyn SessionExportPort>),
    ))
}

#[tokio::test]
async fn no_eligible_assistant_message_yields_no_report() {
    for messages in [
        Vec::new(),
        vec![Message::user("only a prompt")],
        vec![
            Message::assistant("   ", vec![]),
            Message::assistant(
                "calling",
                vec![ToolCall {
                    id: "c".into(),
                    name: "bash".into(),
                    arguments: "{}".into(),
                }],
            ),
        ],
    ] {
        let report = use_case(state_with(&messages, None), None)
            .execute(false)
            .await
            .unwrap();
        assert_eq!(report.report, None);
        assert_eq!(report.raw_export, None);
    }
}

#[tokio::test]
async fn the_latest_substantive_assistant_message_is_reported_over_later_tool_steps_and_prompts() {
    let earlier = Message::assistant("first report", vec![]);
    let latest = Message::assistant("the report", vec![]);
    let messages = vec![
        earlier,
        latest.clone(),
        Message::assistant(
            "",
            vec![ToolCall {
                id: "c".into(),
                name: "bash".into(),
                arguments: "{}".into(),
            }],
        ),
        Message::assistant("\n\t", vec![]),
        Message::user("pending instruction"),
    ];
    let report = use_case(state_with(&messages, None), None)
        .execute(false)
        .await
        .unwrap();
    let preview = report.report.unwrap();
    assert_eq!(preview.message_id.as_str(), latest.id().to_string());
    assert_eq!(preview.content, "the report");
    assert!(!preview.content_truncated);
}

#[tokio::test]
async fn the_ledger_full_copy_is_preferred_over_a_collapsed_live_stub() {
    let full = Message::assistant("the complete final report", vec![]);
    let state = state_with(std::slice::from_ref(&full), None);
    let mut stub = full.clone();
    stub.content = "recall(spilled)".to_string();
    state.write().await.publish(&[stub]);
    let report = use_case(state, None).execute(false).await.unwrap();
    let preview = report.report.unwrap();
    assert_eq!(preview.message_id.as_str(), full.id().to_string());
    assert_eq!(preview.content, "the complete final report");
}

#[tokio::test]
async fn a_long_report_is_previewed_on_a_character_boundary_with_recovery_metadata() {
    let message = Message::assistant("界".repeat(10_000), vec![]);
    let report = use_case(state_with(std::slice::from_ref(&message), None), None)
        .execute(false)
        .await
        .unwrap();
    let preview = report.report.unwrap();
    assert_eq!(preview.content.len(), 8190);
    assert!(preview.content_truncated);
    assert_eq!(preview.full_length_bytes, 30_000);
    assert_eq!(preview.recovery().unwrap().offset, 8190);
}

#[tokio::test]
async fn a_raw_export_is_refused_as_unavailable_when_composed_without_an_exporter() {
    let message = Message::assistant("report", vec![]);
    let error = use_case(state_with(&[message], None), None)
        .execute(true)
        .await
        .unwrap_err();
    assert!(matches!(error, ReportError::ExportUnavailable), "{error:?}");
}

#[tokio::test]
async fn a_successful_export_records_every_retained_message_then_every_spill_with_counts() {
    let user = Message::user("hi");
    let full = Message::assistant("the report", vec![]);
    let state = state_with(&[user.clone(), full.clone()], Some(SpillBehaviour::Entries));
    let mut stub = full.clone();
    stub.content = "recall(x)".into();
    state.write().await.publish(&[user.clone(), stub]);
    let (epoch, rev) = {
        let ledger = state.read().await;
        (ledger.conversation().epoch(), ledger.conversation().rev())
    };
    let export = Arc::new(FakeExport::default());
    let report = use_case(state.clone(), Some(export.clone()))
        .execute(true)
        .await
        .unwrap();
    assert_eq!(report.report.as_ref().unwrap().content, "the report");
    let receipt = report.raw_export.unwrap();
    assert_eq!(
        receipt.records_path.to_str().unwrap(),
        "/exports/x/records.jsonl"
    );
    assert_eq!(receipt.bytes, 4);
    {
        let writes = export.writes.lock().unwrap();
        let (records, manifest) = &writes[0];
        assert_eq!(records.len(), 4);
        match &records[1] {
            ExportRecord::Message(message) => assert_eq!(message.content, "the report"),
            other => panic!("full copy expected, got {other:?}"),
        }
        match &records[3] {
            ExportRecord::Spill(spill) => assert_eq!(spill.content, "content of spill-2"),
            other => panic!("spill expected, got {other:?}"),
        }
        assert_eq!(
            *manifest,
            ExportManifest {
                epoch,
                revision: rev,
                record_count: 4,
                spill_count: 2,
            }
        );
    }
    let ledger = state.read().await;
    assert_eq!(ledger.conversation().epoch(), epoch);
    assert_eq!(ledger.conversation().rev(), rev);
}

#[tokio::test]
async fn a_failing_writer_is_reported_without_masking_the_writer_text() {
    let export = Arc::new(FakeExport {
        writes: Mutex::default(),
        fail: true,
    });
    let error = use_case(
        state_with(&[Message::assistant("report", vec![])], None),
        Some(export),
    )
    .execute(true)
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "tool error: session export: disk full");
    assert!(matches!(error, ReportError::Writer(_)));
}

#[tokio::test]
async fn incomplete_spill_exports_fail_explicitly_and_write_nothing() {
    for (behaviour, expected) in [
        (
            SpillBehaviour::IndexUnavailable,
            "tool error: index unavailable",
        ),
        (
            SpillBehaviour::RecallUnavailable,
            "tool error: recall unavailable",
        ),
        (
            SpillBehaviour::Disappears,
            "spill disappeared during export: spill-1",
        ),
    ] {
        let export = Arc::new(FakeExport::default());
        let error = use_case(state_with(&[], Some(behaviour)), Some(export.clone()))
            .execute(true)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), expected);
        assert!(
            export.writes.lock().unwrap().is_empty(),
            "no partial artifact"
        );
    }
}

#[tokio::test]
async fn a_session_replaced_during_the_export_is_refused_and_nothing_is_written() {
    let export = Arc::new(FakeExport::default());
    let state = state_with(
        &[Message::assistant("report", vec![])],
        Some(SpillBehaviour::ReplacesSessionWhileListing),
    );
    let error = use_case(state, Some(export.clone()))
        .execute(true)
        .await
        .unwrap_err();
    assert!(matches!(error, ReportError::EpochChanged), "{error:?}");
    assert!(export.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn two_exports_are_admitted_and_a_third_is_refused_until_one_completes() {
    let export = Arc::new(FakeExport::default());
    let state = state_with(
        &[Message::assistant("r", vec![])],
        Some(SpillBehaviour::Slow),
    );
    let use_case = use_case(state, Some(export.clone()));
    let first = use_case.admit_export().unwrap();
    let second = use_case.admit_export().unwrap();
    assert!(matches!(
        use_case.admit_export().map(|_| ()).unwrap_err(),
        ReportError::ExportBusy
    ));
    assert!(format!("{use_case:?}").contains("free_export_slots: 0"));
    drop(second);
    assert!(
        use_case.admit_export().is_ok(),
        "a dropped admission frees its slot"
    );
    let report = first.await.unwrap();
    assert!(report.raw_export.is_some());
    assert_eq!(
        use_case.admissions.available_permits(),
        MAX_CONCURRENT_EXPORTS
    );
}

#[tokio::test]
async fn an_admitted_export_runs_the_same_transaction_as_a_direct_request() {
    let export = Arc::new(FakeExport::default());
    let use_case = use_case(
        state_with(&[], Some(SpillBehaviour::Disappears)),
        Some(export),
    );
    let error = use_case.admit_export().unwrap().await.unwrap_err();
    assert!(matches!(error, ReportError::SpillDisappeared(ref id) if id == "spill-1"));
    assert_eq!(
        use_case.admissions.available_permits(),
        MAX_CONCURRENT_EXPORTS
    );
}
