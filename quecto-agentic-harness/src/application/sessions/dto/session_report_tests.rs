use super::*;

#[test]
fn preview_is_bounded_on_a_character_boundary_with_recovery_from_its_end() {
    let message = Message::assistant("界".repeat(10_000), vec![]);
    let preview = ReportPreview::of(&message);
    assert_eq!(preview.message_id.as_str(), message.id().to_string());
    assert!(preview.content.len() <= REPORT_PREVIEW_BYTES);
    assert_eq!(preview.content.len(), 8190, "8192 is inside a 3-byte char");
    assert!(preview.content_truncated);
    assert_eq!(preview.full_length_bytes, 30_000);
    let recovery = preview.recovery().expect("truncated previews recover");
    assert_eq!(recovery.message_id, preview.message_id);
    assert_eq!(recovery.offset, 8190);
}

#[test]
fn a_preview_that_fits_is_the_whole_content_without_recovery() {
    let message = Message::assistant("done", vec![]);
    let preview = ReportPreview::of(&message);
    assert_eq!(preview.content, "done");
    assert!(!preview.content_truncated);
    assert_eq!(preview.full_length_bytes, 4);
    assert_eq!(preview.recovery(), None);
}

#[test]
fn exactly_the_budget_is_not_truncated() {
    let message = Message::assistant("a".repeat(REPORT_PREVIEW_BYTES), vec![]);
    let preview = ReportPreview::of(&message);
    assert!(!preview.content_truncated);
    assert_eq!(preview.content.len(), REPORT_PREVIEW_BYTES);
}

#[test]
fn errors_display_their_wire_texts() {
    assert_eq!(
        ReportError::ExportUnavailable.to_string(),
        "session export directory unavailable"
    );
    assert_eq!(
        ReportError::ExportBusy.to_string(),
        "two raw exports are already running; retry after completion"
    );
    assert_eq!(
        ReportError::Spill(DomainError::Tool("index unavailable".into())).to_string(),
        "tool error: index unavailable"
    );
    assert_eq!(
        ReportError::SpillDisappeared("spill-1".into()).to_string(),
        "spill disappeared during export: spill-1"
    );
    assert_eq!(
        ReportError::EpochChanged.to_string(),
        "session changed during export; retry against the new epoch"
    );
    assert_eq!(
        ReportError::Writer(DomainError::Tool("session export: denied".into())).to_string(),
        "tool error: session export: denied"
    );
}

#[test]
fn manifest_constants_are_the_format_one_statements() {
    assert_eq!(ExportManifest::FORMAT, 1);
    assert!(ExportManifest::SCOPE.starts_with("retained live and full-message ledger"));
    assert!(ExportManifest::SPILL_CONSISTENCY.starts_with("entries read after"));
    let manifest = ExportManifest {
        epoch: 1,
        revision: 2,
        record_count: 3,
        spill_count: 1,
    };
    assert_eq!(manifest.clone(), manifest);
    let record = ExportRecord::Spill(SpillEntry {
        id: "s".into(),
        tool: "bash".into(),
        input_preview: "ls".into(),
        tokens: 1,
        content: "out".into(),
    });
    assert!(format!("{record:?}").contains("Spill"));
}
